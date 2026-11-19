//! File windows (#62): one per host, the local computer on the left and the
//! host on the right, with a transfer queue. Pure state and update logic;
//! the runtime does the listing and copying.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::files::transfer::{Decision, Direction, Event, Item, Source};
use telepad_core::files::{Entry, local_join, local_parent, sftp};

pub type FmId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    Local,
    Remote,
}

/// How the host is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Sftp,
    /// No SFTP server: `sh` listing and `scp -O` copies.
    Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortCol {
    #[default]
    Name,
    Size,
    Modified,
    Permissions,
    Owner,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pane {
    /// Absolute folder shown.
    pub path: String,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub error: Option<String>,
    pub sort: SortCol,
    pub descending: bool,
}

impl Pane {
    /// Entries in display order: folders first, then the chosen sort.
    pub fn sorted(&self, show_hidden: bool, filter: &str) -> Vec<&Entry> {
        let f = filter.trim().to_lowercase();
        let mut v: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| show_hidden || !e.hidden)
            .filter(|e| f.is_empty() || e.name.to_lowercase().contains(&f))
            .collect();
        v.sort_by(|a, b| {
            let by = match self.sort {
                SortCol::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortCol::Size => a.size.cmp(&b.size),
                SortCol::Modified => a.mtime.cmp(&b.mtime),
                SortCol::Permissions => a.perm.cmp(&b.perm),
                SortCol::Owner => a.owner.cmp(&b.owner),
            };
            let by = if self.descending { by.reverse() } else { by };
            b.is_dir().cmp(&a.is_dir()).then(by)
        });
        v
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TState {
    /// Folders being expanded, existing files looked up.
    Planning,
    /// Waiting for the user's answers about existing files.
    Asking,
    /// rsync dry run done, waiting for Run.
    Previewed(Vec<String>),
    Running,
    Done {
        copied: usize,
        skipped: usize,
        failed: usize,
    },
    Failed(String),
    Cancelled,
}

impl TState {
    pub fn finished(&self) -> bool {
        matches!(
            self,
            TState::Done { .. } | TState::Failed(_) | TState::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transfer {
    pub id: u64,
    pub direction: Direction,
    /// "3 items → /srv/www".
    pub title: String,
    pub sources: Vec<Source>,
    pub dest: String,
    pub rsync: bool,
    pub state: TState,
    pub items: Vec<Item>,
    pub decisions: HashMap<usize, Decision>,
    pub current: String,
    pub done_items: usize,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub started: Option<Instant>,
    pub errors: Vec<String>,
}

impl Transfer {
    /// Bytes per second since the copy started.
    pub fn speed(&self, now: Instant) -> Option<f64> {
        let t = now.duration_since(self.started?).as_secs_f64();
        (t > 0.5).then(|| self.bytes_done as f64 / t)
    }
}

/// The overwrite question for one existing file.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub transfer: u64,
    /// Item indexes of existing files, in order.
    pub conflicts: Vec<usize>,
    /// Position in `conflicts` being asked.
    pub at: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Connecting,
    Ready,
}

#[derive(Debug, Clone, Default)]
pub struct FileSession {
    pub id: FmId,
    pub host: HostEntry,
    pub status: Status,
    pub mode: Mode,
    /// Connection error (window shows it with Retry).
    pub error: Option<String>,
    pub local: Pane,
    pub remote: Pane,
    pub remote_home: String,
    pub transfers: Vec<Transfer>,
    pub ask: Option<Ask>,
    /// rsync usable on both sides (else why not); `None` while checking.
    pub rsync: Option<Result<(), String>>,
    next_transfer: u64,
}

impl FileSession {
    pub fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        }
    }
    fn pane_mut(&mut self, side: Side) -> &mut Pane {
        match side {
            Side::Local => &mut self.local,
            Side::Remote => &mut self.remote,
        }
    }
    pub fn transfer(&self, id: u64) -> Option<&Transfer> {
        self.transfers.iter().find(|t| t.id == id)
    }
    fn transfer_mut(&mut self, id: u64) -> Option<&mut Transfer> {
        self.transfers.iter_mut().find(|t| t.id == id)
    }
}

/// Text the window binds to.
#[derive(Debug, Clone, Default)]
pub struct FileDraft {
    pub local_path: String,
    pub remote_path: String,
    pub local_filter: String,
    pub remote_filter: String,
    pub selected_local: BTreeSet<String>,
    pub selected_remote: BTreeSet<String>,
    /// Pane that was clicked last (F5 copies from it).
    pub active: Option<Side>,
    pub show_hidden: bool,
    pub use_rsync: bool,
    /// "Apply to all" in the overwrite question.
    pub apply_all: bool,
}

impl FileDraft {
    pub fn selected(&self, side: Side) -> &BTreeSet<String> {
        match side {
            Side::Local => &self.selected_local,
            Side::Remote => &self.selected_remote,
        }
    }
    pub fn selected_mut(&mut self, side: Side) -> &mut BTreeSet<String> {
        match side {
            Side::Local => &mut self.selected_local,
            Side::Remote => &mut self.selected_remote,
        }
    }
    fn path_mut(&mut self, side: Side) -> &mut String {
        match side {
            Side::Local => &mut self.local_path,
            Side::Remote => &mut self.remote_path,
        }
    }
}

/// How the host was reached: mode, remote home, remembered (local, remote) folders.
pub type Connection = (Mode, String, Option<(String, String)>);

#[derive(Debug, Clone)]
pub enum FmMsg {
    /// "Open files…" of a host.
    Open(String),
    Connected {
        id: FmId,
        /// (mode, remote home, remembered (local, remote) folders).
        result: Result<Connection, String>,
        local_home: String,
    },
    Retry(FmId),
    Close(FmId),
    /// Go to a folder (absolute, or typed into the path bar).
    Navigate {
        id: FmId,
        side: Side,
        path: String,
    },
    /// Enter a folder of the list.
    Enter {
        id: FmId,
        side: Side,
        name: String,
    },
    Up {
        id: FmId,
        side: Side,
    },
    Home {
        id: FmId,
        side: Side,
    },
    Refresh {
        id: FmId,
        side: Side,
    },
    Listed {
        id: FmId,
        side: Side,
        result: Result<(String, Vec<Entry>), String>,
    },
    Sort {
        id: FmId,
        side: Side,
        col: SortCol,
    },
    /// Copy these names of `from` into the other pane's folder.
    Copy {
        id: FmId,
        from: Side,
        names: Vec<String>,
    },
    Planned {
        id: FmId,
        transfer: u64,
        result: Result<Vec<Item>, String>,
    },
    /// Answer to the overwrite question (Apply to all from the draft).
    Decide {
        id: FmId,
        decision: Decision,
    },
    /// Stop asking: cancel the whole transfer.
    AskCancel(FmId),
    Progress {
        id: FmId,
        transfer: u64,
        event: Event,
    },
    Finished {
        id: FmId,
        transfer: u64,
        result: Result<(usize, usize, usize), String>,
    },
    Cancel {
        id: FmId,
        transfer: u64,
    },
    ClearFinished(FmId),
    RsyncChecked {
        id: FmId,
        result: Result<(), String>,
    },
    RsyncPreviewed {
        id: FmId,
        transfer: u64,
        result: Result<Vec<String>, String>,
    },
    /// Run an rsync transfer after its preview.
    RsyncRun {
        id: FmId,
        transfer: u64,
    },
}

#[derive(Debug, Clone)]
pub enum FmEffect {
    /// SFTP (else the `sh` fallback), home folder, remembered folders, local
    /// home → [`FmMsg::Connected`]; also checks rsync → [`FmMsg::RsyncChecked`].
    Connect {
        id: FmId,
        host: HostEntry,
    },
    /// → [`FmMsg::Listed`]
    List {
        id: FmId,
        side: Side,
        path: String,
    },
    /// Expand folders, find existing files → [`FmMsg::Planned`].
    Plan {
        id: FmId,
        transfer: u64,
        direction: Direction,
        sources: Vec<Source>,
        dest: String,
    },
    /// Copy over SFTP → [`FmMsg::Progress`]…, [`FmMsg::Finished`].
    Run {
        id: FmId,
        transfer: u64,
        direction: Direction,
        items: Vec<Item>,
        decisions: HashMap<usize, Decision>,
    },
    /// Fallback mode: `scp -O` each source → [`FmMsg::Finished`].
    Scp {
        id: FmId,
        transfer: u64,
        direction: Direction,
        sources: Vec<Source>,
        dest: String,
    },
    /// rsync (dry run → [`FmMsg::RsyncPreviewed`], else → [`FmMsg::Finished`]).
    Rsync {
        id: FmId,
        transfer: u64,
        direction: Direction,
        sources: Vec<String>,
        dest: String,
        dry_run: bool,
    },
    Cancel {
        id: FmId,
        transfer: u64,
    },
    /// Close both connections.
    Close(FmId),
    SavePaths {
        alias: String,
        local: String,
        remote: String,
    },
}

fn list(fx: &mut Vec<FmEffect>, s: &mut FileSession, d: &mut FileDraft, side: Side, path: String) {
    let p = s.pane_mut(side);
    p.loading = true;
    p.error = None;
    *d.path_mut(side) = path.clone();
    fx.push(FmEffect::List {
        id: s.id,
        side,
        path,
    });
}

fn join(side: Side, dir: &str, name: &str) -> String {
    match side {
        Side::Local => local_join(dir, name),
        Side::Remote => sftp::join(dir, name),
    }
}

fn parent(side: Side, dir: &str) -> Option<String> {
    match side {
        Side::Local => local_parent(dir),
        Side::Remote => (dir != "/").then(|| sftp::parent(dir)),
    }
}

/// Fallback mode copies whole sources with scp; the existing check is by
/// name in the target pane.
fn fallback_items(s: &FileSession, from: Side, names: &[String]) -> Vec<Item> {
    let (src, dst) = (s.pane(from), s.pane(other(from)));
    names
        .iter()
        .filter_map(|n| src.entries.iter().find(|e| &e.name == n))
        .map(|e| Item {
            src: join(from, &src.path, &e.name),
            dst: join(other(from), &dst.path, &e.name),
            label: e.name.clone(),
            is_dir: e.is_dir(),
            size: e.size,
            mtime: e.mtime,
            perm: e.perm,
            exists: dst
                .entries
                .iter()
                .find(|x| x.name == e.name)
                .map(|x| (x.size, x.mtime)),
        })
        .collect()
}

/// Last part of a folder path for titles (`…/www`); the full path is the transfer's `dest`.
pub fn short_dir(path: &str) -> String {
    let p = path.trim_end_matches(['/', '\\']);
    match p.rfind(['/', '\\']) {
        Some(i) if i + 1 < p.len() && p.len() > 24 => format!("…{}", &p[i..]),
        _ => path.to_string(),
    }
}

pub fn other(side: Side) -> Side {
    match side {
        Side::Local => Side::Remote,
        Side::Remote => Side::Local,
    }
}

/// Ask about the next existing file, or start the copy.
fn next_or_run(s: &mut FileSession, transfer: u64, fx: &mut Vec<FmEffect>, now: Instant) {
    let id = s.id;
    let mode = s.mode;
    if let Some(a) = &s.ask
        && a.transfer == transfer
        && a.at < a.conflicts.len()
    {
        return;
    }
    s.ask = None;
    let Some(t) = s.transfer_mut(transfer) else {
        return;
    };
    t.state = TState::Running;
    t.started = Some(now);
    t.bytes_total = t
        .items
        .iter()
        .enumerate()
        .filter(|(i, it)| !it.is_dir && it.should_copy(t.decisions.get(i).copied()))
        .map(|(_, it)| it.size)
        .sum();
    match mode {
        Mode::Sftp => fx.push(FmEffect::Run {
            id,
            transfer,
            direction: t.direction,
            items: t.items.clone(),
            decisions: t.decisions.clone(),
        }),
        Mode::Fallback => {
            let sources: Vec<Source> = t
                .items
                .iter()
                .enumerate()
                .filter(|(i, it)| it.should_copy(t.decisions.get(i).copied()))
                .map(|(_, it)| (it.src.clone(), it.is_dir))
                .collect();
            fx.push(FmEffect::Scp {
                id,
                transfer,
                direction: t.direction,
                sources,
                dest: t.dest.clone(),
            });
        }
    }
}

/// Items planned: ask about existing files first, if any.
fn planned(
    s: &mut FileSession,
    transfer: u64,
    items: Vec<Item>,
    fx: &mut Vec<FmEffect>,
    now: Instant,
) {
    let conflicts: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| !i.is_dir && i.exists.is_some())
        .map(|(n, _)| n)
        .collect();
    let Some(t) = s.transfer_mut(transfer) else {
        return;
    };
    t.items = items;
    if conflicts.is_empty() {
        next_or_run(s, transfer, fx, now);
    } else {
        t.state = TState::Asking;
        s.ask = Some(Ask {
            transfer,
            conflicts,
            at: 0,
        });
    }
}

pub fn update(
    sessions: &mut Vec<FileSession>,
    drafts: &mut BTreeMap<FmId, FileDraft>,
    next_id: &mut FmId,
    host_entry: &dyn Fn(&str) -> Option<HostEntry>,
    msg: FmMsg,
    now: Instant,
) -> Vec<FmEffect> {
    let mut fx = Vec::new();
    if let FmMsg::Open(alias) = msg {
        if sessions.iter().any(|s| s.host.alias == alias) {
            return fx;
        }
        let Some(host) = host_entry(&alias) else {
            return fx;
        };
        *next_id += 1;
        let id = *next_id;
        sessions.push(FileSession {
            id,
            host: host.clone(),
            ..Default::default()
        });
        drafts.insert(id, FileDraft::default());
        fx.push(FmEffect::Connect { id, host });
        return fx;
    }
    let id = msg_id(&msg);
    let Some(pos) = sessions.iter().position(|s| s.id == id) else {
        return fx;
    };
    if let FmMsg::Close(_) = msg {
        let s = sessions.remove(pos);
        drafts.remove(&id);
        if s.status == Status::Ready {
            fx.push(FmEffect::SavePaths {
                alias: s.host.alias.clone(),
                local: s.local.path.clone(),
                remote: s.remote.path.clone(),
            });
        }
        for t in s.transfers.iter().filter(|t| !t.state.finished()) {
            fx.push(FmEffect::Cancel { id, transfer: t.id });
        }
        fx.push(FmEffect::Close(id));
        return fx;
    }
    let s = &mut sessions[pos];
    let d = drafts.entry(id).or_default();
    match msg {
        FmMsg::Open(_) | FmMsg::Close(_) => {}
        FmMsg::Connected {
            result, local_home, ..
        } => match result {
            Ok((mode, home, remembered)) => {
                s.status = Status::Ready;
                s.mode = mode;
                s.error = None;
                s.remote_home = home.clone();
                let (l, r) = remembered.unwrap_or((local_home, home));
                list(&mut fx, s, d, Side::Local, l);
                list(&mut fx, s, d, Side::Remote, r);
            }
            Err(e) => s.error = Some(e),
        },
        FmMsg::Retry(_) => {
            s.error = None;
            s.status = Status::Connecting;
            fx.push(FmEffect::Close(id));
            fx.push(FmEffect::Connect {
                id,
                host: s.host.clone(),
            });
        }
        FmMsg::Navigate { side, path, .. } => {
            let p = path.trim().to_string();
            if !p.is_empty() {
                d.selected_mut(side).clear();
                list(&mut fx, s, d, side, p);
            }
        }
        FmMsg::Enter { side, name, .. } => {
            let p = join(side, &s.pane(side).path, &name);
            d.selected_mut(side).clear();
            list(&mut fx, s, d, side, p);
        }
        FmMsg::Up { side, .. } => {
            if let Some(p) = parent(side, &s.pane(side).path) {
                d.selected_mut(side).clear();
                list(&mut fx, s, d, side, p);
            }
        }
        FmMsg::Home { side, .. } => {
            let p = match side {
                Side::Local => telepad_core::files::local_home(),
                Side::Remote => s.remote_home.clone(),
            };
            d.selected_mut(side).clear();
            list(&mut fx, s, d, side, p);
        }
        FmMsg::Refresh { side, .. } => {
            let p = s.pane(side).path.clone();
            list(&mut fx, s, d, side, p);
        }
        FmMsg::Listed { side, result, .. } => {
            let p = s.pane_mut(side);
            p.loading = false;
            match result {
                Ok((path, entries)) => {
                    p.path = path.clone();
                    p.entries = entries;
                    p.error = None;
                    *d.path_mut(side) = path;
                    let names: BTreeSet<String> =
                        p.entries.iter().map(|e| e.name.clone()).collect();
                    d.selected_mut(side).retain(|n| names.contains(n));
                }
                Err(e) => {
                    // Keep showing the folder that worked; the path bar goes back to it.
                    p.error = Some(e);
                    *d.path_mut(side) = p.path.clone();
                }
            }
        }
        FmMsg::Sort { side, col, .. } => {
            let p = s.pane_mut(side);
            if p.sort == col {
                p.descending = !p.descending;
            } else {
                p.sort = col;
                p.descending = false;
            }
        }
        FmMsg::Copy { from, names, .. } => {
            if names.is_empty() || s.status != Status::Ready {
                return fx;
            }
            let direction = match from {
                Side::Local => Direction::Upload,
                Side::Remote => Direction::Download,
            };
            let src_dir = s.pane(from).path.clone();
            let dest = s.pane(other(from)).path.clone();
            let sources: Vec<Source> = names
                .iter()
                .filter_map(|n| s.pane(from).entries.iter().find(|e| &e.name == n))
                .map(|e| (join(from, &src_dir, &e.name), e.is_dir()))
                .collect();
            if sources.is_empty() {
                return fx;
            }
            s.next_transfer += 1;
            let tid = s.next_transfer;
            let rsync = d.use_rsync && matches!(s.rsync, Some(Ok(())));
            let what = if names.len() == 1 {
                names[0].clone()
            } else {
                format!("{} items", names.len())
            };
            s.transfers.push(Transfer {
                id: tid,
                direction,
                title: format!(
                    "{} {what} {} {}",
                    if direction == Direction::Upload {
                        "Upload"
                    } else {
                        "Download"
                    },
                    "→",
                    short_dir(&dest)
                ),
                sources: sources.clone(),
                dest: dest.clone(),
                rsync,
                state: TState::Planning,
                items: vec![],
                decisions: HashMap::new(),
                current: String::new(),
                done_items: 0,
                bytes_total: 0,
                bytes_done: 0,
                started: None,
                errors: vec![],
            });
            if rsync {
                fx.push(FmEffect::Rsync {
                    id,
                    transfer: tid,
                    direction,
                    sources: sources.into_iter().map(|(p, _)| p).collect(),
                    dest,
                    dry_run: true,
                });
            } else if s.mode == Mode::Fallback {
                let items = fallback_items(s, from, &names);
                planned(s, tid, items, &mut fx, now);
            } else {
                fx.push(FmEffect::Plan {
                    id,
                    transfer: tid,
                    direction,
                    sources,
                    dest,
                });
            }
        }
        FmMsg::Planned {
            transfer, result, ..
        } => match result {
            Ok(items) => planned(s, transfer, items, &mut fx, now),
            Err(e) => {
                if let Some(t) = s.transfer_mut(transfer) {
                    t.state = if e.contains("Cancelled") {
                        TState::Cancelled
                    } else {
                        TState::Failed(e)
                    };
                }
            }
        },
        FmMsg::Decide { decision, .. } => {
            let Some(a) = s.ask.clone() else {
                return fx;
            };
            let all = d.apply_all;
            if let Some(t) = s.transfer_mut(a.transfer) {
                let targets: Vec<usize> = if all {
                    a.conflicts[a.at..].to_vec()
                } else {
                    vec![a.conflicts[a.at]]
                };
                for i in targets {
                    t.decisions.insert(i, decision);
                }
            }
            let at = if all { a.conflicts.len() } else { a.at + 1 };
            s.ask = Some(Ask { at, ..a.clone() });
            d.apply_all = false;
            next_or_run(s, a.transfer, &mut fx, now);
        }
        FmMsg::AskCancel(_) => {
            if let Some(a) = s.ask.take()
                && let Some(t) = s.transfer_mut(a.transfer)
            {
                t.state = TState::Cancelled;
            }
        }
        FmMsg::Progress {
            transfer, event, ..
        } => {
            if let Some(t) = s.transfer_mut(transfer) {
                match event {
                    Event::Start(i) => {
                        if let Some(it) = t.items.get(i) {
                            t.current = it.label.clone();
                        }
                    }
                    Event::Bytes(n) => t.bytes_done += n,
                    Event::Done(i, r) => {
                        t.done_items += 1;
                        if let Err(e) = r
                            && !e.contains("Cancelled")
                        {
                            let label = t
                                .items
                                .get(i)
                                .map(|it| it.label.clone())
                                .unwrap_or_default();
                            t.errors.push(format!("{label}: {e}"));
                        }
                    }
                }
            }
        }
        FmMsg::Finished {
            transfer, result, ..
        } => {
            let mut refresh = None;
            if let Some(t) = s.transfer_mut(transfer) {
                t.current.clear();
                t.state = match result {
                    Ok((copied, skipped, failed)) => TState::Done {
                        copied,
                        skipped,
                        failed,
                    },
                    Err(e) if e.contains("Cancelled") => TState::Cancelled,
                    Err(e) => TState::Failed(e),
                };
                refresh = Some(match t.direction {
                    Direction::Upload => Side::Remote,
                    Direction::Download => Side::Local,
                });
            }
            if let Some(side) = refresh {
                let p = s.pane(side).path.clone();
                list(&mut fx, s, d, side, p);
            }
        }
        FmMsg::Cancel { transfer, .. } => {
            if let Some(t) = s.transfer_mut(transfer) {
                match t.state {
                    TState::Asking | TState::Previewed(_) => {
                        t.state = TState::Cancelled;
                        if s.ask.as_ref().is_some_and(|a| a.transfer == transfer) {
                            s.ask = None;
                        }
                    }
                    TState::Planning | TState::Running => {
                        fx.push(FmEffect::Cancel { id, transfer })
                    }
                    _ => {}
                }
            }
        }
        FmMsg::ClearFinished(_) => s.transfers.retain(|t| !t.state.finished()),
        FmMsg::RsyncChecked { result, .. } => s.rsync = Some(result),
        FmMsg::RsyncPreviewed {
            transfer, result, ..
        } => {
            if let Some(t) = s.transfer_mut(transfer) {
                t.state = match result {
                    Ok(list) => TState::Previewed(list),
                    Err(e) => TState::Failed(e),
                };
            }
        }
        FmMsg::RsyncRun { transfer, .. } => {
            if let Some(t) = s.transfer_mut(transfer)
                && matches!(t.state, TState::Previewed(_))
            {
                t.state = TState::Running;
                t.started = Some(now);
                fx.push(FmEffect::Rsync {
                    id,
                    transfer,
                    direction: t.direction,
                    sources: t.sources.iter().map(|(p, _)| p.clone()).collect(),
                    dest: t.dest.clone(),
                    dry_run: false,
                });
            }
        }
    }
    fx
}

fn msg_id(m: &FmMsg) -> FmId {
    use FmMsg::*;
    match m {
        Open(_) => 0,
        Connected { id, .. }
        | Navigate { id, .. }
        | Enter { id, .. }
        | Up { id, .. }
        | Home { id, .. }
        | Refresh { id, .. }
        | Listed { id, .. }
        | Sort { id, .. }
        | Copy { id, .. }
        | Planned { id, .. }
        | Decide { id, .. }
        | Progress { id, .. }
        | Finished { id, .. }
        | Cancel { id, .. }
        | RsyncChecked { id, .. }
        | RsyncPreviewed { id, .. }
        | RsyncRun { id, .. } => *id,
        Retry(id) | Close(id) | AskCancel(id) | ClearFinished(id) => *id,
    }
}
