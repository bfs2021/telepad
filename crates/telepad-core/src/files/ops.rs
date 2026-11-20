//! File operations of the file window (#63): new folder / file, rename,
//! delete, permissions and sizes, the same code for this computer and for the
//! host (over SFTP). Every name is checked first; folders are walked without
//! following links; one failing item never stops the others.

use std::sync::atomic::{AtomicBool, Ordering};

use super::sftp::{Attrs, Sftp};
use super::{Entry, Kind, list_local, local_join};

/// Stop walking a folder tree after this many entries.
pub const MAX_WALK: usize = 100_000;

/// What a recursive operation did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub done: usize,
    /// `path: reason`, one per failed item.
    pub errors: Vec<String>,
}

/// Size of one item: files and folders inside, total bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Measure {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// The walk stopped at [`MAX_WALK`] entries (or a folder could not be read).
    pub partial: bool,
}

/// A folder tree: this computer or the host.
pub trait Fs {
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>, String>;
    fn join(&self, dir: &str, name: &str) -> String;
    fn exists(&mut self, path: &str) -> bool;
    fn mkdir(&mut self, path: &str) -> Result<(), String>;
    /// Create an empty file; fails when it exists.
    fn create(&mut self, path: &str) -> Result<(), String>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String>;
    /// Remove a file or a link (never what a link points to).
    fn unlink(&mut self, path: &str, e: &Entry) -> Result<(), String>;
    fn rmdir(&mut self, path: &str) -> Result<(), String>;
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), String>;
    /// Names differing only in case are the same file here.
    fn case_insensitive(&self) -> bool {
        false
    }
}

/// Check a new name typed by the user: one path element, not `.` / `..`.
/// `local`: also refuse characters this computer cannot store.
pub fn check_name(name: &str, local: bool) -> Result<(), String> {
    if name.is_empty() {
        return Err("The name is empty".into());
    }
    if name == "." || name == ".." {
        return Err(format!("\"{name}\" is not a valid name"));
    }
    if name.contains('/') || name.contains('\0') {
        return Err("A name cannot contain / ".into());
    }
    if name.trim() != name {
        return Err("A name cannot start or end with a space".into());
    }
    if name.chars().any(char::is_control) {
        return Err("A name cannot contain control characters".into());
    }
    if local && cfg!(windows) {
        if let Some(c) = name.chars().find(|c| r#"\<>:"|?*"#.contains(*c)) {
            return Err(format!("A name cannot contain {c} on Windows"));
        }
        if name.ends_with('.') {
            return Err("A name cannot end with a dot on Windows".into());
        }
    } else if local && name.contains('\\') {
        return Err("A name cannot contain \\".into());
    }
    Ok(())
}

pub fn new_folder(fs: &mut dyn Fs, dir: &str, name: &str, local: bool) -> Result<String, String> {
    check_name(name, local)?;
    let p = fs.join(dir, name);
    if fs.exists(&p) {
        return Err(format!("{name} already exists"));
    }
    fs.mkdir(&p).map(|()| p)
}

pub fn new_file(fs: &mut dyn Fs, dir: &str, name: &str, local: bool) -> Result<String, String> {
    check_name(name, local)?;
    let p = fs.join(dir, name);
    if fs.exists(&p) {
        return Err(format!("{name} already exists"));
    }
    fs.create(&p).map(|()| p)
}

/// Rename inside one folder; never replaces another file.
pub fn rename(fs: &mut dyn Fs, dir: &str, from: &str, to: &str, local: bool) -> Result<(), String> {
    check_name(to, local)?;
    if from == to {
        return Ok(());
    }
    let (a, b) = (fs.join(dir, from), fs.join(dir, to));
    let same = fs.case_insensitive() && from.to_lowercase() == to.to_lowercase();
    if !same && fs.exists(&b) {
        return Err(format!("{to} already exists"));
    }
    fs.rename(&a, &b)
}

fn real_dir(e: &Entry) -> bool {
    e.kind == Kind::Dir
}

/// Files, folders and bytes of each item (links count as files, not followed).
pub fn measure(fs: &mut dyn Fs, dir: &str, items: &[Entry], cancel: &AtomicBool) -> Vec<Measure> {
    items
        .iter()
        .map(|e| {
            let mut m = Measure::default();
            let mut seen = 0;
            let p = fs.join(dir, &e.name);
            walk_measure(fs, &p, e, &mut m, &mut seen, cancel);
            m
        })
        .collect()
}

