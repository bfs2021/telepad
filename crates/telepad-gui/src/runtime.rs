//! Executes [`Effect`]s and turns their results into [`Msg`]s. This is the
//! only GUI code that touches the database, ssh, the network, files, the
//! clipboard or the OS. Slow work (collection, update check/download) runs on
//! background threads; the rest is quick and runs inline.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use eframe::egui;
use telepad_app::db::{DbEffect, DbId, DbJob, DbMsg};
use telepad_app::{Effect, Msg, State};
use telepad_core::authorized_keys;
use telepad_core::collector::{self, Collector, SshSettings};
use telepad_core::db::client::{CancelSlot, Session};
use telepad_core::db::{detect, secret, tunnel};
use telepad_core::store;
use telepad_core::update::{self, Updater, relaunch};
use telepad_core::{Workspace, config, open_workspace, platform};

const WORKERS: usize = 12;

pub struct Runtime {
    ctx: egui::Context,
    ws: Option<Workspace>,
    ssh: Option<SshSettings>,
    collector: Option<(Collector, Receiver<collector::Update>)>,
    /// Results of background threads other than the collector.
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// One worker thread per database window; dropping it closes the connection.
    db: HashMap<DbId, DbWorker>,
}

struct DbWorker {
    jobs: Sender<DbJob>,
    cancel: CancelSlot,
}

impl Runtime {
    /// Open the database and start the collector; reports through `state`.
    pub fn start(
        ctx: egui::Context,
        state: &mut State,
        cli_config: Option<&str>,
        cli_db: Option<&str>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut rt = Self {
            ctx,
            ws: None,
            ssh: None,
            collector: None,
            tx,
            rx,
            db: HashMap::new(),
        };
        match open_workspace(cli_config, cli_db) {
            Ok(ws) => {
                let ssh = ws.settings();
                let repaint = rt.ctx.clone();
                rt.collector = Some(Collector::start(
                    ssh.clone(),
                    WORKERS,
                    Arc::new(move || repaint.request_repaint()),
                ));
                let started = Msg::Started {
                    db_path: ws.store.path().to_path_buf(),
                    ssh_config_path: ws.ssh_config.clone(),
                    imported_from: ws.imported_from.clone(),
                    update_config: ws.store.update_config().map_err(|e| e.to_string()),
                    ui_prefs: ws.store.ui_prefs().unwrap_or_default(),
                };
                rt.ssh = Some(ssh);
                rt.ws = Some(ws);
                rt.dispatch(state, started);
                if let Ok(alias) = std::env::var("TELEPAD_SELECT") {
                    rt.dispatch(state, Msg::SelectAlias(alias));
                }
            }
            Err(e) => rt.dispatch(state, Msg::Fatal(e)),
        }
        rt
    }

    /// Feed background results and the frame tick into `update`.
    pub fn frame(&mut self, state: &mut State) {
        let mut incoming = Vec::new();
        if let Some((_, rx)) = &self.collector {
            incoming.extend(rx.try_iter().map(Msg::Collected));
        }
        incoming.extend(self.rx.try_iter());
        for msg in incoming {
            self.dispatch(state, msg);
        }
        let busy = self.collector.as_ref().map_or(0, |(c, _)| c.busy());
        self.dispatch(state, Msg::Tick { busy });
    }

    /// Run `msg` through `update`, then its effects, then the messages those
    /// produce, until nothing is left.
    pub fn dispatch(&mut self, state: &mut State, msg: Msg) {
        let mut queue = VecDeque::from([msg]);
        while let Some(msg) = queue.pop_front() {
            for effect in telepad_app::update(state, msg, Instant::now()) {
                if let Some(next) = self.run(effect) {
                    queue.push_back(next);
                }
            }
        }
    }

