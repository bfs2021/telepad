//! Which containers and hosts run a database Telepad can open, from data the
//! collector already has (container images and IPs, listening ports).

use crate::model::{Container, HostReport, Listener};

use super::Engine;

/// A database Telepad can reach through the host's SSH login.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct DbTarget {
    pub engine: Engine,
    /// Host alias (ssh config `Host`).
    pub host: String,
    /// Container name; empty for a database installed on the host.
    pub container: String,
    /// Container id, for reading the user name from its environment.
    pub container_id: String,
    /// Address the tunnel connects to, seen from the host.
    pub address: String,
    pub port: u16,
}

impl DbTarget {
    pub fn engine(&self) -> Engine {
        self.engine
    }
    /// Stable key for the saved connection and the stored password: the
    /// container name survives a re-create, its IP does not.
    pub fn key(&self) -> String {
        let place = if self.container.is_empty() {
            format!("port {}", self.port)
        } else {
            format!("container {}", self.container)
        };
        format!("{} / {} / {}", self.host, place, self.engine().as_str())
    }
    /// Window title: `shop-db on dockercenter`, `PostgreSQL :5432 on db1`.
    pub fn title(&self) -> String {
        if self.container.is_empty() {
            format!("{} :{} on {}", self.engine().title(), self.port, self.host)
        } else {
            format!("{} on {}", self.container, self.host)
        }
    }
}

/// Engine of a database image (`mariadb:11`, `bitnami/postgresql`,
/// `registry.example/team/mysql-server`), ignoring tools such as exporters.
pub fn image_engine(image: &str) -> Option<Engine> {
    let img = image.to_ascii_lowercase();
    let name = img.rsplit('/').next().unwrap_or(&img);
    let name = name.split(['@', ':']).next().unwrap_or(name);
    const TOOLS: [&str; 7] = [
        "exporter", "admin", "backup", "adminer", "bouncer", "dump", "proxy",
    ];
    if TOOLS.iter().any(|t| name.contains(t)) {
        return None;
    }
    if ["mariadb", "mysql", "percona"]
        .iter()
        .any(|e| name.contains(e))
    {
        Some(Engine::MySql)
    } else if ["postgres", "postgis", "timescale"]
        .iter()
        .any(|e| name.contains(e))
    {
        Some(Engine::Postgres)
    } else {
        None
    }
}

/// A running database container: tunnel to its IP on its Docker network, or
/// to a published port on the host when it has no reachable IP.
pub fn container_target(host: &str, c: &Container) -> Option<DbTarget> {
    let engine = image_engine(&c.image)?;
    if !c.running() {
        return None;
    }
    let port = engine.default_port();
    let ip = c
        .networks
        .iter()
        .map(|(_, ip)| ip.trim())
        .find(|ip| !ip.is_empty() && *ip != "-");
    let (address, port) = match ip {
        Some(ip) => (ip.to_string(), port),
        None => {
            let published = c.published.iter().find(|p| p.container_port == port)?;
            (local_address(&published.host_ip), published.host_port)
        }
    };
    Some(DbTarget {
        engine,
        host: host.to_string(),
        container: c.name.clone(),
        container_id: c.id.clone(),
        address,
        port,
    })
}

/// Engine of a listening process, if it is a database server.
fn process_engine(process: &str) -> Option<Engine> {
    let p = process.to_ascii_lowercase();
    if p.contains("mysqld") || p.contains("mariadbd") {
        Some(Engine::MySql)
    } else if p.contains("postgres") || p.contains("postmaster") {
        Some(Engine::Postgres)
    } else {
        None
    }
}

/// Where the tunnel connects for a socket bound to `addr`.
fn local_address(addr: &str) -> String {
    let a = addr.trim().trim_start_matches('[').trim_end_matches(']');
    match a {
        "" | "0.0.0.0" | "*" | "::" | "127.0.0.1" | "localhost" => "127.0.0.1".into(),
        "::1" => "::1".into(),
        other => other.to_string(),
    }
}

