//! Load and save for the editor (#64). Saving writes `<name>.telepad-part`
//! next to the file, gives it the file's permissions (and owner, when the
//! login may set it), then renames it over the file. Before writing, the
//! file's size / modified time are compared with what was loaded, so a file
//! changed meanwhile is never overwritten without asking.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::Kind;
use super::sftp::{Attrs, Sftp};
use super::text::{MAX_EDIT, too_big};

/// What a file looked like when it was loaded or saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Stamp {
    pub size: u64,
    /// Seconds since 1970.
    pub mtime: Option<i64>,
    /// Sub-second part (this computer only; SFTP v3 has whole seconds).
    pub nanos: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The file is not what was loaded any more (`None`: it was deleted).
    Changed(Option<Stamp>),
    Failed(String),
}

fn part_of(path: &str) -> String {
    format!("{path}.telepad-part")
}

fn local_stamp(m: &std::fs::Metadata) -> Stamp {
    let t = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    Stamp {
        size: m.len(),
        mtime: t.map(|d| d.as_secs() as i64),
        nanos: t.map_or(0, |d| d.subsec_nanos()),
    }
}

fn remote_stamp(a: &Attrs) -> Stamp {
    Stamp {
        size: a.size.unwrap_or(0),
        mtime: a.mtime.map(i64::from),
        nanos: 0,
    }
}