    fn run(&mut self, effect: Effect) -> Option<Msg> {
        match effect {
            Effect::LoadHosts => {
                let ws = self.ws.as_ref()?;
                let ssh_config_error = ws
                    .sync_ssh_config()
                    .err()
                    .map(|e| format!("Cannot write {}: {e}", ws.ssh_config.display()));
                Some(Msg::HostsLoaded {
                    hosts: ws.store.hosts().map_err(|e| e.to_string()),
                    ssh_config_error,
                })
            }
            Effect::Collect(hosts) => {
                if let Some((c, _)) = &self.collector {
                    c.enqueue(hosts);
                }
                None
            }
            Effect::SaveHost(entry) => {
                let ws = self.ws.as_ref()?;
                Some(Msg::HostSaved(
                    ws.store
                        .save_host(&entry)
                        .map(|id| (id, entry.alias.clone()))
                        .map_err(|e| e.to_string()),
                ))
            }
            Effect::DeleteHost { id, alias } => {
                let ws = self.ws.as_ref()?;
                Some(Msg::HostDeleted {
                    id,
                    alias,
                    result: ws.store.delete_host(id).map_err(|e| e.to_string()),
                })
            }
            Effect::PickImportFile => {
                let path = rfd::FileDialog::new()
                    .set_title("Import hosts (OpenSSH config or JSON host list)")
                    .add_filter("All files", &["*"])
                    .add_filter("JSON", &["json"])
                    .pick_file()?;
                Some(read_import(path))
            }
            Effect::SaveOrder(ids) => {
                let ws = self.ws.as_mut()?;
                let saved = ws.store.save_order(&ids).map_err(|e| e.to_string());
                Some(Msg::OrderSaved(saved.and_then(|()| ws.sync_ssh_config())))
            }
            Effect::ReadClipboard => Some(Msg::ClipboardRead(
                arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .map_err(|e| e.to_string()),
            )),
            Effect::ReadUserSshConfig => {
                Some(read_import(telepad_core::expand_home("~/.ssh/config")))
            }
            Effect::PreviewImport { inventory, mode } => Some(Msg::ImportPreviewed(
                self.with_store(|s| s.preview_import(&inventory, mode)),
            )),
            Effect::ApplyImport { inventory, mode } => {
                let ws = self.ws.as_mut()?;
                Some(Msg::Imported(
                    ws.store.import(&inventory, mode).map_err(|e| e.to_string()),
                ))
            }
            Effect::PickExportFile => {
                let ws = self.ws.as_ref()?;
                let path = rfd::FileDialog::new()
                    .set_title("Export hosts")
                    .set_file_name("telepad-hosts.json")
                    .add_filter("JSON (re-importable, keeps everything)", &["json"])
                    .add_filter("OpenSSH config", &["config", "conf", "txt"])
                    .save_file()?;
                Some(Msg::Exported(export(ws, &path).map(|n| (n, path))))
            }
            Effect::PickKeyFile(current) => {
                // Start where keys live: ~/.ssh is hidden on macOS/Linux, so a
                // dialog opened in the home folder would not show it.
                let current = telepad_core::expand_home(current.trim());
                let start = current
                    .parent()
                    .filter(|d| !current.as_os_str().is_empty() && d.is_dir())
                    .map(Path::to_path_buf)
                    .or_else(|| Some(telepad_core::expand_home("~/.ssh")).filter(|d| d.is_dir()));
                let mut dialog = rfd::FileDialog::new().set_title("Private key");
                if let Some(dir) = start {
                    dialog = dialog.set_directory(dir);
                }
                dialog.pick_file().map(Msg::KeyPicked)
            }
            Effect::FindKeys => Some(Msg::KeysFound(telepad_core::find_ssh_keys())),
            Effect::OpenSsh(h) => {
                let s = self.ssh.as_ref()?;
                let text = match platform::open_terminal(&s.ssh_program, &s.args(&h, false)) {
                    Ok(()) => format!("Opened SSH session to {}", h.alias),
                    Err(e) => e,
                };
                Some(Msg::Notice(text))
            }
            Effect::OpenRdp(h) => {
                let host = if h.hostname.is_empty() {
                    &h.alias
                } else {
                    &h.hostname
                };
                let text = match platform::open_rdp(host, h.port) {
                    Ok(()) => format!("Opened Remote Desktop to {}", h.alias),
                    Err(e) => e,
                };
                Some(Msg::Notice(text))
            }
            Effect::OpenBmcSetup(h) => {
                let s = self.ssh.as_ref()?;
                let args = collector::bmc_setup_args(s, &h);
                let text = match platform::open_terminal(&s.ssh_program, &args) {
                    Ok(()) => format!(
                        "Opened BMC setup for {}: press Enter there, type your sudo password, then Refresh",
                        h.alias
                    ),
                    Err(e) => e,
                };
                Some(Msg::Notice(text))
            }
            Effect::OpenDockerAction(h, plan) => {
                let s = self.ssh.as_ref()?;
                let args = collector::docker_action_args(s, &h, &plan);
                let text = match platform::open_terminal(&s.ssh_program, &args) {
                    Ok(()) => format!(
                        "Opened a terminal on {}: {} — runs after you confirm there",
                        h.alias,
                        plan.display()
                    ),
                    Err(e) => e,
                };
                Some(Msg::Notice(text))
            }
            Effect::FetchRunInfo(h, c) => {
                let s = self.ssh.clone()?;
                self.background(move || Msg::RunInfoLoaded(collector::fetch_run_info(&s, &h, &c)));
                None
            }
            Effect::CopySshCommand(h) => {
                let s = self.ssh.as_ref()?;
                self.ctx
                    .copy_text(platform::command_line(&s.ssh_program, &s.args(&h, false)));
                Some(Msg::Notice("SSH command copied to clipboard".into()))
            }
            Effect::CheckUpdate(cfg) => {
                self.background(move || Msg::UpdateChecked(Updater::new(cfg).check()));
                None
            }
            Effect::InstallUpdate(check) => {
                self.background(move || Msg::UpdateInstalled(update::install(&check)));
                None
            }
            Effect::SaveUpdateConfig(cfg) => Some(Msg::UpdateSettingsSaved(
                self.with_store(|s| s.set_update_config(&cfg)).map(|()| cfg),
            )),
            Effect::SaveUiPrefs(p) => {
                Some(Msg::UiPrefsSaved(self.with_store(|s| s.set_ui_prefs(&p))))
            }
            Effect::ApplyKey { action, key, hosts } => {
                let s = self.ssh.clone()?;
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    authorized_keys::apply_all(
                        &s,
                        &hosts,
                        action,
                        &key,
                        WORKERS,
                        |alias, outcome| {
                            let _ = tx.send(Msg::KeyResult { alias, outcome });
                            ctx.request_repaint();
                        },
                    );
                });
                None
            }
            Effect::Db(e) => self.run_db(e),
            Effect::OpenUrl(url) => platform::open_url(&url).err().map(Msg::Notice),
            Effect::Relaunch => match relaunch() {
                Ok(()) => {
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    None
                }
                Err(e) => Some(Msg::Notice(e)),
            },
        }
    }

    /// Close every database connection and its SSH tunnel (app exit).
    pub fn shutdown(&mut self) {
        for (_, w) in self.db.drain() {
            cancel(&w.cancel);
        }
        tunnel::close_all();
    }

    fn run_db(&mut self, e: DbEffect) -> Option<Msg> {
        match e {
            DbEffect::LoadProfile { id, target, host } => {
                let key = target.key();
                let profile = self
                    .ws
                    .as_ref()
                    .and_then(|ws| ws.store.db_profile(&key).ok().flatten());
                let want = profile.as_ref().is_some_and(|p| p.save_password);
                self.background(move || {
                    let store = secret::available();
                    let saved_password =
                        want && store.is_ok() && matches!(secret::load(&key), Ok(Some(_)));
                    Msg::Db(DbMsg::Loaded {
                        id,
                        profile,
                        store,
                        saved_password,
                    })
                });
                if !target.container_id.is_empty()
                    && let Some(s) = self.ssh.clone()
                {
                    self.background(move || {
                        Msg::Db(DbMsg::EnvUser {
                            id,
                            user: detect::fetch_env_user(&s, &host, &target.container_id),
                        })
                    });
                }
                None
            }
            DbEffect::Connect {
                id,
                target,
                host,
                mut login,
                use_saved_password,
                profile,
            } => {
                let ssh = self.ssh.clone()?;
                let (jobs, inbox) = mpsc::channel::<DbJob>();
                let slot: CancelSlot = Arc::new(Mutex::new(None));
                if let Some(old) = self.db.insert(
                    id,
                    DbWorker {
                        jobs,
                        cancel: slot.clone(),
                    },
                ) {
                    cancel(&old.cancel);
                }
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    let send = |m: Msg| {
                        let _ = tx.send(m);
                        ctx.request_repaint();
                    };
                    if use_saved_password {
                        match secret::load(&profile.key) {
                            Ok(Some(p)) => login.password = p,
                            Ok(None) => {}
                            Err(e) => {
                                send(Msg::Db(DbMsg::Connected {
                                    id,
                                    result: Err(format!("Cannot read the saved password: {e}")),
                                }));
                                return;
                            }
                        }
                    }
                    let password = login.password.clone();
                    let (session, info) =
                        match Session::connect(&ssh, &host, &target, login, profile.read_only) {
                            Ok(v) => v,
                            Err(e) => {
                                send(Msg::Db(DbMsg::Connected { id, result: Err(e) }));
                                return;
                            }
                        };
                    if profile.save_password {
                        if !password.is_empty()
                            && let Err(e) = secret::save(&profile.key, &password)
                        {
                            send(Msg::Notice(format!("Password not saved: {e}")));
                        }
                    } else {
                        let _ = secret::forget(&profile.key);
                    }
                    send(Msg::Db(DbMsg::Connected {
                        id,
                        result: Ok(info),
                    }));
                    let mut session = session.with_cancel_slot(slot);
                    for job in inbox {
                        send(Msg::Db(db_job(&mut session, id, job)));
                    }
                });
                None
            }
            DbEffect::Job { id, job } => match self.db.get(&id) {
                Some(w) => match w.jobs.send(job) {
                    Ok(()) => None,
                    Err(mpsc::SendError(job)) => Some(Msg::Db(db_failed(id, job))),
                },
                None => Some(Msg::Db(db_failed(id, job))),
            },
            DbEffect::Cancel(id) => {
                if let Some(w) = self.db.get(&id) {
                    let slot = w.cancel.clone();
                    thread::spawn(move || cancel(&slot));
                }
                None
            }
            DbEffect::Close(id) => {
                if let Some(w) = self.db.remove(&id) {
                    let slot = w.cancel;
                    thread::spawn(move || cancel(&slot));
                }
                None
            }
            DbEffect::SaveProfile(p) => self
                .with_store(|s| s.save_db_profile(&p))
                .err()
                .map(|e| Msg::Notice(format!("Cannot save the database connection: {e}"))),
            DbEffect::Forget { id, key } => {
                let removed = self.with_store(|s| s.delete_db_profile(&key));
                self.background(move || {
                    let result = removed.and_then(|()| match secret::available() {
                        Ok(()) => secret::forget(&key),
                        Err(_) => Ok(()),
                    });
                    Msg::Db(DbMsg::Forgotten { id, result })
                });
                None
            }
        }
    }

    fn with_store<T>(
        &self,
        f: impl FnOnce(&store::Store) -> Result<T, store::StoreError>,
    ) -> Result<T, String> {
        match &self.ws {
            Some(ws) => f(&ws.store).map_err(|e| e.to_string()),
            None => Err("no database".into()),
        }
    }

    fn background(&self, job: impl FnOnce() -> Msg + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }
}

