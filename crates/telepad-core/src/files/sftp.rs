//! A small synchronous SFTP (version 3) client that talks to the host's
//! `sftp-server` through the system OpenSSH client: `ssh -s <alias> sftp`.
//! Same login, keys and config as every other Telepad connection.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use super::{Entry, Kind};
use crate::collector::SshSettings;
use crate::config::HostEntry;
use crate::platform;

const INIT: u8 = 1;
const VERSION: u8 = 2;
const OPEN: u8 = 3;
const CLOSE: u8 = 4;
const READ: u8 = 5;
const WRITE: u8 = 6;
const LSTAT: u8 = 7;
const SETSTAT: u8 = 9;
const OPENDIR: u8 = 11;
const READDIR: u8 = 12;
const REMOVE: u8 = 13;
const MKDIR: u8 = 14;
const RMDIR: u8 = 15;
const REALPATH: u8 = 16;
const STAT: u8 = 17;
const RENAME: u8 = 18;

const STATUS: u8 = 101;
const HANDLE: u8 = 102;
const DATA: u8 = 103;
const NAME: u8 = 104;
const ATTRS: u8 = 105;

const FX_OK: u32 = 0;
const FX_EOF: u32 = 1;

const ATTR_SIZE: u32 = 0x1;
const ATTR_UIDGID: u32 = 0x2;
const ATTR_PERMISSIONS: u32 = 0x4;
const ATTR_ACMODTIME: u32 = 0x8;
const ATTR_EXTENDED: u32 = 0x8000_0000;

pub const OPEN_READ: u32 = 0x1;
pub const OPEN_WRITE: u32 = 0x2;
pub const OPEN_CREATE: u32 = 0x8;
pub const OPEN_TRUNCATE: u32 = 0x10;

/// Bytes per READ / WRITE request and how many are in flight at once.
const CHUNK: u32 = 32 * 1024;
const WINDOW: usize = 16;

/// File attributes (only what the server sent).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Attrs {
    pub size: Option<u64>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub perm: Option<u32>,
    pub atime: Option<u32>,
    pub mtime: Option<u32>,
}

impl Attrs {
    pub fn kind(&self) -> Kind {
        match self.perm.map(|p| p & 0o170000) {
            Some(0o040000) => Kind::Dir,
            Some(0o120000) => Kind::Link,
            Some(0o100000) => Kind::File,
            Some(_) => Kind::Other,
            None => Kind::File,
        }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let mut flags = 0;
        if self.size.is_some() {
            flags |= ATTR_SIZE;
        }
        if self.uid.is_some() && self.gid.is_some() {
            flags |= ATTR_UIDGID;
        }
        if self.perm.is_some() {
            flags |= ATTR_PERMISSIONS;
        }
        if self.atime.is_some() && self.mtime.is_some() {
            flags |= ATTR_ACMODTIME;
        }
        put_u32(out, flags);
        if let Some(s) = self.size {
            put_u64(out, s);
        }
        if let (Some(u), Some(g)) = (self.uid, self.gid) {
            put_u32(out, u);
            put_u32(out, g);
        }
        if let Some(p) = self.perm {
            put_u32(out, p);
        }
        if let (Some(a), Some(m)) = (self.atime, self.mtime) {
            put_u32(out, a);
            put_u32(out, m);
        }
    }
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &[u8]) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s);
}