/// Databases installed on the host itself: `mysqld` / `mariadbd` / `postgres`
/// in the listening ports, or the default port with no visible owner that no
/// container publishes (non-root users cannot see other users' processes).
pub fn host_targets(host: &str, report: &HostReport) -> Vec<DbTarget> {
    let published: Vec<u16> = report
        .docker
        .containers
        .iter()
        .flat_map(|c| c.published.iter().map(|p| p.host_port))
        .collect();
    let mut out: Vec<DbTarget> = Vec::new();
    let tcp = |l: &&Listener| l.proto.starts_with("tcp");
    for l in report.listeners.iter().filter(tcp) {
        let engine = match process_engine(&l.process) {
            Some(e) => e,
            None if l.process.trim().is_empty() && !published.contains(&l.port) => match l.port {
                3306 => Engine::MySql,
                5432 => Engine::Postgres,
                _ => continue,
            },
            None => continue,
        };
        if out.iter().any(|t| t.port == l.port && t.engine == engine) {
            continue; // the same port on IPv4 and IPv6
        }
        out.push(DbTarget {
            engine,
            host: host.to_string(),
            container: String::new(),
            container_id: String::new(),
            address: local_address(&l.address),
            port: l.port,
        });
    }
    out
}

/// Read-only script printing the database user names from a container's
/// environment (`MYSQL_USER=…`). Filtering happens on the host, so passwords
/// in the environment never leave it.
pub fn env_user_script(container_id: &str) -> Option<String> {
    if !crate::docker_actions::valid_id(container_id) {
        return None;
    }
    Some(format!(
        "LC_ALL=C; export LC_ALL\nD=''\nfor rt in docker podman; do\n  command -v \"$rt\" >/dev/null 2>&1 || continue\n  if \"$rt\" ps -q >/dev/null 2>&1; then D=\"$rt\"; break\n  elif sudo -n \"$rt\" ps -q >/dev/null 2>&1; then D=\"sudo -n $rt\"; break; fi\ndone\n[ -n \"$D\" ] || exit 0\n$D inspect --format '{{{{range .Config.Env}}}}{{{{println .}}}}{{{{end}}}}' {} 2>/dev/null | grep -E '^(MARIADB_USER|MYSQL_USER|POSTGRES_USER)='\nexit 0\n",
        crate::platform::shell_quote(container_id)
    ))
}

/// Database user name from a container's environment, over the normal
/// read-only SSH login; `None` when there is none or it cannot be read.
pub fn fetch_env_user(
    settings: &crate::collector::SshSettings,
    host: &crate::config::HostEntry,
    container_id: &str,
) -> Option<String> {
    let script = env_user_script(container_id)?;
    let (out, _, _) = crate::collector::run_script(settings, host, &script).ok()?;
    parse_env_user(&out)
}