/// Stop whatever statement the connection is running (no-op when idle).
fn cancel(slot: &CancelSlot) {
    if let Ok(guard) = slot.lock()
        && let Some(c) = guard.as_ref()
    {
        let _ = c.cancel();
    }
}

/// Run one database job on the worker thread and report it.
fn db_job(s: &mut Session, id: DbId, job: DbJob) -> DbMsg {
    match job {
        DbJob::Databases => DbMsg::DatabasesLoaded {
            id,
            result: s.databases(),
        },
        DbJob::Schemas { database } => DbMsg::SchemasLoaded {
            id,
            result: s.schemas(&database),
            database,
        },
        DbJob::Tables { database, schema } => DbMsg::TablesLoaded {
            id,
            result: s.tables(&database, &schema),
            database,
            schema,
        },
        DbJob::Info(table) => DbMsg::InfoLoaded {
            id,
            result: s.table_info(&table),
            table,
        },
        DbJob::Page {
            table,
            filter,
            sort,
            offset,
            limit,
        } => DbMsg::PageLoaded {
            id,
            result: s.page(&table, &filter, &sort, offset, limit),
            table,
            offset,
        },
        DbJob::Run {
            tab,
            database,
            schema,
            statements,
        } => DbMsg::QueryDone {
            id,
            tab,
            results: s.run(&database, &schema, &statements),
        },
        DbJob::Apply {
            database,
            statements,
        } => DbMsg::Posted {
            id,
            result: s.apply(&database, &statements),
        },
        DbJob::Ddl {
            database,
            statements,
            after,
        } => DbMsg::DdlDone {
            id,
            result: s.ddl(&database, &statements),
            after,
        },
        DbJob::ReadOnly(on) => DbMsg::ReadOnlySet {
            id,
            on,
            result: s.set_read_only(on),
        },
    }
}