/// Reads fields from a packet body.
struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn u32(&mut self) -> Result<u32, String> {
        if self.b.len() < 4 {
            return Err("short SFTP packet".into());
        }
        let (v, rest) = self.b.split_at(4);
        self.b = rest;
        Ok(u32::from_be_bytes([v[0], v[1], v[2], v[3]]))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok((u64::from(self.u32()?) << 32) | u64::from(self.u32()?))
    }
    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()? as usize;
        if self.b.len() < n {
            return Err("short SFTP packet".into());
        }
        let (v, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(v)
    }
    fn string(&mut self) -> Result<String, String> {
        Ok(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
    fn attrs(&mut self) -> Result<Attrs, String> {
        let flags = self.u32()?;
        let mut a = Attrs::default();
        if flags & ATTR_SIZE != 0 {
            a.size = Some(self.u64()?);
        }
        if flags & ATTR_UIDGID != 0 {
            a.uid = Some(self.u32()?);
            a.gid = Some(self.u32()?);
        }
        if flags & ATTR_PERMISSIONS != 0 {
            a.perm = Some(self.u32()?);
        }
        if flags & ATTR_ACMODTIME != 0 {
            a.atime = Some(self.u32()?);
            a.mtime = Some(self.u32()?);
        }
        if flags & ATTR_EXTENDED != 0 {
            for _ in 0..self.u32()? {
                self.bytes()?;
                self.bytes()?;
            }
        }
        Ok(a)
    }
}

/// A server reply: type, request id, body after the id.
struct Reply {
    kind: u8,
    id: u32,
    body: Vec<u8>,
}

impl Reply {
    fn reader(&self) -> Reader<'_> {
        Reader { b: &self.body }
    }
    /// `Err(message)` for a STATUS that is not OK; `Ok(())` for OK.
    fn status(&self) -> Result<u32, String> {
        let mut r = self.reader();
        let code = r.u32()?;
        let msg = r.string().unwrap_or_default();
        if code == FX_OK || code == FX_EOF {
            Ok(code)
        } else {
            Err(status_text(code, &msg))
        }
    }
}

fn status_text(code: u32, msg: &str) -> String {
    let what = match code {
        2 => "No such file or folder",
        3 => "Permission denied",
        4 => "Failure",
        5 => "Bad message",
        6 => "No connection",
        7 => "Connection lost",
        8 => "Not supported by the server",
        _ => "SFTP error",
    };
    if msg.is_empty() || msg.eq_ignore_ascii_case(what) {
        what.to_string()
    } else {
        format!("{what}: {msg}")
    }
}

/// ssh arguments for the SFTP subsystem (`-s` before the alias, `sftp` after).
pub fn sftp_args(settings: &SshSettings, host: &HostEntry) -> Vec<String> {
    let mut a = settings.args(host, true);
    let alias = a.pop().unwrap_or_default();
    a.extend([
        "-o".into(),
        "ServerAliveInterval=30".into(),
        "-s".into(),
        alias,
        "sftp".into(),
    ]);
    a
}

/// One SFTP session (one ssh process).
pub struct Sftp {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    next: u32,
    stderr: Arc<Mutex<String>>,
}

