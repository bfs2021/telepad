//! `ssh -N -L` through the host's normal Telepad login: the database port is
//! reached from the host itself and never opened to the network.

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::collector::{SshSettings, explain_ssh_error};
use crate::config::HostEntry;
use crate::platform;

/// Every tunnel's ssh process, so [`close_all`] can stop them when the app exits
/// (worker threads may still be busy then, and Windows does not end child processes).
static OPEN: Mutex<Vec<Weak<Mutex<Child>>>> = Mutex::new(Vec::new());

/// Stop every tunnel's ssh process (on exit).
pub fn close_all() {
    let open = OPEN
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default();
    for child in open.iter().filter_map(Weak::upgrade) {
        if let Ok(mut c) = child.try_lock() {
            let _ = c.kill();
            let _ = c.wait();
        } else if let Ok(mut c) = child.lock() {
            let _ = c.kill();
        }
    }
}

/// A running local forward; the ssh process is stopped when this is dropped.
pub struct Tunnel {
    child: Arc<Mutex<Child>>,
    pub local_port: u16,
    stderr: Arc<Mutex<String>>,
}

/// ssh arguments for the forward (options before the alias).
pub fn tunnel_args(
    settings: &SshSettings,
    host: &HostEntry,
    local_port: u16,
    address: &str,
    port: u16,
) -> Vec<String> {
    let mut args = settings.args(host, true);
    let alias = args.pop().unwrap_or_default();
    let target = if address.contains(':') {
        format!("[{address}]")
    } else {
        address.to_string()
    };
    args.extend([
        "-N".into(),
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-o".into(),
        "ServerAliveInterval=30".into(),
        "-L".into(),
        format!("127.0.0.1:{local_port}:{target}:{port}"),
        alias,
    ]);
    args
}

impl Tunnel {
    /// Start the forward and wait until its local port accepts connections.
    pub fn open(
        settings: &SshSettings,
        host: &HostEntry,
        address: &str,
        port: u16,
    ) -> Result<Self, String> {
        let local_port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .map_err(|e| format!("no free local port: {e}"))?;
        let mut cmd = Command::new(&settings.ssh_program);
        cmd.args(tunnel_args(settings, host, local_port, address, port))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
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
        let child = Arc::new(Mutex::new(child));
        if let Ok(mut open) = OPEN.lock() {
            open.retain(|w| w.strong_count() > 0);
            open.push(Arc::downgrade(&child));
        }
        let tunnel = Tunnel {
            child,
            local_port,
            stderr,
        };
        let deadline = Instant::now() + settings.connect_timeout + Duration::from_secs(7);
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, local_port));
        loop {
            let exited = tunnel
                .child
                .lock()
                .ok()
                .and_then(|mut c| c.try_wait().ok().flatten());
            if let Some(status) = exited {
                thread::sleep(Duration::from_millis(100));
                return Err(explain_tunnel_error(&tunnel.stderr_text(), status.code()));
            }
            if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                return Ok(tunnel);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "SSH tunnel to {} did not start in time{}",
                    host.alias,
                    tail(&tunnel.stderr_text())
                ));
            }
            thread::sleep(Duration::from_millis(150));
        }
    }

    /// ssh's messages so far (e.g. `channel 2: open failed: connect failed`).
    pub fn stderr_text(&self) -> String {
        self.stderr.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn alive(&mut self) -> bool {
        self.child
            .lock()
            .is_ok_and(|mut c| matches!(c.try_wait(), Ok(None)))
    }

    /// A driver error, explained with what ssh said about the forward.
    pub fn explain(&self, driver_error: &str) -> String {
        let ssh = self.stderr_text();
        if ssh.contains("open failed") || ssh.contains("connect failed") {
            let why = if ssh.contains("administratively prohibited") {
                "the SSH server does not allow port forwarding (AllowTcpForwarding no)"
            } else {
                "the host cannot reach the database address (stopped, or bound to another address)"
            };
            format!("{driver_error} — SSH tunnel: {why}")
        } else {
            driver_error.to_string()
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn tail(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        String::new()
    } else {
        format!(": {}", t.lines().last().unwrap_or(t))
    }
}

/// Why ssh exited before the forward was up.
pub fn explain_tunnel_error(stderr: &str, code: Option<i32>) -> String {
    if stderr.contains("administratively prohibited") || stderr.contains("forwarding disabled") {
        return "The SSH server does not allow port forwarding (AllowTcpForwarding no in sshd_config)".into();
    }
    if stderr.contains("Could not request local forwarding")
        || stderr.contains("bind") && stderr.contains("Address already in use")
    {
        return format!("Could not open the local end of the tunnel{}", tail(stderr));
    }
    explain_ssh_error(stderr, code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn forward_options_come_before_the_alias() {
        let s = SshSettings::new(PathBuf::from("/tmp/telepad.ssh_config"));
        let h = HostEntry {
            alias: "db1".into(),
            ..Default::default()
        };
        let a = tunnel_args(&s, &h, 40001, "172.18.0.5", 3306);
        assert_eq!(a.last().map(String::as_str), Some("db1"));
        assert!(
            a.windows(2)
                .any(|w| w[0] == "-L" && w[1] == "127.0.0.1:40001:172.18.0.5:3306")
        );
        assert!(a.contains(&"BatchMode=yes".to_string()));
        assert!(a.contains(&"ExitOnForwardFailure=yes".to_string()));
        let v6 = tunnel_args(&s, &h, 40001, "::1", 5432);
        assert!(v6.contains(&"127.0.0.1:40001:[::1]:5432".to_string()));
    }

    #[test]
    fn explains_forwarding_refusals() {
        assert!(
            explain_tunnel_error(
                "channel 0: open failed: administratively prohibited: open failed",
                Some(255)
            )
            .contains("AllowTcpForwarding")
        );
    }
}
