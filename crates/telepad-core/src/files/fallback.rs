//! Hosts without an SFTP server: list folders with a read-only POSIX `sh`
//! script and copy with `scp -O` (the old protocol, which needs no SFTP).

use super::{Entry, Kind};
use crate::collector::SshSettings;
use crate::config::HostEntry;
use crate::platform::shell_quote;

/// Script that prints the absolute path of `dir` (`~` = home), then one
/// line per entry: `type|size|mtime|mode|owner|name`. Read-only.
pub fn list_script(dir: &str) -> String {
    let cd = if dir.is_empty() || dir == "~" {
        "cd || exit 3".to_string()
    } else {
        format!("cd -- {} || exit 3", shell_quote(dir))
    };
    format!(
        "LC_ALL=C; export LC_ALL\n{cd}\npwd\nfor f in .[!.]* ..?* *; do\n  [ -e \"$f\" ] || [ -L \"$f\" ] || continue\n  stat -c '%F|%s|%Y|%a|%U|%n' -- \"$f\" 2>/dev/null\ndone\nexit 0\n"
    )
}

/// (absolute folder, entries) from [`list_script`] output.
pub fn parse_list(out: &str) -> Result<(String, Vec<Entry>), String> {
    let mut lines = out.lines();
    let dir = lines
        .next()
        .map(str::trim)
        .filter(|d| d.starts_with('/'))
        .ok_or("Cannot open the folder on the host")?
        .to_string();
    let mut v = Vec::new();
    for l in lines {
        let p: Vec<&str> = l.splitn(6, '|').collect();
        if p.len() < 6 {
            continue;
        }
        let kind = match p[0] {
            "directory" => Kind::Dir,
            "symbolic link" => Kind::Link,
            t if t.contains("regular") => Kind::File,
            _ => Kind::Other,
        };
        let name = p[5].to_string();
        v.push(Entry {
            hidden: name.starts_with('.'),
            name,
            kind,
            link_dir: false,
            size: p[1].parse().unwrap_or(0),
            mtime: p[2].parse().ok(),
            perm: u32::from_str_radix(p[3], 8).ok(),
            owner: p[4].to_string(),
        });
    }
    Ok((dir, v))
}

/// `alias:'remote path'` for scp's old protocol (the remote shell reads it).
pub fn remote_spec(host: &HostEntry, path: &str) -> String {
    format!("{}:{}", host.alias, shell_quote(path))
}

/// scp arguments (`-O`: no SFTP needed; `-r`: folders). Downloads add `-p`
/// (keep times and modes). Uploads do not: Windows reports every file as
/// 0666 / 0777, and `-p` would make them world-writable on the host; without
/// it the host's umask decides.
pub fn scp_args(
    settings: &SshSettings,
    host: &HostEntry,
    upload: bool,
    local: &str,
    remote: &str,
) -> Vec<String> {
    let mut a = vec![
        "-F".to_string(),
        settings.config_path.display().to_string(),
        "-O".into(),
    ];
    if !upload {
        a.push("-p".into());
    }
    a.extend([
        "-r".to_string(),
        "-q".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={}", settings.connect_timeout.as_secs()),
    ]);
    if host.identity_file.is_none()
        && let Some(id) = &settings.fallback_identity
    {
        a.extend(["-i".into(), id.display().to_string()]);
    }
    let remote = remote_spec(host, remote);
    let (from, to) = if upload {
        (local.to_string(), remote)
    } else {
        (remote, local.to_string())
    };
    a.extend(["--".into(), from, to]);
    a
}

/// `scp` next to the ssh binary Telepad uses (same OpenSSH).
pub fn scp_program(settings: &SshSettings) -> std::path::PathBuf {
    let name = if cfg!(windows) { "scp.exe" } else { "scp" };
    match settings.ssh_program.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(name),
        _ => name.into(),
    }
}

/// List a folder on a host without SFTP (read-only script over the normal login).
pub fn list(
    settings: &SshSettings,
    host: &HostEntry,
    dir: &str,
) -> Result<(String, Vec<Entry>), String> {
    let (out, err, code) = crate::collector::run_script(settings, host, &list_script(dir))?;
    if code == Some(3) {
        return Err(format!("Cannot open {dir} on the host"));
    }
    if code != Some(0) {
        return Err(crate::collector::explain_ssh_error(&err, code));
    }
    parse_list(&out)
}

/// Run scp and wait; `cancel` stops it. Upload: `local` into the remote folder
/// `remote`; download: the remote path `remote` into the local folder `local`.
pub fn copy(
    settings: &SshSettings,
    host: &HostEntry,
    upload: bool,
    local: &str,
    remote: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut c = Command::new(scp_program(settings));
    c.args(scp_args(settings, host, upload, local, remote))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    crate::platform::hide_console(&mut c);
    let mut child = c.spawn().map_err(|e| format!("cannot start scp: {e}"))?;
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(st)) => {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    use std::io::Read;
                    let _ = e.read_to_string(&mut err);
                }
                return if st.success() {
                    Ok(())
                } else {
                    Err(crate::collector::explain_ssh_error(&err, st.code()))
                };
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_script_and_parser() {
        let s = list_script("/srv/my app's");
        assert!(s.contains("cd -- '/srv/my app'\\''s'"), "{s}");
        assert!(list_script("~").contains("cd || exit 3"));
        let (dir, e) = parse_list(
            "/srv/app\ndirectory|4096|1700000000|755|deploy|conf\nregular file|12|1700000001|640|root|a|b.txt\nsymbolic link|7|1|777|root|current\n",
        )
        .unwrap();
        assert_eq!(dir, "/srv/app");
        assert_eq!(e.len(), 3);
        assert!(e[0].is_dir());
        assert_eq!(
            (e[1].name.as_str(), e[1].size, e[1].perm),
            ("a|b.txt", 12, Some(0o640))
        );
        assert_eq!(e[2].kind, Kind::Link);
        assert!(parse_list("cannot cd\n").is_err());
    }

    #[test]
    fn scp_quotes_the_remote_path() {
        let h = HostEntry::new("web1");
        assert_eq!(remote_spec(&h, "/srv/a b"), "web1:'/srv/a b'");
        let s = SshSettings::new(std::path::PathBuf::from("/tmp/c"));
        let up = scp_args(&s, &h, true, "/tmp/x", "/srv/a b");
        assert_eq!(
            &up[up.len() - 3..],
            &["--".to_string(), "/tmp/x".into(), "web1:'/srv/a b'".into()]
        );
        assert!(up.contains(&"-O".to_string()));
        // Uploads never copy modes (Windows would make files world-writable).
        assert!(!up.contains(&"-p".to_string()));
        let down = scp_args(&s, &h, false, "/tmp/x", "/srv/a b");
        assert_eq!(
            &down[down.len() - 3..],
            &["--".to_string(), "web1:'/srv/a b'".into(), "/tmp/x".into()]
        );
        assert!(down.contains(&"-p".to_string()));
    }
}
