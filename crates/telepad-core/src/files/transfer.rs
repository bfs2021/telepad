//! Transfers between a local folder and the host over SFTP: first a plan
//! (folders expanded, files that already exist found), then the copy with
//! the user's decisions for those files.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::sftp::{self, Attrs, Sftp};
use super::{Kind, list_local, local_join};

/// Most items one transfer may plan (keeps a mistaken `/` drag in check).
pub const MAX_ITEMS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
}

/// One file or folder to copy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub src: String,
    pub dst: String,
    /// Path shown to the user (relative to the source folder).
    pub label: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<i64>,
    pub perm: Option<u32>,
    /// The target exists already: (size, mtime).
    pub exists: Option<(u64, Option<i64>)>,
}

/// What to do with a file that exists at the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Overwrite,
    Skip,
    /// Overwrite only when the source is newer.
    NewerOnly,
}

impl Item {
    /// Copy this item, given the user's decision for existing files.
    pub fn should_copy(&self, d: Option<Decision>) -> bool {
        match (self.exists, d) {
            (None, _) | (Some(_), Some(Decision::Overwrite)) => true,
            (Some(_), Some(Decision::Skip) | None) => false,
            (Some((_, dst)), Some(Decision::NewerOnly)) => match (self.mtime, dst) {
                (Some(s), Some(d)) => s > d,
                _ => true,
            },
        }
    }
}

/// Source: (path, is folder).
pub type Source = (String, bool);

fn name_of(path: &str) -> String {
    let p = path.trim_end_matches(['/', '\\']);
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

fn remote_exists(s: &mut Sftp, path: &str) -> Option<(u64, Option<i64>)> {
    s.stat(path)
        .ok()
        .map(|a| (a.size.unwrap_or(0), a.mtime.map(i64::from)))
}

fn local_exists(path: &str) -> Option<(u64, Option<i64>)> {
    std::fs::metadata(path).ok().map(|m| {
        (
            m.len(),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
        )
    })
}

/// Expand `sources` (folders recursively) into items copied into `dest_dir`.
pub fn plan(
    s: &mut Sftp,
    dir: Direction,
    sources: &[Source],
    dest_dir: &str,
    cancel: &AtomicBool,
) -> Result<Vec<Item>, String> {
    let mut out = Vec::new();
    for (src, is_dir) in sources {
        let name = name_of(src);
        let dst = match dir {
            Direction::Upload => sftp::join(dest_dir, &name),
            Direction::Download => local_join(dest_dir, &name),
        };
        walk(s, dir, src, &dst, &name, *is_dir, &mut out, cancel)?;
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    s: &mut Sftp,
    dir: Direction,
    src: &str,
    dst: &str,
    label: &str,
    is_dir: bool,
    out: &mut Vec<Item>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    if out.len() >= MAX_ITEMS {
        return Err(format!(
            "More than {MAX_ITEMS} files and folders; copy a smaller folder"
        ));
    }
    let exists = match dir {
        Direction::Upload => remote_exists(s, dst),
        Direction::Download => local_exists(dst),
    };
    if is_dir {
        out.push(Item {
            src: src.into(),
            dst: dst.into(),
            label: format!("{label}/"),
            is_dir: true,
            exists,
            ..Default::default()
        });
        let children: Vec<(String, bool)> = match dir {
            Direction::Upload => list_local(src)?
                .into_iter()
                .filter(|e| e.kind != Kind::Link || e.link_dir || e.size > 0)
                .map(|e| (e.name.clone(), e.is_dir() && e.kind != Kind::Link))
                .collect(),
            Direction::Download => s
                .list(src)?
                .into_iter()
                // Links to folders are not followed (no loops); file links are copied as files.
                .filter(|e| !(e.kind == Kind::Link && e.link_dir))
                .map(|e| (e.name.clone(), e.kind == Kind::Dir))
                .collect(),
        };
        for (n, d) in children {
            let (cs, cd) = match dir {
                Direction::Upload => (local_join(src, &n), sftp::join(dst, &n)),
                Direction::Download => (sftp::join(src, &n), local_join(dst, &n)),
            };
            walk(s, dir, &cs, &cd, &format!("{label}/{n}"), d, out, cancel)?;
        }
        return Ok(());
    }
    let (size, mtime, perm) = match dir {
        Direction::Upload => {
            let m = std::fs::metadata(src).map_err(|e| format!("{src}: {e}"))?;
            (
                m.len(),
                m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64),
                local_mode(&m),
            )
        }
        Direction::Download => {
            let a = s.stat(src)?;
            (
                a.size.unwrap_or(0),
                a.mtime.map(i64::from),
                a.perm.map(|p| p & 0o7777),
            )
        }
    };
    out.push(Item {
        src: src.into(),
        dst: dst.into(),
        label: label.into(),
        is_dir: false,
        size,
        mtime,
        perm,
        exists,
    });
    Ok(())
}

#[cfg(unix)]
fn local_mode(m: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn local_mode(_: &std::fs::Metadata) -> Option<u32> {
    None
}

/// What the runner reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Item index started.
    Start(usize),
    /// Bytes of the current file.
    Bytes(u64),
    /// Item done: `Ok(true)` copied, `Ok(false)` skipped, `Err` failed.
    Done(usize, Result<bool, String>),
}

/// Copy the planned items in order; a failed item does not stop the rest.
/// Returns (copied, skipped, failed).
pub fn run(
    s: &mut Sftp,
    dir: Direction,
    items: &[Item],
    decisions: &HashMap<usize, Decision>,
    cancel: &AtomicBool,
    events: &mut dyn FnMut(Event),
) -> (usize, usize, usize) {
    let (mut copied, mut skipped, mut failed) = (0, 0, 0);
    for (i, it) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        events(Event::Start(i));
        let r = if it.is_dir {
            match dir {
                Direction::Upload => {
                    if it.exists.is_some() {
                        Ok(false)
                    } else {
                        s.mkdir(&it.dst, None).map(|()| true)
                    }
                }
                Direction::Download => std::fs::create_dir_all(&it.dst)
                    .map(|()| it.exists.is_none())
                    .map_err(|e| e.to_string()),
            }
        } else if !it.should_copy(decisions.get(&i).copied()) {
            Ok(false)
        } else {
            copy_file(s, dir, it, cancel, events).map(|()| true)
        };
        match &r {
            Ok(true) => copied += 1,
            Ok(false) => skipped += 1,
            Err(_) => failed += 1,
        }
        events(Event::Done(i, r));
    }
    (copied, skipped, failed)
}