fn walk_measure(
    fs: &mut dyn Fs,
    path: &str,
    e: &Entry,
    m: &mut Measure,
    seen: &mut usize,
    cancel: &AtomicBool,
) {
    *seen += 1;
    if !real_dir(e) {
        m.files += 1;
        m.bytes += e.size;
        return;
    }
    m.dirs += 1;
    if *seen > MAX_WALK || cancel.load(Ordering::Relaxed) {
        m.partial = true;
        return;
    }
    match fs.list(path) {
        Ok(children) => {
            for c in children {
                let p = fs.join(path, &c.name);
                walk_measure(fs, &p, &c, m, seen, cancel);
            }
        }
        Err(_) => m.partial = true,
    }
}

/// Delete the items of `dir` (folders with everything in them). Links are
/// removed, never followed.
pub fn delete(fs: &mut dyn Fs, dir: &str, items: &[Entry], cancel: &AtomicBool) -> Report {
    let mut r = Report::default();
    for e in items {
        if cancel.load(Ordering::Relaxed) {
            r.errors.push("Cancelled".into());
            break;
        }
        let p = fs.join(dir, &e.name);
        if delete_one(fs, &p, e, &mut r, cancel) {
            r.done += 1;
        }
    }
    r
}

fn delete_one(fs: &mut dyn Fs, path: &str, e: &Entry, r: &mut Report, cancel: &AtomicBool) -> bool {
    if !real_dir(e) {
        return match fs.unlink(path, e) {
            Ok(()) => true,
            Err(err) => {
                r.errors.push(format!("{path}: {err}"));
                false
            }
        };
    }
    let children = match fs.list(path) {
        Ok(c) => c,
        Err(err) => {
            r.errors.push(format!("{path}: {err}"));
            return false;
        }
    };
    let mut all = true;
    for c in &children {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let p = fs.join(path, &c.name);
        all &= delete_one(fs, &p, c, r, cancel);
    }
    if !all {
        // A child is still there: the folder cannot go (already reported).
        return false;
    }
    match fs.rmdir(path) {
        Ok(()) => true,
        Err(err) => {
            r.errors.push(format!("{path}: {err}"));
            false
        }
    }
}

/// Mode for folders inside a recursive change: like `chmod -R`, plus `x`
/// wherever `r` is set, so folders stay openable (`chmod -R u=rw,go=r` would
/// lock everyone out of them).
pub fn dir_mode(mode: u32) -> u32 {
    let mut m = mode & 0o7777;
    for (r, x) in [(0o400, 0o100), (0o040, 0o010), (0o004, 0o001)] {
        if m & r != 0 {
            m |= x;
        }
    }
    m
}

/// Mode for files inside a recursive change: the read / write bits of `mode`;
/// execute only where the file already had some `x` (`chmod -R a+X`
/// style), and never setuid / setgid / sticky, so a folder's `2775` does not
/// turn every file into an executable.
pub fn file_mode(mode: u32, old: Option<u32>) -> u32 {
    let rw = mode & 0o666;
    if old.is_some_and(|o| o & 0o111 != 0) {
        rw | (mode & 0o111)
    } else {
        rw
    }
}

/// Set `mode` on the items; `recursive`: also on everything inside folders
/// (files [`file_mode`], folders [`dir_mode`]). Links are never changed or
/// followed. Contents are changed before their folder, so a mode without `x`
/// on a folder does not lock the walk out of it.
pub fn chmod(
    fs: &mut dyn Fs,
    dir: &str,
    items: &[Entry],
    mode: u32,
    recursive: bool,
    cancel: &AtomicBool,
) -> Report {
    let mut r = Report::default();
    for e in items {
        if e.kind == Kind::Link {
            r.errors.push(format!(
                "{}: links have no permissions of their own",
                e.name
            ));
            continue;
        }
        let p = fs.join(dir, &e.name);
        if recursive && real_dir(e) {
            chmod_inside(fs, &p, mode, &mut r, cancel);
        }
        set_mode(fs, &p, mode & 0o7777, &mut r);
    }
    r
}

fn set_mode(fs: &mut dyn Fs, path: &str, mode: u32, r: &mut Report) {
    match fs.chmod(path, mode) {
        Ok(()) => r.done += 1,
        Err(err) => r.errors.push(format!("{path}: {err}")),
    }
}

