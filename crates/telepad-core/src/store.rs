//! SQLite host inventory (`telepad.db`). Single file, no server; editable in the
//! app or with any SQLite tool. Import/export: OpenSSH config text or JSON.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::config::{self, Access, HostEntry, Inventory};
use crate::update::UpdateConfig;

const SCHEMA_VERSION: i32 = 2;
const JSON_FORMAT: &str = "telepad/hosts";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub struct Store {
    conn: Connection,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// Add new aliases, update existing ones, keep the rest.
    Merge,
    /// Make the inventory exactly the imported file.
    Replace,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
}

/// Colour theme: follow the OS, or always light / dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }
}

/// View settings kept in the database so they survive restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPrefs {
    /// One line per host in the host list (default), or the detailed three-line rows.
    pub compact_host_list: bool,
    /// Host list grouped into Down / Up / Not checked yet.
    pub group_by_status: bool,
    /// Overview as cards instead of the table.
    pub overview_cards: bool,
    pub theme: ThemePref,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            compact_host_list: true,
            group_by_status: false,
            overview_cards: false,
            theme: ThemePref::System,
        }
    }
}

#[derive(Serialize)]
struct JsonFile {
    format: String,
    version: i32,
    #[serde(flatten)]
    inventory: Inventory,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS hosts (
                 id            INTEGER PRIMARY KEY,
                 alias         TEXT    NOT NULL UNIQUE,
                 hostname      TEXT    NOT NULL DEFAULT '',
                 user          TEXT    NOT NULL DEFAULT '',
                 port          INTEGER NOT NULL DEFAULT 22,
                 identity_file TEXT,
                 description   TEXT    NOT NULL DEFAULT '',
                 -- one 'Keyword value' per line
                 options       TEXT    NOT NULL DEFAULT '',
                 monitored     INTEGER NOT NULL DEFAULT 1,
                 access        TEXT    NOT NULL DEFAULT 'ssh',
                 sort_order    INTEGER NOT NULL DEFAULT 0,
                 updated_at    TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v == 1 {
            // v2: hosts.access ('ssh' | 'rdp')
            conn.execute_batch(
                "BEGIN;
                 ALTER TABLE hosts ADD COLUMN access TEXT NOT NULL DEFAULT 'ssh';
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?;
        } else if v == 0 {
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        } else if v > SCHEMA_VERSION {
            return Err(StoreError::Invalid(format!(
                "{} was written by a newer Telepad (schema {v})",
                path.display()
            )));
        }
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_empty(&self) -> Result<bool, StoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM hosts", [], |r| r.get(0))?;
        Ok(n == 0)
    }

    pub fn hosts(&self) -> Result<Vec<HostEntry>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT id, alias, hostname, user, port, identity_file, description, options, monitored,
                    access
             FROM hosts ORDER BY sort_order, id",
        )?;
        let rows = st.query_map([], |r| {
            let options: String = r.get(7)?;
            Ok(HostEntry {
                id: Some(r.get(0)?),
                alias: r.get(1)?,
                hostname: r.get(2)?,
                user: r.get(3)?,
                port: r.get::<_, i64>(4)?.clamp(1, 65535) as u16,
                identity_file: r.get(5)?,
                note: r.get(6)?,
                options: decode_options(&options),
                unmonitored: r.get::<_, i64>(8)? == 0,
                access: Access::from_db(&r.get::<_, String>(9)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn global(&self) -> Result<String, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'global_ssh'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default())
    }

    pub fn set_global(&self, text: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES('global_ssh', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![text],
        )?;
        Ok(())
    }

    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Self-update settings (`update_*` keys); missing keys use the defaults.
    pub fn update_config(&self) -> Result<UpdateConfig, StoreError> {
        let d = UpdateConfig::default();
        Ok(UpdateConfig {
            enabled: self
                .setting("update_enabled")?
                .map_or(d.enabled, |v| v != "0"),
            repo: self
                .setting("update_repo")?
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(d.repo),
            auto_install: self
                .setting("update_auto_install")?
                .map_or(d.auto_install, |v| v == "1"),
        })
    }

    pub fn set_update_config(&self, c: &UpdateConfig) -> Result<(), StoreError> {
        c.validate().map_err(StoreError::Invalid)?;
        let rows = [
            (
                "update_enabled",
                if c.enabled { "1" } else { "0" }.to_string(),
            ),
            ("update_repo", c.repo.clone()),
            (
                "update_auto_install",
                if c.auto_install { "1" } else { "0" }.to_string(),
            ),
        ];
        let tx = self.conn.unchecked_transaction()?;
        for (k, v) in rows {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remembered view settings (`ui_*` keys); missing keys use the defaults.
    pub fn ui_prefs(&self) -> Result<UiPrefs, StoreError> {
        let d = UiPrefs::default();
        let flag = |key: &str, default: bool| -> Result<bool, StoreError> {
            Ok(self.setting(key)?.map_or(default, |v| v != "0"))
        };
        Ok(UiPrefs {
            compact_host_list: flag("ui_compact_host_list", d.compact_host_list)?,
            group_by_status: flag("ui_group_by_status", d.group_by_status)?,
            overview_cards: flag("ui_overview_cards", d.overview_cards)?,
            theme: self
                .setting("ui_theme")?
                .map_or(d.theme, |v| ThemePref::parse(&v)),
        })
    }

    pub fn set_ui_prefs(&self, p: &UiPrefs) -> Result<(), StoreError> {
        let flag = |v: bool| if v { "1" } else { "0" };
        let rows = [
            ("ui_compact_host_list", flag(p.compact_host_list)),
            ("ui_group_by_status", flag(p.group_by_status)),
            ("ui_overview_cards", flag(p.overview_cards)),
            ("ui_theme", p.theme.as_str()),
        ];
        let tx = self.conn.unchecked_transaction()?;
        for (k, v) in rows {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn inventory(&self) -> Result<Inventory, StoreError> {
        Ok(Inventory {
            hosts: self.hosts()?,
            global: self.global()?,
        })
    }

    /// Insert (no id) or update (id) one host. Returns its id.
    pub fn save_host(&self, h: &HostEntry) -> Result<i64, StoreError> {
        h.validate().map_err(StoreError::Invalid)?;
        let clash: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM hosts WHERE alias = ?1",
                params![h.alias],
                |r| r.get(0),
            )
            .optional()?;
        if clash.is_some() && clash != h.id {
            return Err(StoreError::Invalid(format!(
                "Another host already uses the alias '{}'",
                h.alias
            )));
        }
        let opts = encode_options(&h.options);
        match h.id {
            Some(id) => {
                let n = self.conn.execute(
                    "UPDATE hosts SET alias=?2, hostname=?3, user=?4, port=?5, identity_file=?6,
                     description=?7, options=?8, monitored=?9, access=?10,
                     updated_at=CURRENT_TIMESTAMP
                     WHERE id=?1",
                    params![
                        id,
                        h.alias,
                        h.hostname,
                        h.user,
                        h.port,
                        h.identity_file,
                        h.note,
                        opts,
                        !h.unmonitored,
                        h.access.as_str()
                    ],
                )?;
                if n == 0 {
                    return Err(StoreError::Invalid(format!("host #{id} no longer exists")));
                }
                Ok(id)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO hosts(alias, hostname, user, port, identity_file, description,
                     options, monitored, access, sort_order)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                            (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM hosts))",
                    params![
                        h.alias,
                        h.hostname,
                        h.user,
                        h.port,
                        h.identity_file,
                        h.note,
                        opts,
                        !h.unmonitored,
                        h.access.as_str()
                    ],
                )?;
                Ok(self.conn.last_insert_rowid())
            }
        }
    }

    pub fn delete_host(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM hosts WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// What an import would do, without changing anything.
    pub fn preview_import(
        &self,
        inv: &Inventory,
        mode: ImportMode,
    ) -> Result<ImportSummary, StoreError> {
        let existing = self.hosts()?;
        let mut s = ImportSummary::default();
        for h in &inv.hosts {
            match existing.iter().find(|e| e.alias == h.alias) {
                None => s.added += 1,
                Some(e) if same_fields(e, h) => s.unchanged += 1,
                Some(_) => s.updated += 1,
            }
        }
        if mode == ImportMode::Replace {
            s.removed = existing
                .iter()
                .filter(|e| !inv.hosts.iter().any(|h| h.alias == e.alias))
                .count();
        }
        Ok(s)
    }

    /// Apply an import in one transaction: all of it or nothing.
    pub fn import(
        &mut self,
        inv: &Inventory,
        mode: ImportMode,
    ) -> Result<ImportSummary, StoreError> {
        for h in &inv.hosts {
            h.validate()
                .map_err(|e| StoreError::Invalid(format!("{}: {e}", h.alias)))?;
        }
        let summary = self.preview_import(inv, mode)?;
        let tx = self.conn.transaction()?;
        if mode == ImportMode::Replace {
            tx.execute("DELETE FROM hosts", [])?;
        }
        for (i, h) in inv.hosts.iter().enumerate() {
            tx.execute(
                "INSERT INTO hosts(alias, hostname, user, port, identity_file, description, options,
                 monitored, sort_order, access)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(alias) DO UPDATE SET hostname=excluded.hostname, user=excluded.user,
                   port=excluded.port, identity_file=excluded.identity_file,
                   description=excluded.description, options=excluded.options,
                   monitored=excluded.monitored, access=excluded.access,
                   updated_at=CURRENT_TIMESTAMP",
                params![
                    h.alias,
                    h.hostname,
                    h.user,
                    h.port,
                    h.identity_file,
                    h.note,
                    encode_options(&h.options),
                    !h.unmonitored,
                    i as i64 + 1,
                    h.access.as_str()
                ],
            )?;
        }
        if mode == ImportMode::Replace || !inv.global.trim().is_empty() {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES('global_ssh', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![inv.global],
            )?;
        }
        tx.commit()?;
        Ok(summary)
    }

    /// Render the file `ssh -F` reads. Written atomically so a running ssh never
    /// sees a half-written file.
    pub fn write_ssh_config(&self, path: &Path) -> Result<(), StoreError> {
        let text = config::to_ssh_config(&self.inventory()?);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

fn same_fields(a: &HostEntry, b: &HostEntry) -> bool {
    HostEntry {
        id: None,
        ..a.clone()
    } == HostEntry {
        id: None,
        ..b.clone()
    }
}

fn encode_options(o: &[(String, String)]) -> String {
    o.iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn decode_options(s: &str) -> Vec<(String, String)> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once([' ', '\t', '=']) {
            Some((k, v)) => (k.to_string(), v.trim().to_string()),
            None => (l.to_string(), String::new()),
        })
        .collect()
}

pub fn options_text(o: &[(String, String)]) -> String {
    encode_options(o)
}

/// Parse an import file. JSON is anything with a list of hosts: a Telepad
/// export of any version or app name, `{"hosts": [...]}`, or a bare `[...]`,
/// with common field names (`alias`/`name`/`host`, `hostname`/`ip`/`address`,
/// `user`/`username`, `identity_file`/`key`, `description`/`note`, ...).
/// Everything else is read as an OpenSSH client config (e.g. `~/.ssh/config`).
/// Hosts are validated later, before anything is saved.
pub fn parse_import(text: &str) -> Result<Inventory, StoreError> {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        parse_json_import(t)
    } else {
        Ok(config::parse_inventory(t))
    }
}

/// Hosts pasted as text. JSON and OpenSSH config text (anything with a `Host`
/// line) are read as by [`parse_import`]; otherwise one host per line,
/// `[alias] [user@]host[:port]`, with blank lines and `#` comments ignored.
/// Unreadable lines and invalid hosts are left out and described in the
/// returned list, so the valid ones can still be imported.
pub fn parse_pasted(text: &str) -> (Inventory, Vec<String>) {
    let t = text.trim_start_matches('\u{feff}').trim();
    let has_host_block = t.lines().any(|l| {
        let mut w = l.split_whitespace();
        w.next().is_some_and(|k| k.eq_ignore_ascii_case("host")) && w.next().is_some()
    });
    let (mut inv, mut problems) = if t.starts_with('{') || t.starts_with('[') || has_host_block {
        match parse_import(t) {
            Ok(inv) => (inv, Vec::new()),
            Err(e) => (Inventory::default(), vec![e.to_string()]),
        }
    } else {
        parse_host_lines(t)
    };
    inv.hosts.retain(|h| match h.validate() {
        Ok(()) => true,
        Err(e) => {
            problems.push(format!("{}: {e}", h.alias));
            false
        }
    });
    (inv, problems)
}

/// `[alias] [user@]host[:port]` per line.
fn parse_host_lines(t: &str) -> (Inventory, Vec<String>) {
    let mut inv = Inventory::default();
    let mut problems = Vec::new();
    for (n, line) in t.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let n = n + 1;
        let words: Vec<&str> = line.split_whitespace().collect();
        let (alias, target) = match words.as_slice() {
            [target] => (None, *target),
            [alias, target] => (Some(*alias), *target),
            _ => {
                problems.push(format!("line {n}: expected `[alias] [user@]host[:port]`"));
                continue;
            }
        };
        let (user, rest) = target.rsplit_once('@').unwrap_or(("", target));
        let (host, port) = match rest.rsplit_once(':') {
            // One colon only: `host:port` (an IPv6 address has several).
            Some((h, p)) if !h.contains(':') => match p.parse::<u16>() {
                Ok(p) if p > 0 => (h, p),
                _ => {
                    problems.push(format!("line {n}: port '{p}' is not 1-65535"));
                    continue;
                }
            },
            _ => (rest, 22),
        };
        if host.is_empty() {
            problems.push(format!("line {n}: no host name or address"));
            continue;
        }
        let alias = alias.unwrap_or(host);
        if inv.hosts.iter().any(|h| h.alias == alias) {
            problems.push(format!("line {n}: alias {alias} appears twice"));
            continue;
        }
        inv.hosts.push(HostEntry {
            hostname: host.to_string(),
            user: user.to_string(),
            port,
            ..HostEntry::new(alias)
        });
    }
    (inv, problems)
}

fn parse_json_import(t: &str) -> Result<Inventory, StoreError> {
    let v: serde_json::Value = serde_json::from_str(t)?;
    let (list, global) = match &v {
        serde_json::Value::Array(a) => (a, String::new()),
        serde_json::Value::Object(o) => {
            let hosts = field(o, &["hosts", "servers", "machines", "items"])
                .and_then(|h| h.as_array())
                .ok_or_else(|| StoreError::Invalid("no list of hosts in this JSON".into()))?;
            let global = field(o, &["global"])
                .and_then(|g| g.as_str())
                .unwrap_or_default()
                .to_string();
            (hosts, global)
        }
        _ => return Err(StoreError::Invalid("no list of hosts in this JSON".into())),
    };
    let hosts = list
        .iter()
        .enumerate()
        .map(|(i, h)| {
            host_from_json(h).ok_or_else(|| {
                StoreError::Invalid(format!("host #{}: needs at least a name or address", i + 1))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Inventory { hosts, global })
}

/// Look up a key ignoring case, `_` and `-` (`IdentityFile` = `identity_file`).
fn field<'a>(
    o: &'a serde_json::Map<String, serde_json::Value>,
    names: &[&str],
) -> Option<&'a serde_json::Value> {
    let norm = |k: &str| {
        k.chars()
            .filter(|c| *c != '_' && *c != '-')
            .collect::<String>()
            .to_ascii_lowercase()
    };
    names.iter().find_map(|n| {
        let n = norm(n);
        o.iter().find(|(k, _)| norm(k) == n).map(|(_, v)| v)
    })
}

fn host_from_json(v: &serde_json::Value) -> Option<HostEntry> {
    let o = v.as_object()?;
    let text = |names: &[&str]| -> Option<String> {
        field(o, names).and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    };
    let alias = text(&["alias", "name", "hostalias"]);
    let address = text(&[
        "hostname",
        "hostaddress",
        "ip",
        "ipaddress",
        "address",
        "addr",
    ]);
    // "host" is the alias in some exports and the address in others.
    let host = text(&["host"]);
    let (alias, hostname) = match (alias, address, host) {
        (Some(a), Some(h), _) => (a, h),
        (Some(a), None, h) => (a, h.unwrap_or_default()),
        (None, Some(h), Some(a)) => (a, h),
        (None, Some(h), None) => (h.clone(), h),
        (None, None, Some(h)) => (h.clone(), h),
        (None, None, None) => return None,
    };
    let access = text(&["access", "type", "protocol"])
        .map(|a| Access::from_db(&a))
        .unwrap_or_default();
    let port = text(&["port"])
        .and_then(|p| p.parse().ok())
        .filter(|p| *p != 0)
        .unwrap_or_else(|| access.default_port());
    let flag = |names: &[&str]| field(o, names).and_then(|v| v.as_bool());
    let unmonitored = flag(&["unmonitored", "disabled"])
        .or_else(|| flag(&["monitored", "enabled"]).map(|m| !m))
        .unwrap_or(false);
    let options = match field(o, &["options", "sshoptions"]) {
        // [["Keyword", "value"], ...] as Telepad exports them
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|kv| {
                let kv = kv.as_array()?;
                Some((
                    kv.first()?.as_str()?.to_string(),
                    kv.get(1)?.as_str()?.to_string(),
                ))
            })
            .collect(),
        // {"Keyword": "value", ...}
        Some(serde_json::Value::Object(m)) => m
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect(),
        _ => Vec::new(),
    };
    Some(HostEntry {
        id: None,
        alias,
        hostname,
        user: text(&["user", "username", "login"]).unwrap_or_default(),
        port,
        identity_file: text(&[
            "identityfile",
            "identity",
            "key",
            "keyfile",
            "privatekey",
            "sshkey",
        ]),
        note: text(&["description", "note", "comment", "desc"]).unwrap_or_default(),
        options,
        unmonitored,
        access,
    })
}

pub fn export_json(inv: &Inventory) -> Result<String, StoreError> {
    Ok(serde_json::to_string_pretty(&JsonFile {
        format: JSON_FORMAT.into(),
        version: SCHEMA_VERSION,
        inventory: inv.clone(),
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Store {
        Store::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn pasted_host_lists_and_ssh_config_text() {
        let (inv, problems) = parse_pasted(
            "# office\nweb01 admin@192.0.2.10\n198.51.100.7:2222\n\nnas 192.0.2.20 extra\n\
             db01 root@192.0.2.30:0\nweb01 192.0.2.11\nbad/alias 192.0.2.40\n",
        );
        let got: Vec<(&str, &str, &str, u16)> = inv
            .hosts
            .iter()
            .map(|h| {
                (
                    h.alias.as_str(),
                    h.hostname.as_str(),
                    h.user.as_str(),
                    h.port,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("web01", "192.0.2.10", "admin", 22),
                ("198.51.100.7", "198.51.100.7", "", 2222)
            ]
        );
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(problems[0].starts_with("line 5:"), "{problems:?}");
        assert!(problems[1].starts_with("line 6: port"), "{problems:?}");
        assert!(problems[2].contains("appears twice"), "{problems:?}");
        assert!(problems[3].starts_with("bad/alias:"), "{problems:?}");

        // OpenSSH config text goes through the config parser.
        let (inv, problems) =
            parse_pasted("Host web02\n  HostName 192.0.2.12\n  User deploy\n  Port 2200\n");
        assert!(problems.is_empty());
        assert_eq!(inv.hosts[0].alias, "web02");
        assert_eq!(
            (inv.hosts[0].user.as_str(), inv.hosts[0].port),
            ("deploy", 2200)
        );
    }

    fn sample() -> Inventory {
        config::parse_inventory(
            "# Mail\nHost mail\n  HostName 10.0.0.1\n  User root\n  KexAlgorithms +x\n\nHost web\n  HostName 10.0.0.2\n\nHost *\n  ServerAliveInterval 30\n",
        )
    }

    #[test]
    fn import_merge_replace_and_preview() {
        let mut s = mem();
        assert!(s.is_empty().unwrap());
        let r = s.import(&sample(), ImportMode::Merge).unwrap();
        assert_eq!(r.added, 2);
        let inv = s.inventory().unwrap();
        assert_eq!(inv.hosts[0].note, "Mail");
        assert_eq!(
            inv.hosts[0].options,
            vec![("KexAlgorithms".into(), "+x".into())]
        );
        assert_eq!(inv.global, "Host *\n  ServerAliveInterval 30");

        let mut next = sample();
        next.hosts[1].hostname = "10.0.0.20".into();
        next.hosts.truncate(2);
        next.hosts.push(HostEntry::new("db"));
        let p = s.preview_import(&next, ImportMode::Merge).unwrap();
        assert_eq!((p.added, p.updated, p.unchanged), (1, 1, 1));
        s.import(&next, ImportMode::Merge).unwrap();
        assert_eq!(s.hosts().unwrap().len(), 3);

        let only = Inventory {
            hosts: vec![HostEntry::new("db")],
            global: String::new(),
        };
        let r = s.import(&only, ImportMode::Replace).unwrap();
        assert_eq!(r.removed, 2);
        assert_eq!(s.hosts().unwrap().len(), 1);
        assert_eq!(s.global().unwrap(), "");
    }

    #[test]
    fn save_validates_and_rejects_duplicate_alias() {
        let s = mem();
        let id = s.save_host(&HostEntry::new("a")).unwrap();
        s.save_host(&HostEntry::new("b")).unwrap();
        let mut a = s.hosts().unwrap().remove(0);
        assert_eq!(a.id, Some(id));
        a.alias = "b".into();
        assert!(s.save_host(&a).is_err());
        a.alias = "a2".into();
        a.unmonitored = true;
        s.save_host(&a).unwrap();
        let got = s.hosts().unwrap();
        assert_eq!(got[0].alias, "a2");
        assert!(got[0].unmonitored);
        assert!(s.save_host(&HostEntry::new("has space")).is_err());
        s.delete_host(id).unwrap();
        assert_eq!(s.hosts().unwrap().len(), 1);
    }

    #[test]
    fn migrates_v1_database_and_keeps_rows() {
        let dir = std::env::temp_dir().join(format!("telepad-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v1.db");
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE hosts (id INTEGER PRIMARY KEY, alias TEXT NOT NULL UNIQUE,
                   hostname TEXT NOT NULL DEFAULT '', user TEXT NOT NULL DEFAULT '',
                   port INTEGER NOT NULL DEFAULT 22, identity_file TEXT,
                   description TEXT NOT NULL DEFAULT '', options TEXT NOT NULL DEFAULT '',
                   monitored INTEGER NOT NULL DEFAULT 1, sort_order INTEGER NOT NULL DEFAULT 0,
                   updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO hosts(alias, hostname) VALUES('old', '10.0.0.9');
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let h = s.hosts().unwrap();
        assert_eq!(h[0].alias, "old");
        assert_eq!(h[0].access, Access::Ssh);
        let mut w = HostEntry::new("win5");
        w.hostname = "198.51.100.5".into();
        w.access = Access::Rdp;
        w.port = 3389;
        s.save_host(&w).unwrap();
        assert_eq!(s.hosts().unwrap()[1].access, Access::Rdp);
        let v: i32 = s
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_settings_default_round_trip_and_validate() {
        let s = mem();
        assert_eq!(s.update_config().unwrap(), UpdateConfig::default());
        let c = UpdateConfig {
            enabled: false,
            repo: "someone/telepad-fork".into(),
            auto_install: true,
        };
        s.set_update_config(&c).unwrap();
        assert_eq!(s.update_config().unwrap(), c);
        let bad = UpdateConfig {
            repo: "../../evil".into(),
            ..UpdateConfig::default()
        };
        assert!(s.set_update_config(&bad).is_err());
        assert_eq!(
            s.update_config().unwrap(),
            c,
            "rejected save changes nothing"
        );
    }

    #[test]
    fn ui_prefs_default_and_round_trip() {
        let s = mem();
        assert_eq!(s.ui_prefs().unwrap(), UiPrefs::default());
        assert!(UiPrefs::default().compact_host_list);
        let p = UiPrefs {
            compact_host_list: false,
            group_by_status: true,
            overview_cards: true,
            theme: ThemePref::Light,
        };
        s.set_ui_prefs(&p).unwrap();
        assert_eq!(s.ui_prefs().unwrap(), p);
    }

    #[test]
    fn bad_import_changes_nothing() {
        let mut s = mem();
        s.import(&sample(), ImportMode::Merge).unwrap();
        let mut bad = sample();
        bad.hosts.push(HostEntry::new("bad alias"));
        assert!(s.import(&bad, ImportMode::Replace).is_err());
        assert_eq!(s.hosts().unwrap().len(), 2);
    }

    #[test]
    fn json_round_trip_and_format_check() {
        let inv = sample();
        let json = export_json(&inv).unwrap();
        assert_eq!(parse_import(&json).unwrap(), inv);
        assert!(parse_import("{\"format\":\"other\",\"version\":1}").is_err());
        assert_eq!(parse_import(&config::to_ssh_config(&inv)).unwrap(), inv);

        // Labels do not matter: older versions or the app's previous name import too.
        let relabelled = json
            .replace("\"telepad/hosts\"", "\"old-monitor/hosts\"")
            .replace(&format!("\"version\": {SCHEMA_VERSION}"), "\"version\": 99");
        assert_ne!(relabelled, json);
        assert_eq!(parse_import(&relabelled).unwrap(), inv);
    }

    #[test]
    fn imports_foreign_json_with_common_field_names() {
        let inv = parse_import(
            r#"[
              {"name": "web01", "ip": "192.0.2.10", "username": "admin", "Port": "2222",
               "IdentityFile": "~/.ssh/id_web", "comment": "Web server"},
              {"host": "192.0.2.20"},
              {"host": "db01", "address": "192.0.2.30", "monitored": false,
               "options": {"ProxyJump": "bastion"}},
              {"alias": "win01", "hostname": "192.0.2.40", "access": "RDP"}
            ]"#,
        )
        .unwrap();
        let h = &inv.hosts;
        assert_eq!(h.len(), 4);
        assert_eq!(
            (
                h[0].alias.as_str(),
                h[0].hostname.as_str(),
                h[0].user.as_str(),
                h[0].port
            ),
            ("web01", "192.0.2.10", "admin", 2222)
        );
        assert_eq!(h[0].identity_file.as_deref(), Some("~/.ssh/id_web"));
        assert_eq!(h[0].note, "Web server");
        assert_eq!(
            (h[1].alias.as_str(), h[1].hostname.as_str(), h[1].port),
            ("192.0.2.20", "192.0.2.20", 22)
        );
        assert!(h[2].unmonitored);
        assert_eq!(
            h[2].options,
            vec![("ProxyJump".to_string(), "bastion".to_string())]
        );
        assert_eq!((h[3].access, h[3].port), (Access::Rdp, 3389));
        assert!(h.iter().all(|h| h.validate().is_ok()));

        let wrapped = parse_import(r#"{"servers": [{"name": "a", "ip": "192.0.2.1"}]}"#).unwrap();
        assert_eq!(wrapped.hosts[0].alias, "a");
        assert!(parse_import(r#"{"something": 1}"#).is_err());
        assert!(
            parse_import(r#"[{"port": 22}]"#).is_err(),
            "no name or address"
        );
        assert!(parse_import("[1, 2]").is_err());
    }
}