fn copy_file(
    s: &mut Sftp,
    dir: Direction,
    it: &Item,
    cancel: &AtomicBool,
    events: &mut dyn FnMut(Event),
) -> Result<(), String> {
    let mut progress = |n: u64| events(Event::Bytes(n));
    match dir {
        Direction::Upload => {
            let mut f = std::fs::File::open(&it.src).map_err(|e| format!("{}: {e}", it.src))?;
            // Upload next to the target, then rename: a cancelled or failed
            // upload never leaves half a file in place of a good one.
            let part = format!("{}.telepad-part", it.dst);
            let result = (|| {
                s.upload(&mut f, &part, it.perm, cancel, &mut progress)?;
                if let Some(m) = it.mtime {
                    // Keep the source's time so "newer only" works next time.
                    let _ = s.setstat(
                        &part,
                        Attrs {
                            atime: Some(m as u32),
                            mtime: Some(m as u32),
                            ..Default::default()
                        },
                    );
                }
                if it.exists.is_some() {
                    s.remove(&it.dst)?;
                }
                s.rename(&part, &it.dst)
            })();
            if result.is_err() {
                let _ = s.remove(&part);
            }
            result
        }
        Direction::Download => {
            // Write next to the target, then rename: a failed copy never
            // leaves half a file in place of a good one.
            let part = format!("{}.telepad-part", it.dst);
            let result = (|| {
                let mut f = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", it.dst))?;
                s.download(&it.src, &mut f, cancel, &mut progress)?;
                if let Some(m) = it.mtime {
                    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(m.max(0) as u64);
                    let _ = f.set_modified(t);
                }
                drop(f);
                if Path::new(&it.dst).exists() {
                    std::fs::remove_file(&it.dst).map_err(|e| format!("{}: {e}", it.dst))?;
                }
                std::fs::rename(&part, &it.dst).map_err(|e| format!("{}: {e}", it.dst))
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&part);
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(src_m: Option<i64>, dst: Option<(u64, Option<i64>)>) -> Item {
        Item {
            mtime: src_m,
            exists: dst,
            ..Default::default()
        }
    }

    #[test]
    fn decisions_for_existing_files() {
        assert!(item(None, None).should_copy(None), "new files always copy");
        let older = item(Some(10), Some((1, Some(20))));
        let newer = item(Some(30), Some((1, Some(20))));
        assert!(
            !older.should_copy(None),
            "undecided existing files are skipped"
        );
        assert!(older.should_copy(Some(Decision::Overwrite)));
        assert!(!older.should_copy(Some(Decision::Skip)));
        assert!(!older.should_copy(Some(Decision::NewerOnly)));
        assert!(newer.should_copy(Some(Decision::NewerOnly)));
        assert!(item(None, Some((1, None))).should_copy(Some(Decision::NewerOnly)));
    }

    #[test]
    fn names_of_local_and_remote_paths() {
        assert_eq!(name_of("/srv/www/"), "www");
        assert_eq!(name_of("C:\\Users\\me\\site"), "site");
        assert_eq!(name_of("a b.txt"), "a b.txt");
    }
}