fn chmod_inside(fs: &mut dyn Fs, dir: &str, mode: u32, r: &mut Report, cancel: &AtomicBool) {
    let children = match fs.list(dir) {
        Ok(c) => c,
        Err(err) => {
            r.errors.push(format!("{dir}: {err}"));
            return;
        }
    };
    for c in children {
        if cancel.load(Ordering::Relaxed) {
            r.errors.push("Cancelled".into());
            return;
        }
        if c.kind == Kind::Link {
            continue;
        }
        let p = fs.join(dir, &c.name);
        if real_dir(&c) {
            chmod_inside(fs, &p, mode, r, cancel);
            set_mode(fs, &p, dir_mode(mode), r);
        } else {
            set_mode(fs, &p, file_mode(mode, c.perm), r);
        }
    }
}

/// This computer.
pub struct LocalFs;

impl Fs for LocalFs {
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>, String> {
        list_local(dir)
    }
    fn join(&self, dir: &str, name: &str) -> String {
        local_join(dir, name)
    }
    fn exists(&mut self, path: &str) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }
    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        std::fs::create_dir(path).map_err(|e| e.to_string())
    }
    fn create(&mut self, path: &str) -> Result<(), String> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| e.to_string())
    }
    fn unlink(&mut self, path: &str, e: &Entry) -> Result<(), String> {
        // A Windows link to a folder (symlink or junction) is removed as a folder.
        let r = if e.kind == Kind::Link && e.link_dir && cfg!(windows) {
            std::fs::remove_dir(path)
        } else {
            std::fs::remove_file(path)
        };
        r.map_err(|e| e.to_string())
    }
    fn rmdir(&mut self, path: &str) -> Result<(), String> {
        std::fs::remove_dir(path).map_err(|e| e.to_string())
    }
    #[cfg(unix)]
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    fn chmod(&mut self, _path: &str, _mode: u32) -> Result<(), String> {
        Err("Unix permissions do not exist on this computer".into())
    }
    fn case_insensitive(&self) -> bool {
        cfg!(any(windows, target_os = "macos"))
    }
}