/// The answer to a job when the connection is gone.
fn db_failed(id: DbId, job: DbJob) -> DbMsg {
    const GONE: &str = "Not connected — click Reconnect";
    match job {
        DbJob::Databases => DbMsg::DatabasesLoaded {
            id,
            result: Err(GONE.into()),
        },
        DbJob::Schemas { database } => DbMsg::SchemasLoaded {
            id,
            database,
            result: Err(GONE.into()),
        },
        DbJob::Tables { database, schema } => DbMsg::TablesLoaded {
            id,
            database,
            schema,
            result: Err(GONE.into()),
        },
        DbJob::Info(table) => DbMsg::InfoLoaded {
            id,
            table,
            result: Err(GONE.into()),
        },
        DbJob::Page { table, offset, .. } => DbMsg::PageLoaded {
            id,
            table,
            offset,
            result: Err(GONE.into()),
        },
        DbJob::Run { tab, .. } => DbMsg::QueryDone {
            id,
            tab,
            results: vec![telepad_core::db::ResultSet {
                error: Some(GONE.into()),
                ..Default::default()
            }],
        },
        DbJob::Apply { .. } => DbMsg::Posted {
            id,
            result: Err(GONE.into()),
        },
        DbJob::Ddl { after, .. } => DbMsg::DdlDone {
            id,
            result: Err(GONE.into()),
            after,
        },
        DbJob::ReadOnly(on) => DbMsg::ReadOnlySet {
            id,
            on,
            result: Err(GONE.into()),
        },
    }
}

/// Read and parse an import file; errors keep the path for the message.
fn read_import(path: std::path::PathBuf) -> Msg {
    let parsed = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| store::parse_import(&t).map_err(|e| e.to_string()));
    Msg::ImportOpened(match parsed {
        Ok(inv) => Ok((path, inv)),
        Err(e) => Err((path, e)),
    })
}

fn export(ws: &Workspace, path: &Path) -> Result<usize, String> {
    let inv = ws.store.inventory().map_err(|e| e.to_string())?;
    let json = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let text = if json {
        store::export_json(&inv).map_err(|e| e.to_string())?
    } else {
        config::to_ssh_config(&inv)
    };
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    Ok(inv.hosts.len())
}