/// Read a file on this computer (links followed).
pub fn read_local(path: &str) -> Result<(Vec<u8>, Stamp), String> {
    let m = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
    if !m.is_file() {
        return Err(format!("{path} is not a file"));
    }
    if m.len() > MAX_EDIT {
        return Err(too_big(m.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    Ok((bytes, local_stamp(&m)))
}

/// Collects a download, failing once it is larger than the editor takes.
struct Capped(Vec<u8>);

impl std::io::Write for Capped {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if (self.0.len() + b.len()) as u64 > MAX_EDIT {
            return Err(std::io::Error::other(too_big(
                (self.0.len() + b.len()) as u64,
            )));
        }
        self.0.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Read a file on the host (links followed).
pub fn read_remote(s: &mut Sftp, path: &str) -> Result<(Vec<u8>, Stamp), String> {
    let a = s.stat(path)?;
    if a.kind() != Kind::File {
        return Err(format!("{path} is not a file"));
    }
    if let Some(n) = a.size
        && n > MAX_EDIT
    {
        return Err(too_big(n));
    }
    let mut out = Capped(Vec::new());
    s.download(path, &mut out, &AtomicBool::new(false), &mut |_| {})?;
    Ok((out.0, remote_stamp(&a)))
}

fn check(expect: Option<Stamp>, now: Option<Stamp>) -> Result<(), SaveError> {
    match expect {
        Some(e) if now != Some(e) => Err(SaveError::Changed(now)),
        _ => Ok(()),
    }
}

/// Save on this computer. `expect`: the stamp from loading (`None` to overwrite
/// whatever is there).
pub fn save_local(path: &str, bytes: &[u8], expect: Option<Stamp>) -> Result<Stamp, SaveError> {
    let fail = |e: std::io::Error| SaveError::Failed(format!("{path}: {e}"));
    // Edit what a link points to, not the link.
    let target = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(path)
            .map_err(fail)?
            .display()
            .to_string(),
        _ => path.to_string(),
    };
    let meta = std::fs::metadata(&target).ok();
    check(expect, meta.as_ref().map(local_stamp))?;
    let part = part_of(&target);
    let result = (|| {
        std::fs::write(&part, bytes)?;
        if let Some(m) = &meta {
            std::fs::set_permissions(&part, m.permissions())?;
        }
        std::fs::rename(&part, &target)?;
        std::fs::metadata(&target)
    })();
    match result {
        Ok(m) => Ok(local_stamp(&m)),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(fail(e))
        }
    }
}

/// Save on the host over SFTP; see [`save_local`].
pub fn save_remote(
    s: &mut Sftp,
    path: &str,
    bytes: &[u8],
    expect: Option<Stamp>,
) -> Result<Stamp, SaveError> {
    let fail = SaveError::Failed;
    let target = match s.lstat(path) {
        Ok(a) if a.kind() == Kind::Link => s.realpath(path).map_err(fail)?,
        _ => path.to_string(),
    };
    let now = s.stat(&target).ok();
    check(expect, now.as_ref().map(remote_stamp))?;
    let perm = now.and_then(|a| a.perm).map_or(0o644, |p| p & 0o7777);
    let part = part_of(&target);
    let result = (|| {
        let mut src = bytes;
        s.upload(
            &mut src,
            &part,
            Some(perm),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .map_err(|e| {
            let dir = super::sftp::parent(&target);
            format!("Cannot write a temporary file in {dir}: {e}")
        })?;
        // The umask may have taken bits away; set them exactly.
        s.setstat(
            &part,
            Attrs {
                perm: Some(perm),
                ..Default::default()
            },
        )?;
        // Keep the owner when the login may set it (root); otherwise the
        // file stays the login user's, as with any upload.
        if let Some(a) = now
            && let (Some(uid), Some(gid)) = (a.uid, a.gid)
        {
            let _ = s.setstat(
                &part,
                Attrs {
                    uid: Some(uid),
                    gid: Some(gid),
                    ..Default::default()
                },
            );
        }
        if now.is_some() {
            s.remove(&target)?;
        }
        s.rename(&part, &target)?;
        s.stat(&target)
    })();
    match result {
        Ok(a) => Ok(remote_stamp(&a)),
        Err(e) => {
            let _ = s.remove(&part);
            Err(SaveError::Failed(e))
        }
    }
}

/// The stamp of a local file now (`None` when it is gone).
pub fn local_now(path: &str) -> Option<Stamp> {
    std::fs::metadata(Path::new(path))
        .ok()
        .map(|m| local_stamp(&m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::text;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("telepad-edit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn local_save_keeps_crlf_and_detects_changes() {
        let d = dir("save");
        let p = d.join("app.env").display().to_string();
        std::fs::write(&p, b"A=1\r\nB=2\r\n").unwrap();
        let (bytes, stamp) = read_local(&p).unwrap();
        let doc = text::decode(&bytes).unwrap();
        let edited = doc.text.replace("B=2", "B=3");
        let out = text::encode(&edited, doc.crlf, doc.bom);
        let stamp2 = save_local(&p, &out, Some(stamp)).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"A=1\r\nB=3\r\n");
        assert!(!Path::new(&format!("{p}.telepad-part")).exists());
        // Someone else changes it: the old stamp no longer matches.
        std::fs::write(&p, b"A=1\r\nB=4 changed elsewhere\r\n").unwrap();
        match save_local(&p, b"mine", Some(stamp2)) {
            Err(SaveError::Changed(Some(_))) => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"A=1\r\nB=4 changed elsewhere\r\n"
        );
        // Overwrite on purpose.
        save_local(&p, b"mine", None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"mine");
        std::fs::remove_file(&p).unwrap();
        assert!(matches!(
            save_local(&p, b"x", Some(stamp2)),
            Err(SaveError::Changed(None))
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn local_read_refuses_folders_and_big_files() {
        let d = dir("read");
        assert!(
            read_local(&d.display().to_string())
                .unwrap_err()
                .contains("not a file")
        );
        let big = d.join("big.log");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_EDIT + 1).unwrap();
        drop(f);
        assert!(
            read_local(&big.display().to_string())
                .unwrap_err()
                .contains("10 MiB")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn local_save_keeps_mode_and_edits_through_links() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("unix");
        let real = d.join("run.sh");
        std::fs::write(&real, b"echo a\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o750)).unwrap();
        let link = d.join("link.sh");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let lp = link.display().to_string();
        let (_, st) = read_local(&lp).unwrap();
        save_local(&lp, b"echo b\n", Some(st)).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&real).unwrap(), b"echo b\n");
        let mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o750);
        let _ = std::fs::remove_dir_all(&d);
    }
}