impl Drop for Sftp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Sftp {
    /// Start `ssh -s <alias> sftp` and do the version handshake.
    pub fn connect(settings: &SshSettings, host: &HostEntry) -> Result<Self, String> {
        let mut cmd = Command::new(&settings.ssh_program);
        cmd.args(sftp_args(settings, host))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        platform::hide_console(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", settings.ssh_program.display()))?;
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(mut pipe) = child.stderr.take() {
            let buf = stderr.clone();
            thread::spawn(move || {
                let mut chunk = [0u8; 1024];
                while let Ok(n) = pipe.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut b) = buf.lock() {
                        b.push_str(&String::from_utf8_lossy(&chunk[..n]));
                    }
                }
            });
        }
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            return Err("ssh has no stdin / stdout".into());
        };
        let mut s = Sftp {
            child,
            input,
            output,
            next: 1,
            stderr,
        };
        let mut init = Vec::new();
        put_u32(&mut init, 3);
        if let Err(e) = s.send_raw(INIT, &init).and_then(|()| s.read_packet()) {
            return Err(s.explain(&e));
        }
        Ok(s)
    }

    /// Why the session failed, from ssh's own messages when there are any.
    fn explain(&mut self, e: &str) -> String {
        thread::sleep(std::time::Duration::from_millis(150));
        let code = self.child.try_wait().ok().flatten().and_then(|s| s.code());
        let err = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
        if err.contains("subsystem request failed") {
            return "The host has no SFTP server (sftp-server / Subsystem sftp in sshd_config)"
                .into();
        }
        if err.trim().is_empty() {
            return format!("SFTP: {e}");
        }
        crate::collector::explain_ssh_error(&err, code)
    }

    fn send_raw(&mut self, kind: u8, body: &[u8]) -> Result<(), String> {
        let mut p = Vec::with_capacity(body.len() + 5);
        put_u32(&mut p, body.len() as u32 + 1);
        p.push(kind);
        p.extend_from_slice(body);
        self.input.write_all(&p).map_err(|e| e.to_string())?;
        self.input.flush().map_err(|e| e.to_string())
    }

    /// Send a request; returns its id.
    fn send(&mut self, kind: u8, fill: impl FnOnce(&mut Vec<u8>)) -> Result<u32, String> {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        let mut b = Vec::new();
        put_u32(&mut b, id);
        fill(&mut b);
        self.send_raw(kind, &b).map_err(|e| self.explain(&e))?;
        Ok(id)
    }

    fn read_packet(&mut self) -> Result<Reply, String> {
        let mut len = [0u8; 4];
        self.output
            .read_exact(&mut len)
            .map_err(|e| e.to_string())?;
        let len = u32::from_be_bytes(len) as usize;
        if len == 0 || len > 1 << 24 {
            return Err(format!("bad SFTP packet length {len}"));
        }
        let mut buf = vec![0u8; len];
        self.output
            .read_exact(&mut buf)
            .map_err(|e| e.to_string())?;
        let kind = buf[0];
        if kind == VERSION {
            return Ok(Reply {
                kind,
                id: 0,
                body: buf[1..].to_vec(),
            });
        }
        if buf.len() < 5 {
            return Err("short SFTP packet".into());
        }
        let id = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]);
        Ok(Reply {
            kind,
            id,
            body: buf[5..].to_vec(),
        })
    }

    fn recv(&mut self, id: u32) -> Result<Reply, String> {
        let r = self.read_packet().map_err(|e| self.explain(&e))?;
        if r.id != id {
            return Err(format!(
                "SFTP reply out of order ({} instead of {id})",
                r.id
            ));
        }
        Ok(r)
    }

    fn call(&mut self, kind: u8, fill: impl FnOnce(&mut Vec<u8>)) -> Result<Reply, String> {
        let id = self.send(kind, fill)?;
        self.recv(id)
    }

    fn expect_ok(&mut self, kind: u8, fill: impl FnOnce(&mut Vec<u8>)) -> Result<(), String> {
        let r = self.call(kind, fill)?;
        match r.kind {
            STATUS => r.status().map(|_| ()),
            k => Err(format!("unexpected SFTP reply {k}")),
        }
    }

    fn expect_handle(
        &mut self,
        kind: u8,
        fill: impl FnOnce(&mut Vec<u8>),
    ) -> Result<Vec<u8>, String> {
        let r = self.call(kind, fill)?;
        match r.kind {
            HANDLE => Ok(r.reader().bytes()?.to_vec()),
            STATUS => Err(r.status().err().unwrap_or_else(|| "no handle".into())),
            k => Err(format!("unexpected SFTP reply {k}")),
        }
    }

    fn expect_attrs(&mut self, kind: u8, path: &str) -> Result<Attrs, String> {
        let r = self.call(kind, |b| put_str(b, path.as_bytes()))?;
        match r.kind {
            ATTRS => r.reader().attrs(),
            STATUS => Err(r.status().err().unwrap_or_else(|| "no attributes".into())),
            k => Err(format!("unexpected SFTP reply {k}")),
        }
    }

    /// Absolute form of `path` (`.` = the login user's home).
    pub fn realpath(&mut self, path: &str) -> Result<String, String> {
        let r = self.call(REALPATH, |b| put_str(b, path.as_bytes()))?;
        match r.kind {
            NAME => {
                let mut rd = r.reader();
                if rd.u32()? == 0 {
                    return Err("empty REALPATH reply".into());
                }
                rd.string()
            }
            STATUS => Err(r.status().err().unwrap_or_else(|| "no path".into())),
            k => Err(format!("unexpected SFTP reply {k}")),
        }
    }

    pub fn stat(&mut self, path: &str) -> Result<Attrs, String> {
        self.expect_attrs(STAT, path)
    }

    pub fn lstat(&mut self, path: &str) -> Result<Attrs, String> {
        self.expect_attrs(LSTAT, path)
    }

    /// The entries of a folder (without `.` and `..`); links show the type
    /// of their target.
    pub fn list(&mut self, dir: &str) -> Result<Vec<Entry>, String> {
        let h = self.expect_handle(OPENDIR, |b| put_str(b, dir.as_bytes()))?;
        let mut out = Vec::new();
        let result = loop {
            let r = match self.call(READDIR, |b| put_str(b, &h)) {
                Ok(r) => r,
                Err(e) => break Err(e),
            };
            match r.kind {
                NAME => {
                    let mut rd = r.reader();
                    let n = rd.u32()?;
                    for _ in 0..n {
                        let name = rd.string()?;
                        let long = rd.string()?;
                        let attrs = rd.attrs()?;
                        if name == "." || name == ".." {
                            continue;
                        }
                        out.push(Entry::from_sftp(name, &long, attrs));
                    }
                }
                STATUS => match r.status() {
                    Ok(_) => break Ok(()),
                    Err(e) => break Err(e),
                },
                k => break Err(format!("unexpected SFTP reply {k}")),
            }
        };
        let _ = self.close(&h);
        result?;
        // Links: show folders as folders (stat follows the link).
        for e in out.iter_mut().filter(|e| e.kind == Kind::Link) {
            let p = join(dir, &e.name);
            if let Ok(a) = self.stat(&p) {
                e.link_dir = a.kind() == Kind::Dir;
            }
        }
        Ok(out)
    }

    fn close(&mut self, handle: &[u8]) -> Result<(), String> {
        self.expect_ok(CLOSE, |b| put_str(b, handle))
    }

    pub fn mkdir(&mut self, path: &str, perm: Option<u32>) -> Result<(), String> {
        self.expect_ok(MKDIR, |b| {
            put_str(b, path.as_bytes());
            Attrs {
                perm,
                ..Default::default()
            }
            .encode(b);
        })
    }

    pub fn remove(&mut self, path: &str) -> Result<(), String> {
        self.expect_ok(REMOVE, |b| put_str(b, path.as_bytes()))
    }

    pub fn rmdir(&mut self, path: &str) -> Result<(), String> {
        self.expect_ok(RMDIR, |b| put_str(b, path.as_bytes()))
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.expect_ok(RENAME, |b| {
            put_str(b, from.as_bytes());
            put_str(b, to.as_bytes());
        })
    }

    pub fn setstat(&mut self, path: &str, attrs: Attrs) -> Result<(), String> {
        self.expect_ok(SETSTAT, |b| {
            put_str(b, path.as_bytes());
            attrs.encode(b);
        })
    }

    /// Copy a remote file into `dest` (any writer), `progress(bytes)` after
    /// each chunk; stops early when `cancel` is set. Returns bytes copied.
    pub fn download(
        &mut self,
        remote: &str,
        dest: &mut dyn Write,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, String> {
        let h = self.expect_handle(OPEN, |b| {
            put_str(b, remote.as_bytes());
            put_u32(b, OPEN_READ);
            Attrs::default().encode(b);
        })?;
        let result = self.download_handle(&h, dest, cancel, progress);
        let _ = self.close(&h);
        result
    }

    fn download_handle(
        &mut self,
        h: &[u8],
        dest: &mut dyn Write,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, String> {
        // Requests in flight: (id, offset). Replies come back in order.
        let mut inflight: std::collections::VecDeque<(u32, u64)> = Default::default();
        let mut next_offset = 0u64;
        let mut written = 0u64;
        let mut eof = false;
        loop {
            while !eof && inflight.len() < WINDOW {
                let off = next_offset;
                let id = self.send(READ, |b| {
                    put_str(b, h);
                    put_u64(b, off);
                    put_u32(b, CHUNK);
                })?;
                inflight.push_back((id, off));
                next_offset += u64::from(CHUNK);
            }
            let Some((id, off)) = inflight.pop_front() else {
                break;
            };
            let r = self.recv(id)?;
            match r.kind {
                DATA => {
                    let data = r.reader().bytes()?.to_vec();
                    if off != written {
                        return Err("SFTP read out of order".into());
                    }
                    dest.write_all(&data).map_err(|e| e.to_string())?;
                    written += data.len() as u64;
                    progress(data.len() as u64);
                    if (data.len() as u32) < CHUNK {
                        // Short read: the file ends here (or shrank); ask for
                        // the rest from this point on.
                        eof = data.is_empty();
                        if !eof {
                            for (rid, _) in inflight.drain(..) {
                                let _ = self.recv(rid)?;
                            }
                            next_offset = written;
                        }
                    }
                }
                STATUS => {
                    r.status()?;
                    eof = true;
                }
                k => return Err(format!("unexpected SFTP reply {k}")),
            }
            if cancel.load(Ordering::Relaxed) {
                for (rid, _) in inflight.drain(..) {
                    let _ = self.recv(rid);
                }
                return Err("Cancelled".into());
            }
        }
        dest.flush().map_err(|e| e.to_string())?;
        Ok(written)
    }

    /// Write `src` to the remote file (created or truncated) with `perm`.
    pub fn upload(
        &mut self,
        src: &mut dyn Read,
        remote: &str,
        perm: Option<u32>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, String> {
        let h = self.expect_handle(OPEN, |b| {
            put_str(b, remote.as_bytes());
            put_u32(b, OPEN_WRITE | OPEN_CREATE | OPEN_TRUNCATE);
            Attrs {
                perm,
                ..Default::default()
            }
            .encode(b);
        })?;
        let result = self.upload_handle(&h, src, cancel, progress);
        let closed = self.close(&h);
        let n = result?;
        closed?;
        Ok(n)
    }

    fn upload_handle(
        &mut self,
        h: &[u8],
        src: &mut dyn Read,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, String> {
        let mut inflight: std::collections::VecDeque<(u32, u64)> = Default::default();
        let mut offset = 0u64;
        let mut done = false;
        let mut buf = vec![0u8; CHUNK as usize];
        loop {
            while !done && inflight.len() < WINDOW {
                let n = read_full(src, &mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    done = true;
                    break;
                }
                let off = offset;
                let chunk = &buf[..n];
                let id = self.send(WRITE, |b| {
                    put_str(b, h);
                    put_u64(b, off);
                    put_str(b, chunk);
                })?;
                inflight.push_back((id, n as u64));
                offset += n as u64;
                if cancel.load(Ordering::Relaxed) {
                    done = true;
                }
            }
            let Some((id, n)) = inflight.pop_front() else {
                break;
            };
            let r = self.recv(id)?;
            match r.kind {
                STATUS => {
                    r.status()?;
                    progress(n);
                }
                k => return Err(format!("unexpected SFTP reply {k}")),
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        Ok(offset)
    }
}

fn read_full(src: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match src.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// `dir/name` with exactly one slash (remote paths are always `/`).
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Parent of a remote path (`/` stays `/`).
pub fn parent(path: &str) -> String {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => p[..i].to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attrs_round_trip_and_kinds() {
        let a = Attrs {
            size: Some(1 << 40),
            uid: Some(1000),
            gid: Some(1000),
            perm: Some(0o100644),
            atime: Some(1),
            mtime: Some(1_700_000_000),
        };
        let mut b = Vec::new();
        a.encode(&mut b);
        assert_eq!(Reader { b: &b }.attrs().unwrap(), a);
        assert_eq!(a.kind(), Kind::File);
        assert_eq!(
            Attrs {
                perm: Some(0o040755),
                ..a
            }
            .kind(),
            Kind::Dir
        );
        assert_eq!(
            Attrs {
                perm: Some(0o120777),
                ..a
            }
            .kind(),
            Kind::Link
        );
        let mut e = Vec::new();
        Attrs::default().encode(&mut e);
        assert_eq!(e, vec![0, 0, 0, 0]);
    }

    #[test]
    fn reader_rejects_short_packets() {
        assert!(
            Reader {
                b: &[0, 0, 0, 9, 1]
            }
            .bytes()
            .is_err()
        );
        assert!(Reader { b: &[0, 0] }.u32().is_err());
    }

    #[test]
    fn remote_paths() {
        assert_eq!(join("/home/ops", "a b.txt"), "/home/ops/a b.txt");
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(parent("/home/ops/"), "/home");
        assert_eq!(parent("/home"), "/");
        assert_eq!(parent("/"), "/");
    }

    #[test]
    fn subsystem_args_put_the_alias_before_sftp() {
        let s = SshSettings::new(std::path::PathBuf::from("/tmp/telepad.ssh_config"));
        let h = HostEntry::new("web1");
        let a = sftp_args(&s, &h);
        assert_eq!(
            &a[a.len() - 3..],
            &["-s".to_string(), "web1".into(), "sftp".into()]
        );
        assert!(a.contains(&"BatchMode=yes".to_string()));
    }

    #[test]
    fn status_messages() {
        assert_eq!(
            status_text(2, "No such file"),
            "No such file or folder: No such file"
        );
        assert_eq!(status_text(3, ""), "Permission denied");
    }
}