/// The host, over SFTP (the login user's rights; never `sudo`).
impl Fs for Sftp {
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>, String> {
        Sftp::list(self, dir)
    }
    fn join(&self, dir: &str, name: &str) -> String {
        super::sftp::join(dir, name)
    }
    fn exists(&mut self, path: &str) -> bool {
        self.lstat(path).is_ok()
    }
    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        Sftp::mkdir(self, path, None)
    }
    fn create(&mut self, path: &str) -> Result<(), String> {
        self.create_new(path)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        Sftp::rename(self, from, to)
    }
    fn unlink(&mut self, path: &str, _e: &Entry) -> Result<(), String> {
        self.remove(path)
    }
    fn rmdir(&mut self, path: &str) -> Result<(), String> {
        Sftp::rmdir(self, path)
    }
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), String> {
        self.setstat(
            path,
            Attrs {
                perm: Some(mode),
                ..Default::default()
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "telepad-ops-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn entry(fs: &mut LocalFs, dir: &str, name: &str) -> Entry {
        fs.list(dir)
            .unwrap()
            .into_iter()
            .find(|e| e.name == name)
            .unwrap()
    }

    #[test]
    fn names_are_one_path_element() {
        for ok in ["a", "my site", "it's \"x\"", "ünï ñame", ".env", "a..b"] {
            assert!(check_name(ok, false).is_ok(), "{ok}");
        }
        for bad in [
            "", ".", "..", "a/b", "../x", "a\0b", " lead", "trail ", "a\nb",
        ] {
            assert!(check_name(bad, false).is_err(), "{bad:?}");
        }
        assert!(check_name("a\\b", true).is_err());
        if cfg!(windows) {
            assert!(check_name("a:b", true).is_err());
            assert!(check_name("x.", true).is_err());
            // The host side takes what POSIX takes.
            assert!(check_name("a:b", false).is_ok());
        }
    }

    #[test]
    fn files_inside_never_become_executable_or_setgid() {
        assert_eq!(file_mode(0o2775, Some(0o644)), 0o664);
        assert_eq!(file_mode(0o2775, Some(0o755)), 0o775);
        assert_eq!(file_mode(0o750, Some(0o700)), 0o750);
        assert_eq!(file_mode(0o640, None), 0o640);
        assert_eq!(file_mode(0o4755, Some(0o755)), 0o755);
    }

    #[test]
    fn dir_mode_keeps_folders_openable() {
        assert_eq!(dir_mode(0o644), 0o755);
        assert_eq!(dir_mode(0o640), 0o750);
        assert_eq!(dir_mode(0o600), 0o700);
        assert_eq!(dir_mode(0o755), 0o755);
        assert_eq!(dir_mode(0o000), 0o000);
    }

    #[test]
    fn local_new_rename_measure_delete() {
        let base = tmp("local");
        let dir = base.display().to_string();
        let mut fs = LocalFs;
        let cancel = AtomicBool::new(false);

        let f = new_folder(&mut fs, &dir, "my 'site' ü", true).unwrap();
        assert!(std::path::Path::new(&f).is_dir());
        assert!(
            new_folder(&mut fs, &dir, "my 'site' ü", true)
                .unwrap_err()
                .contains("exists")
        );
        let n = new_file(&mut fs, &f, "note one.txt", true).unwrap();
        std::fs::write(&n, b"made-up").unwrap();
        assert!(new_file(&mut fs, &f, "note one.txt", true).is_err());
        assert!(new_folder(&mut fs, &dir, "..", true).is_err());
        std::fs::create_dir(std::path::Path::new(&f).join("img")).unwrap();
        std::fs::write(
            std::path::Path::new(&f).join("img").join("b.bin"),
            [0u8; 100],
        )
        .unwrap();

        // Rename never replaces another file; a case change is allowed.
        std::fs::write(base.join("other.txt"), b"x").unwrap();
        assert!(rename(&mut fs, &f, "note one.txt", "../other.txt", true).is_err());
        rename(&mut fs, &f, "note one.txt", "Note One.txt", true).unwrap();
        assert!(
            rename(&mut fs, &dir, "other.txt", "my 'site' ü", true)
                .unwrap_err()
                .contains("exists")
        );

        let site = entry(&mut fs, &dir, "my 'site' ü");
        let m = measure(&mut fs, &dir, std::slice::from_ref(&site), &cancel);
        assert_eq!(
            m[0],
            Measure {
                files: 2,
                dirs: 2,
                bytes: 107,
                partial: false
            }
        );

        let other = entry(&mut fs, &dir, "other.txt");
        let r = delete(&mut fs, &dir, &[site, other], &cancel);
        assert_eq!(
            r,
            Report {
                done: 2,
                errors: vec![]
            }
        );
        assert!(fs.list(&dir).unwrap().is_empty());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_delete_never_follows_links_and_chmod_recurses() {
        let base = tmp("links");
        let dir = base.display().to_string();
        let keep = base.join("keep");
        std::fs::create_dir(&keep).unwrap();
        std::fs::write(keep.join("precious.txt"), b"x").unwrap();
        let tree = base.join("tree");
        std::fs::create_dir_all(tree.join("sub")).unwrap();
        std::fs::write(tree.join("sub").join("f"), b"x").unwrap();
        std::os::unix::fs::symlink(&keep, tree.join("to-keep")).unwrap();
        let mut fs = LocalFs;
        let cancel = AtomicBool::new(false);
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let keep_before = mode(&keep);

        let t = entry(&mut fs, &dir, "tree");
        let r = chmod(
            &mut fs,
            &dir,
            std::slice::from_ref(&t),
            0o640,
            true,
            &cancel,
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(mode(&tree), 0o640);
        // Without x on `tree` nothing inside can be looked at: give it back first.
        std::fs::set_permissions(&tree, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(mode(&tree.join("sub")), 0o750);
        assert_eq!(mode(&tree.join("sub").join("f")), 0o640);
        assert_eq!(mode(&keep), keep_before, "link target untouched");

        let r = delete(&mut fs, &dir, &[t], &cancel);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(!tree.exists());
        assert!(keep.join("precious.txt").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn delete_reports_per_item_and_continues() {
        let base = tmp("errors");
        let dir = base.display().to_string();
        let locked = base.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::write(locked.join("f"), b"x").unwrap();
        std::fs::write(base.join("free.txt"), b"x").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let mut fs = LocalFs;
        let cancel = AtomicBool::new(false);
        let items = fs.list(&dir).unwrap();
        let r = delete(&mut fs, &dir, &items, &cancel);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // root may delete anyway; otherwise one error and the free file is gone.
        assert!(!base.join("free.txt").exists());
        if locked.exists() {
            assert_eq!(r.done, 1);
            assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        }
        std::fs::remove_dir_all(&base).unwrap();
    }
}