/// User name from [`env_user_script`] output.
pub fn parse_env_user(out: &str) -> Option<String> {
    out.lines()
        .filter_map(|l| l.split_once('='))
        .filter(|(k, _)| matches!(*k, "MARIADB_USER" | "MYSQL_USER" | "POSTGRES_USER"))
        .map(|(_, v)| v.trim().to_string())
        .find(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PortMap;

    fn container(image: &str, ip: &str) -> Container {
        Container {
            id: "0123456789ab".into(),
            name: "shop-db".into(),
            image: image.into(),
            state: "running".into(),
            networks: if ip.is_empty() {
                vec![]
            } else {
                vec![("shop_default".into(), ip.into())]
            },
            ..Default::default()
        }
    }

    #[test]
    fn database_images() {
        for img in [
            "mariadb:11",
            "docker.io/library/mysql:8.4",
            "percona/percona-server:8.0",
            "registry.example/team/my-mariadb:latest",
        ] {
            assert_eq!(image_engine(img), Some(Engine::MySql), "{img}");
        }
        for img in [
            "postgres:17-alpine",
            "bitnami/postgresql",
            "postgis/postgis:16-3.4",
            "timescale/timescaledb:latest-pg16",
        ] {
            assert_eq!(image_engine(img), Some(Engine::Postgres), "{img}");
        }
        for img in [
            "prom/mysqld-exporter",
            "phpmyadmin:latest",
            "dpage/pgadmin4",
            "adminer",
            "edoburu/pgbouncer",
            "nginx:1.27",
            "sha256:0123456789abcdef",
        ] {
            assert_eq!(image_engine(img), None, "{img}");
        }
    }

    #[test]
    fn container_targets_use_the_container_ip_or_a_published_port() {
        let t = container_target("dock1", &container("mariadb:11", "172.18.0.5")).unwrap();
        assert_eq!((t.address.as_str(), t.port), ("172.18.0.5", 3306));
        assert_eq!(t.key(), "dock1 / container shop-db / mysql");
        assert_eq!(t.title(), "shop-db on dock1");

        let mut c = container("postgres:17", "");
        assert_eq!(
            container_target("dock1", &c),
            None,
            "no IP, nothing published"
        );
        c.published.push(PortMap {
            host_ip: "0.0.0.0".into(),
            host_port: 15432,
            container_port: 5432,
            proto: "tcp".into(),
        });
        let t = container_target("dock1", &c).unwrap();
        assert_eq!((t.address.as_str(), t.port), ("127.0.0.1", 15432));

        let mut stopped = container("mariadb", "172.18.0.5");
        stopped.state = "exited".into();
        assert_eq!(container_target("dock1", &stopped), None);
        assert_eq!(
            container_target("dock1", &container("nginx", "172.18.0.6")),
            None
        );
    }

    fn listener(addr: &str, port: u16, process: &str) -> Listener {
        Listener {
            proto: "tcp".into(),
            address: addr.into(),
            port,
            process: process.into(),
        }
    }

    #[test]
    fn host_databases_from_listening_ports() {
        let mut r = HostReport {
            listeners: vec![
                listener("127.0.0.1", 3306, "mariadbd"),
                listener("::1", 3306, "mariadbd"),
                listener("192.0.2.10", 5433, "postgres"),
                listener("0.0.0.0", 5432, ""),
                listener("0.0.0.0", 3307, "docker-proxy"),
                listener("0.0.0.0", 22, "sshd"),
            ],
            ..Default::default()
        };
        let t = host_targets("db1", &r);
        let got: Vec<(Engine, &str, u16)> = t
            .iter()
            .map(|t| (t.engine(), t.address.as_str(), t.port))
            .collect();
        assert_eq!(
            got,
            vec![
                (Engine::MySql, "127.0.0.1", 3306),
                (Engine::Postgres, "192.0.2.10", 5433),
                (Engine::Postgres, "127.0.0.1", 5432),
            ]
        );
        assert_eq!(t[0].key(), "db1 / port 3306 / mysql");

        // A default port published by a container is the container's, not the host's.
        r.listeners = vec![listener("0.0.0.0", 5432, "")];
        let mut c = container("postgres", "172.18.0.7");
        c.published.push(PortMap {
            host_ip: "0.0.0.0".into(),
            host_port: 5432,
            container_port: 5432,
            proto: "tcp".into(),
        });
        r.docker.containers.push(c);
        assert!(host_targets("db1", &r).is_empty());
    }

    #[test]
    fn env_user_is_filtered_on_the_host() {
        let s = env_user_script("0123456789ab").unwrap();
        assert!(s.contains("grep -E '^(MARIADB_USER|MYSQL_USER|POSTGRES_USER)='"));
        assert!(
            s.contains("{{range .Config.Env}}{{println .}}{{end}}"),
            "{s}"
        );
        assert!(env_user_script("x; rm -rf /").is_none());
        assert_eq!(parse_env_user("MYSQL_USER=shop\n"), Some("shop".into()));
        assert_eq!(parse_env_user("MYSQL_ROOT_PASSWORD=x\n"), None);
        assert_eq!(parse_env_user(""), None);
    }
}
