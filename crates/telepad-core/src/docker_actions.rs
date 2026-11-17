//! Docker / Podman actions and the read-only "how it runs" view.
//!
//! Actions never run in the background: they become a small POSIX `sh` script
//! that runs in a terminal the user opened. It prints the exact command, waits
//! for Enter (or `yes` for destructive ones) and only then runs it — the second
//! documented exception to Telepad's read-only rule (see AGENTS.md).
//!
//! Container IDs, compose project names and paths come from the host (labels
//! are set by whoever started the container), so they are validated and every
//! argument is single-quoted before it reaches the remote shell.

use serde_json::Value;

use crate::cluster::Workload;
use crate::model::Container;
use crate::platform::shell_quote;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerAction {
    Shell,
    Logs,
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    Kill,
    Remove,
}

impl ContainerAction {
    pub const ALL: [ContainerAction; 9] = [
        Self::Shell,
        Self::Logs,
        Self::Start,
        Self::Stop,
        Self::Restart,
        Self::Pause,
        Self::Unpause,
        Self::Kill,
        Self::Remove,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Shell => "Shell (exec)",
            Self::Logs => "Logs",
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
            Self::Pause => "Pause",
            Self::Unpause => "Resume",
            Self::Kill => "Kill",
            Self::Remove => "Remove",
        }
    }

    /// Changes the container (needs Enter before it runs).
    pub fn changes(self) -> bool {
        !matches!(self, Self::Shell | Self::Logs)
    }

    /// Hard to undo: needs `yes` typed before it runs.
    pub fn destructive(self) -> bool {
        matches!(self, Self::Kill | Self::Remove)
    }

    /// Offered for a container in this `docker ps` state.
    pub fn available(self, state: &str) -> bool {
        let running = state == "running";
        let paused = state == "paused";
        match self {
            Self::Shell | Self::Kill => running,
            Self::Logs | Self::Restart => true,
            Self::Start => !running && !paused,
            Self::Stop => running || paused,
            Self::Pause => running,
            Self::Unpause => paused,
            Self::Remove => !running && !paused,
        }
    }

    fn argv(self, id: &str) -> Vec<String> {
        let a: &[&str] = match self {
            Self::Shell => &[
                "exec",
                "-it",
                id,
                "sh",
                "-c",
                "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi",
            ],
            Self::Logs => &["logs", "-f", "--tail", "200", id],
            Self::Start => &["start", id],
            Self::Stop => &["stop", id],
            Self::Restart => &["restart", id],
            Self::Pause => &["pause", id],
            Self::Unpause => &["unpause", id],
            Self::Kill => &["kill", id],
            // No -f: a running container must be stopped first; volumes are kept.
            Self::Remove => &["rm", id],
        };
        a.iter().map(|s| s.to_string()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackAction {
    Logs,
    Up,
    Restart,
    Stop,
    Down,
}

impl StackAction {
    pub const ALL: [StackAction; 5] = [Self::Logs, Self::Up, Self::Restart, Self::Stop, Self::Down];

    pub fn title(self) -> &'static str {
        match self {
            Self::Logs => "Logs",
            Self::Up => "Up (create / start)",
            Self::Restart => "Restart",
            Self::Stop => "Stop",
            Self::Down => "Down (remove containers)",
        }
    }

    pub fn changes(self) -> bool {
        self != Self::Logs
    }

    pub fn destructive(self) -> bool {
        self == Self::Down
    }

    fn argv(self) -> &'static [&'static str] {
        match self {
            Self::Logs => &["logs", "-f", "--tail", "200"],
            Self::Up => &["up", "-d"],
            Self::Restart => &["restart"],
            Self::Stop => &["stop"],
            // No -v: named volumes are kept.
            Self::Down => &["down"],
        }
    }
}

/// A docker compose project as recorded in its containers' labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeProject {
    pub name: String,
    pub dir: String,
    pub files: Vec<String>,
}

impl ComposeProject {
    /// From the compose labels of any container of the stack.
    pub fn from_container(c: &Container) -> Result<Self, String> {
        let files = c
            .config_files
            .split(',')
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(String::from)
            .collect();
        let p = Self {
            name: c.stack.clone(),
            dir: c.working_dir.clone(),
            files,
        };
        p.validate()?;
        Ok(p)
    }

    fn validate(&self) -> Result<(), String> {
        if !valid_project(&self.name) {
            return Err(format!("unexpected compose project name {:?}", self.name));
        }
        for path in std::iter::once(&self.dir).chain(&self.files) {
            if !valid_path(path) {
                return Err(format!("unexpected compose path {path:?}"));
            }
        }
        Ok(())
    }

    /// Arguments after `docker compose`, the same for every action.
    fn base(&self) -> Vec<String> {
        let mut a = vec![
            "--project-directory".to_string(),
            self.dir.clone(),
            "-p".into(),
            self.name.clone(),
        ];
        for f in &self.files {
            a.push("-f".into());
            a.push(f.clone());
        }
        a
    }
}

/// `docker ps --no-trunc` IDs are 64 hex digits; accept short IDs too.
pub fn valid_id(id: &str) -> bool {
    (12..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Compose project names: lowercase letters, digits, `-` and `_`.
fn valid_project(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn valid_path(p: &str) -> bool {
    p.starts_with('/') && p.len() < 1024 && !p.chars().any(char::is_control)
}

/// What to put in the terminal for one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Shown in the terminal header and in Telepad's notice.
    pub title: String,
    tool: Tool,
    args: Vec<String>,
    confirm: Confirm,
    /// Scale: the replica count is asked in the terminal and appended.
    scale: Option<Scale>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tool {
    Docker,
    Compose,
    /// `kubectl`, `k3s kubectl` or `microk8s kubectl`.
    Kube(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Scale {
    /// `docker service scale <service>=N`
    Service(String),
    /// `kubectl scale ... --replicas=N`
    Replicas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Confirm {
    None,
    Enter,
    Yes,
}

impl Plan {
    pub fn container(c: &Container, action: ContainerAction) -> Result<Self, String> {
        if !valid_id(&c.id) {
            return Err(format!("unexpected container id {:?}", c.id));
        }
        Ok(Self::new(
            format!("{} container {}", action.title(), c.name),
            Tool::Docker,
            action.argv(&c.id),
            confirm(action.changes(), action.destructive()),
        ))
    }

    pub fn stack(p: &ComposeProject, action: StackAction) -> Result<Self, String> {
        p.validate()?;
        let mut args = p.base();
        args.extend(action.argv().iter().map(|s| s.to_string()));
        Ok(Self::new(
            format!("{} compose stack {}", action.title(), p.name),
            Tool::Compose,
            args,
            confirm(action.changes(), action.destructive()),
        ))
    }

    pub fn swarm_service(service: &str, action: ServiceAction) -> Result<Self, String> {
        if !valid_swarm_name(service) {
            return Err(format!("unexpected service name {service:?}"));
        }
        let (args, confirm, scale): (&[&str], _, _) = match action {
            ServiceAction::Logs => (
                &["service", "logs", "-f", "--tail", "200", service],
                Confirm::None,
                None,
            ),
            // A rolling restart: new tasks replace the old ones one by one.
            ServiceAction::Restart => (
                &["service", "update", "--force", service],
                Confirm::Enter,
                None,
            ),
            ServiceAction::Scale => (
                &["service", "scale"],
                Confirm::Enter,
                Some(Scale::Service(service.to_string())),
            ),
        };
        let mut p = Self::new(
            format!("{} Swarm service {service}", action.title()),
            Tool::Docker,
            strings(args),
            confirm,
        );
        p.scale = scale;
        Ok(p)
    }

    pub fn swarm_stack_remove(stack: &str) -> Result<Self, String> {
        if !valid_swarm_name(stack) {
            return Err(format!("unexpected stack name {stack:?}"));
        }
        Ok(Self::new(
            format!("Remove Swarm stack {stack}"),
            Tool::Docker,
            strings(&["stack", "rm", stack]),
            Confirm::Yes,
        ))
    }

    pub fn swarm_node(node_id: &str, hostname: &str, action: NodeAction) -> Result<Self, String> {
        if node_id.len() < 12
            || node_id.len() > 64
            || !node_id.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(format!("unexpected node id {node_id:?}"));
        }
        let availability = match action {
            NodeAction::Drain => "drain",
            NodeAction::Pause => "pause",
            NodeAction::Activate => "active",
        };
        Ok(Self::new(
            format!("{} Swarm node {hostname}", action.title()),
            Tool::Docker,
            strings(&["node", "update", "--availability", availability, node_id]),
            Confirm::Enter,
        ))
    }

    pub fn kube_workload(tool: &str, w: &Workload, action: WorkloadAction) -> Result<Self, String> {
        let tool = KUBE_TOOLS
            .into_iter()
            .find(|t| *t == tool)
            .ok_or_else(|| format!("unexpected kubectl tool {tool:?}"))?;
        if !valid_kube_name(&w.namespace) || !valid_kube_name(&w.name) {
            return Err(format!(
                "unexpected workload name {}/{}",
                w.namespace, w.name
            ));
        }
        if action == WorkloadAction::Scale && !w.kind.scalable() {
            return Err(format!("a {} cannot be scaled", w.kind.singular()));
        }
        let target = format!("{}/{}", w.kind.singular(), w.name);
        let ns = w.namespace.as_str();
        let (args, confirm): (Vec<String>, _) = match action {
            WorkloadAction::Logs => (
                strings(&["logs", "-f", "--tail", "200", "-n", ns, &target]),
                Confirm::None,
            ),
            WorkloadAction::Restart => (
                strings(&["rollout", "restart", "-n", ns, &target]),
                Confirm::Enter,
            ),
            WorkloadAction::Scale => (strings(&["scale", "-n", ns, &target]), Confirm::Enter),
        };
        let mut p = Self::new(
            format!("{} {} {ns}/{}", action.title(), w.kind.singular(), w.name),
            Tool::Kube(tool),
            args,
            confirm,
        );
        if action == WorkloadAction::Scale {
            p.scale = Some(Scale::Replicas);
        }
        Ok(p)
    }

    fn new(title: String, tool: Tool, args: Vec<String>, confirm: Confirm) -> Self {
        Self {
            title,
            tool,
            args,
            confirm,
            scale: None,
        }
    }

    fn quoted_args(&self) -> String {
        let args: Vec<String> = self.args.iter().map(|a| shell_quote(a)).collect();
        args.join(" ")
    }

    /// The scale suffix, as shown (`<replicas>`) or as run (`"$n"`).
    fn scale_suffix(&self, n: &str) -> String {
        match &self.scale {
            None => String::new(),
            Some(Scale::Service(s)) => format!(" {}={n}", shell_quote(s)),
            Some(Scale::Replicas) => format!(" --replicas={n}"),
        }
    }

    /// The command as the user will see it (runtime shown as `docker`).
    pub fn display(&self) -> String {
        let prefix = match &self.tool {
            Tool::Docker => "docker",
            Tool::Compose => "docker compose",
            Tool::Kube(t) => t,
        };
        format!(
            "{prefix} {}{}",
            self.quoted_args(),
            self.scale_suffix("<replicas>")
        )
    }

    /// The POSIX `sh` script the terminal runs.
    pub fn script(&self) -> String {
        let args = self.quoted_args();
        let runtime = match &self.tool {
            Tool::Docker => format!("{RUNTIME}T=\"$D\"\n"),
            Tool::Compose => format!(
                "{RUNTIME}T=\"$D compose\"\nif ! $D compose version >/dev/null 2>&1 && command -v docker-compose >/dev/null 2>&1; then\n  case \"$D\" in sudo*) T=\"sudo docker-compose\" ;; *) T=docker-compose ;; esac\nfi\n"
            ),
            Tool::Kube(t) => format!(
                "T={q}\nset -- $T\ncommand -v \"$1\" >/dev/null 2>&1 || {{ echo \"$1 is not installed.\"; read _ </dev/tty; exit 1; }}\n$T get nodes -o name >/dev/null 2>&1 || T=\"sudo $T\"\n",
                q = shell_quote(t)
            ),
        };
        let mut confirm = match self.confirm {
            Confirm::None => String::new(),
            Confirm::Enter => "printf 'Press Enter to run it, or Ctrl+C to cancel. '\nread _ </dev/tty\n".into(),
            Confirm::Yes => "printf 'This cannot be undone. Type yes and press Enter to run it: '\nread ans </dev/tty\n[ \"$ans\" = yes ] || { echo 'Cancelled.'; sleep 2; exit 1; }\n".into(),
        };
        if self.scale.is_some() {
            confirm = "printf 'New number of replicas (Ctrl+C to cancel): '\nread n </dev/tty\ncase \"$n\" in ''|*[!0-9]*) echo 'Not a number, cancelled.'; sleep 2; exit 1 ;; esac\n".into();
        }
        let shown = format!("{args}{}", self.scale_suffix("<replicas>"));
        format!(
            "{runtime}echo {title}\necho\necho 'Command:'\nprintf '  %s %s\\n\\n' \"$T\" {shown}\n{confirm}$T {args}{run_suffix} </dev/tty\nrc=$?\necho\nprintf 'Finished (exit %s). Press Enter to close. ' \"$rc\"\nread _ </dev/tty\n",
            title = shell_quote(&format!("=== Telepad: {} ===", self.title)),
            shown = shell_quote(&shown),
            run_suffix = self.scale_suffix("\"$n\""),
        )
    }
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn confirm(changes: bool, destructive: bool) -> Confirm {
    if destructive {
        Confirm::Yes
    } else if changes {
        Confirm::Enter
    } else {
        Confirm::None
    }
}

const KUBE_TOOLS: [&str; 3] = ["kubectl", "k3s kubectl", "microk8s kubectl"];

/// Swarm service / stack names: letters, digits, `_`, `.`, `-`.
fn valid_swarm_name(s: &str) -> bool {
    s.len() <= 128
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// Kubernetes names and namespaces (DNS-1123).
fn valid_kube_name(s: &str) -> bool {
    s.len() <= 253
        && s.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && s.bytes()
            .last()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Logs,
    Restart,
    Scale,
}

impl ServiceAction {
    pub const ALL: [ServiceAction; 3] = [Self::Logs, Self::Restart, Self::Scale];
    pub fn title(self) -> &'static str {
        match self {
            Self::Logs => "Logs",
            Self::Restart => "Restart (rolling)",
            Self::Scale => "Scale…",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeAction {
    Drain,
    Pause,
    Activate,
}

impl NodeAction {
    pub const ALL: [NodeAction; 3] = [Self::Drain, Self::Pause, Self::Activate];
    pub fn title(self) -> &'static str {
        match self {
            Self::Drain => "Drain",
            Self::Pause => "Pause",
            Self::Activate => "Activate",
        }
    }
    /// Offered for a node with this availability (Active / Pause / Drain).
    pub fn available(self, availability: &str) -> bool {
        let a = availability.to_ascii_lowercase();
        match self {
            Self::Drain => a != "drain",
            Self::Pause => a == "active",
            Self::Activate => a != "active",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadAction {
    Logs,
    Restart,
    Scale,
}

impl WorkloadAction {
    pub const ALL: [WorkloadAction; 3] = [Self::Logs, Self::Restart, Self::Scale];
    pub fn title(self) -> &'static str {
        match self {
            Self::Logs => "Logs",
            Self::Restart => "Rollout restart",
            Self::Scale => "Scale…",
        }
    }
}

/// Same runtime choice as `script.sh`, but `sudo` may ask for a password here.
const RUNTIME: &str = "D=''\nfor rt in docker podman; do\n  command -v \"$rt\" >/dev/null 2>&1 || continue\n  if \"$rt\" ps -q >/dev/null 2>&1; then D=\"$rt\"; else D=\"sudo $rt\"; fi\n  break\ndone\n[ -n \"$D\" ] || { echo 'Neither docker nor podman is installed.'; read _ </dev/tty; exit 1; }\n";

/// Read-only script for "How it runs": `docker inspect` plus the compose files.
pub fn run_info_script(c: &Container) -> Result<String, String> {
    if !valid_id(&c.id) {
        return Err(format!("unexpected container id {:?}", c.id));
    }
    let mut s = String::from(
        "LC_ALL=C; export LC_ALL\nD=''\nfor rt in docker podman; do\n  command -v \"$rt\" >/dev/null 2>&1 || continue\n  if \"$rt\" ps -q >/dev/null 2>&1; then D=\"$rt\"; break\n  elif sudo -n \"$rt\" ps -q >/dev/null 2>&1; then D=\"sudo -n $rt\"; break; fi\ndone\n[ -n \"$D\" ] || { echo 'ERR docker not usable'; exit 0; }\necho '===INSPECT==='\n",
    );
    s.push_str(&format!("$D inspect {} 2>&1\n", shell_quote(&c.id)));
    // Portainer keeps its stack files inside its own volume, not on the host.
    let portainer = matches!(c.manager(), crate::model::Manager::Portainer { .. });
    if let Some(p) = ComposeProject::from_container(c)
        .ok()
        .filter(|_| !portainer)
    {
        for f in &p.files {
            let q = shell_quote(f);
            s.push_str(&format!(
                "echo {}\nif [ -r {q} ]; then head -c 200000 {q}; else echo '(cannot read this file: missing, or no permission for this user)'; fi\n",
                shell_quote(&format!("===FILE {f}===")),
            ));
        }
    }
    Ok(s)
}

/// Result of "How it runs", secrets already masked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunInfo {
    pub container: String,
    /// Reconstructed `docker run` (approximate: image defaults are included).
    pub docker_run: String,
    /// (path, content) of each compose file.
    pub compose_files: Vec<(String, String)>,
    pub error: Option<String>,
}

pub fn parse_run_info(container: &str, out: &str) -> RunInfo {
    let mut info = RunInfo {
        container: container.to_string(),
        ..Default::default()
    };
    if let Some(e) = out.lines().find_map(|l| l.strip_prefix("ERR ")) {
        info.error = Some(e.to_string());
        return info;
    }
    let mut section: Option<String> = None;
    let mut inspect = String::new();
    for line in out.lines() {
        if line == "===INSPECT===" {
            section = Some(String::new());
            continue;
        }
        if let Some(f) = line
            .strip_prefix("===FILE ")
            .and_then(|l| l.strip_suffix("==="))
        {
            section = Some(f.to_string());
            info.compose_files.push((f.to_string(), String::new()));
            continue;
        }
        match section.as_deref() {
            Some("") => {
                inspect.push_str(line);
                inspect.push('\n');
            }
            Some(_) => {
                if let Some((_, text)) = info.compose_files.last_mut() {
                    text.push_str(&mask_line(line));
                    text.push('\n');
                }
            }
            None => {}
        }
    }
    match serde_json::from_str::<Value>(&inspect) {
        Ok(v) => match v.get(0) {
            Some(c) => info.docker_run = docker_run(c),
            None => info.error = Some("docker inspect returned nothing".into()),
        },
        Err(_) => info.error = Some(inspect.trim().chars().take(300).collect()),
    }
    info
}

const SECRET_WORDS: [&str; 10] = [
    "PASS", "PWD", "SECRET", "TOKEN", "KEY", "CRED", "AUTH", "PRIVATE", "DSN", "SALT",
];

fn looks_secret(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    SECRET_WORDS.iter().any(|w| n.contains(w))
}

/// `KEY=value` with a secret-looking key, or a URL with a password → `***`.
fn mask_env(kv: &str) -> String {
    match kv.split_once('=') {
        Some((k, v)) if looks_secret(k) && !v.is_empty() => format!("{k}=***"),
        Some((k, v)) => format!("{k}={}", mask_url(v)),
        None => kv.to_string(),
    }
}

/// `scheme://user:password@host` → `scheme://user:***@host`.
fn mask_url(v: &str) -> String {
    let Some(start) = v.find("://") else {
        return v.to_string();
    };
    let rest = &v[start + 3..];
    let Some(at) = rest.find('@') else {
        return v.to_string();
    };
    match rest[..at].split_once(':') {
        Some((user, _)) => format!("{}{user}:***{}", &v[..start + 3], &rest[at..]),
        None => v.to_string(),
    }
}

/// Compose file lines: `KEY: value`, `- KEY=value`, `KEY=value`.
fn mask_line(line: &str) -> String {
    let indent = &line[..line.len() - line.trim_start().len()];
    let t = line.trim_start();
    let (dash, body) = match t.strip_prefix("- ") {
        Some(b) => ("- ", b),
        None => ("", t),
    };
    let colon = body.find(':');
    let equals = body.find('=');
    let masked = match (colon, equals) {
        (Some(c), e) if e.is_none_or(|e| c < e) => {
            let (k, v) = body.split_at(c);
            let v = &v[1..];
            let key = k.trim_matches(['"', '\'']);
            if looks_secret(key) && !v.trim().is_empty() && !v.trim().starts_with(['{', '[']) {
                format!("{k}: ***")
            } else {
                format!("{k}:{}", mask_url(v))
            }
        }
        (_, Some(_)) => mask_env(body),
        _ => body.to_string(),
    };
    format!("{indent}{dash}{masked}")
}

fn s<'a>(v: &'a Value, path: &[&str]) -> &'a str {
    path.iter()
        .try_fold(v, |v, k| v.get(*k))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Rebuild a `docker run` command from one `docker inspect` object, one option
/// per line. Approximate: image defaults (env, cmd) show up as explicit options.
pub fn docker_run(c: &Value) -> String {
    let mut lines: Vec<String> = vec!["docker run".into(), "-d".into()];
    let mut opt = |k: &str, v: &str| lines.push(format!("{k} {}", shell_quote(v)));
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|x| {
                x.iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let name = s(c, &["Name"]).trim_start_matches('/');
    if !name.is_empty() {
        opt("--name", name);
    }
    let hc = c.get("HostConfig").cloned().unwrap_or(Value::Null);
    let restart = s(&hc, &["RestartPolicy", "Name"]);
    if !restart.is_empty() && restart != "no" {
        let max = hc
            .get("RestartPolicy")
            .and_then(|r| r.get("MaximumRetryCount"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if restart == "on-failure" && max > 0 {
            opt("--restart", &format!("on-failure:{max}"));
        } else {
            opt("--restart", restart);
        }
    }
    let mut net = s(&hc, &["NetworkMode"]).to_string();
    // Portainer stores the network ID; show the name when it is unambiguous.
    if valid_id(&net)
        && let Some(nets) = c
            .get("NetworkSettings")
            .and_then(|n| n.get("Networks"))
            .and_then(Value::as_object)
        && let Some(name) = nets
            .iter()
            .find(|(_, v)| s(v, &["NetworkID"]) == net)
            .map(|(k, _)| k.clone())
            .or_else(|| {
                (nets.len() == 1)
                    .then(|| nets.keys().next().cloned())
                    .flatten()
            })
    {
        net = name;
    }
    if !net.is_empty() && net != "default" && net != "bridge" {
        opt("--network", &net);
    }
    for cap in strings(hc.get("CapAdd")) {
        opt("--cap-add", &cap);
    }
    if let Some(bindings) = hc.get("PortBindings").and_then(Value::as_object) {
        let mut ports: Vec<_> = bindings.iter().collect();
        ports.sort_by(|x, y| x.0.cmp(y.0));
        let mut seen: Vec<String> = Vec::new();
        for (cport, binds) in ports {
            let cport = cport.strip_suffix("/tcp").unwrap_or(cport);
            for b in binds.as_array().into_iter().flatten() {
                let spec = match (s(b, &["HostIp"]), s(b, &["HostPort"])) {
                    ("" | "0.0.0.0" | "::", "") => cport.to_string(),
                    ("" | "0.0.0.0" | "::", hp) => format!("{hp}:{cport}"),
                    (ip, hp) => format!("{ip}:{hp}:{cport}"),
                };
                if !seen.contains(&spec) {
                    opt("-p", &spec);
                    seen.push(spec);
                }
            }
        }
    }
    for m in c
        .get("Mounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let src = match s(m, &["Type"]) {
            "volume" => s(m, &["Name"]),
            _ => s(m, &["Source"]),
        };
        let dst = s(m, &["Destination"]);
        if src.is_empty() || dst.is_empty() {
            continue;
        }
        let ro = m.get("RW").and_then(Value::as_bool) == Some(false);
        opt("-v", &format!("{src}:{dst}{}", if ro { ":ro" } else { "" }));
    }
    let cfg = c.get("Config").cloned().unwrap_or(Value::Null);
    for e in strings(cfg.get("Env")) {
        opt("-e", &mask_env(&e));
    }
    if hc.get("Privileged").and_then(Value::as_bool) == Some(true) {
        lines.push("--privileged".into());
    }
    let image = s(&cfg, &["Image"]);
    let mut last = shell_quote(if image.is_empty() { "<image>" } else { image });
    for arg in strings(cfg.get("Cmd")) {
        last.push(' ');
        last.push_str(&shell_quote(&arg));
    }
    lines.push(last);
    lines.join(" \\\n  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "4f1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c";

    fn container() -> Container {
        Container {
            id: ID.into(),
            name: "shop-web".into(),
            state: "running".into(),
            stack: "shop".into(),
            working_dir: "/srv/shop".into(),
            config_files: "/srv/shop/compose.yml,/srv/shop/compose.override.yml".into(),
            ..Default::default()
        }
    }

    #[test]
    fn container_commands_use_the_id_and_confirm_changes() {
        let c = container();
        let p = Plan::container(&c, ContainerAction::Stop).unwrap();
        assert_eq!(p.display(), format!("docker stop {ID}"));
        let sc = p.script();
        assert!(sc.contains("Press Enter to run it"));
        assert!(sc.contains(&format!("$T stop {ID} </dev/tty")));

        let p = Plan::container(&c, ContainerAction::Remove).unwrap();
        assert!(p.script().contains("Type yes"), "remove needs yes");
        assert_eq!(p.display(), format!("docker rm {ID}"), "no -f");

        let p = Plan::container(&c, ContainerAction::Logs).unwrap();
        assert!(
            !p.script().contains("Press Enter to run"),
            "logs run directly"
        );

        let mut bad = container();
        bad.id = "abc; rm -rf /".into();
        assert!(Plan::container(&bad, ContainerAction::Stop).is_err());
    }

    #[test]
    fn actions_offered_by_state() {
        use ContainerAction::*;
        let on = |s: &str| -> Vec<ContainerAction> {
            ContainerAction::ALL
                .into_iter()
                .filter(|a| a.available(s))
                .collect()
        };
        assert_eq!(on("running"), [Shell, Logs, Stop, Restart, Pause, Kill]);
        assert_eq!(on("exited"), [Logs, Start, Restart, Remove]);
        assert_eq!(on("paused"), [Logs, Stop, Restart, Unpause]);
    }

    #[test]
    fn compose_commands_quote_paths_and_reject_odd_labels() {
        let p = ComposeProject::from_container(&container()).unwrap();
        let plan = Plan::stack(&p, StackAction::Down).unwrap();
        assert_eq!(
            plan.display(),
            "docker compose --project-directory /srv/shop -p shop -f /srv/shop/compose.yml -f /srv/shop/compose.override.yml down"
        );
        assert!(plan.script().contains("Type yes"));

        let mut c = container();
        c.working_dir = "/srv/my shop's".into();
        c.config_files = "/srv/my shop's/compose.yml".into();
        let plan = Plan::stack(
            &ComposeProject::from_container(&c).unwrap(),
            StackAction::Up,
        )
        .unwrap();
        assert!(
            plan.script().contains(r"'/srv/my shop'\''s'"),
            "{}",
            plan.script()
        );

        for (dir, name) in [
            ("relative/dir", "shop"),
            ("/srv/x\nrm -rf /", "shop"),
            ("/srv", "Shop;x"),
        ] {
            let mut c = container();
            c.working_dir = dir.into();
            c.stack = name.into();
            assert!(
                ComposeProject::from_container(&c).is_err(),
                "{dir:?} {name:?}"
            );
        }
    }

    #[test]
    fn the_terminal_script_is_valid_sh() {
        // Parse-only check with the local sh when there is one (CI on Linux/macOS).
        let Ok(out) = std::process::Command::new("sh")
            .arg("-n")
            .arg("-c")
            .arg(
                Plan::stack(
                    &ComposeProject::from_container(&container()).unwrap(),
                    StackAction::Down,
                )
                .unwrap()
                .script(),
            )
            .output()
        else {
            return;
        };
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn swarm_and_kube_plans() {
        let p = Plan::swarm_service("shop_web", ServiceAction::Scale).unwrap();
        assert_eq!(p.display(), "docker service scale shop_web=<replicas>");
        let sc = p.script();
        assert!(
            sc.contains("read n </dev/tty") && sc.contains("*[!0-9]*"),
            "{sc}"
        );
        assert!(
            sc.contains("$T service scale shop_web=\"$n\" </dev/tty"),
            "{sc}"
        );
        assert_eq!(
            Plan::swarm_service("shop_web", ServiceAction::Restart)
                .unwrap()
                .display(),
            "docker service update --force shop_web"
        );
        assert!(
            Plan::swarm_stack_remove("shop")
                .unwrap()
                .script()
                .contains("Type yes")
        );
        assert!(Plan::swarm_service("x;reboot", ServiceAction::Logs).is_err());
        assert!(
            Plan::swarm_node("a1b2c3d4e5f6g7h8i9j0klmno", "w1", NodeAction::Drain)
                .unwrap()
                .display()
                .ends_with("--availability drain a1b2c3d4e5f6g7h8i9j0klmno")
        );
        assert!(Plan::swarm_node("$(id)", "w1", NodeAction::Drain).is_err());
        assert!(NodeAction::Activate.available("Drain") && !NodeAction::Drain.available("Drain"));

        let w = Workload {
            kind: crate::cluster::WorkloadKind::Deployment,
            namespace: "shop".into(),
            name: "web".into(),
            ready: 1,
            desired: 2,
        };
        let p = Plan::kube_workload("k3s kubectl", &w, WorkloadAction::Scale).unwrap();
        assert_eq!(
            p.display(),
            "k3s kubectl scale -n shop deployment/web --replicas=<replicas>"
        );
        assert!(p.script().contains("T='k3s kubectl'"));
        assert_eq!(
            Plan::kube_workload("kubectl", &w, WorkloadAction::Restart)
                .unwrap()
                .display(),
            "kubectl rollout restart -n shop deployment/web"
        );
        assert!(Plan::kube_workload("rm -rf /", &w, WorkloadAction::Logs).is_err());
        let mut bad = w.clone();
        bad.namespace = "Shop;x".into();
        assert!(Plan::kube_workload("kubectl", &bad, WorkloadAction::Logs).is_err());
        let mut ds = w.clone();
        ds.kind = crate::cluster::WorkloadKind::DaemonSet;
        assert!(Plan::kube_workload("kubectl", &ds, WorkloadAction::Scale).is_err());

        // Every script parses as sh (where a local sh exists).
        for p in [
            Plan::swarm_service("shop_web", ServiceAction::Scale).unwrap(),
            Plan::swarm_stack_remove("shop").unwrap(),
            Plan::kube_workload("microk8s kubectl", &w, WorkloadAction::Scale).unwrap(),
        ] {
            if let Ok(out) = std::process::Command::new("sh")
                .args(["-n", "-c", &p.script()])
                .output()
            {
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }

    #[test]
    fn reconstructs_docker_run_with_secrets_masked() {
        let inspect = r#"[{
          "Name": "/shop-db",
          "NetworkSettings": {"Networks": {"shop_default": {"NetworkID": "9f3c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c"}}},
          "Config": {"Image": "mariadb:11", "Cmd": ["mariadbd"],
                     "Env": ["MARIADB_ROOT_PASSWORD=hunter2", "TZ=Asia/Jakarta",
                             "DATABASE_URL=mysql://app:s3cret@db:3306/shop"]},
          "HostConfig": {"RestartPolicy": {"Name": "unless-stopped", "MaximumRetryCount": 0},
                         "NetworkMode": "9f3c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c",
                         "PortBindings": {"3306/tcp": [{"HostIp": "127.0.0.1", "HostPort": "3307"}],
                                          "8080/tcp": [{"HostIp": "", "HostPort": "8080"},
                                                       {"HostIp": "::", "HostPort": "8080"}]}},
          "Mounts": [{"Type": "volume", "Name": "shop_db", "Destination": "/var/lib/mysql", "RW": true},
                     {"Type": "bind", "Source": "/srv/shop/my.cnf", "Destination": "/etc/my.cnf", "RW": false}]
        }]"#;
        let out = format!(
            "===INSPECT===\n{inspect}\n===FILE /srv/shop/compose.yml===\nservices:\n  db:\n    environment:\n      MARIADB_ROOT_PASSWORD: hunter2\n      - API_TOKEN=abc\n      TZ: Asia/Jakarta\n"
        );
        let info = parse_run_info("shop-db", &out);
        assert_eq!(info.error, None);
        let run = &info.docker_run;
        assert!(run.starts_with("docker run \\\n  -d"), "{run}");
        for want in [
            "--name shop-db",
            "--restart unless-stopped",
            "--network shop_default",
            "-p 127.0.0.1:3307:3306",
            "-p 8080:8080",
            "-v shop_db:/var/lib/mysql",
            "-v /srv/shop/my.cnf:/etc/my.cnf:ro",
            "-e 'MARIADB_ROOT_PASSWORD=***'",
            "-e TZ=Asia/Jakarta",
            "-e 'DATABASE_URL=mysql://app:***@db:3306/shop'",
            "mariadb:11",
            "mariadbd",
        ] {
            assert!(run.contains(want), "missing {want:?} in\n{run}");
        }
        assert_eq!(run.matches("-p 8080:8080").count(), 1, "IPv4/IPv6 merged");
        assert!(!run.contains("hunter2") && !run.contains("s3cret"));
        let (path, text) = &info.compose_files[0];
        assert_eq!(path, "/srv/shop/compose.yml");
        assert!(text.contains("MARIADB_ROOT_PASSWORD: ***") && text.contains("- API_TOKEN=***"));
        assert!(text.contains("TZ: Asia/Jakarta") && !text.contains("hunter2"));
    }

    #[test]
    fn run_info_reports_errors() {
        assert_eq!(
            parse_run_info("x", "ERR docker not usable\n")
                .error
                .as_deref(),
            Some("docker not usable")
        );
        let e = parse_run_info("x", "===INSPECT===\nError: No such object: abc\n").error;
        assert!(e.unwrap().contains("No such object"));
        let mut bad = container();
        bad.id = "$(reboot)".into();
        assert!(run_info_script(&bad).is_err());
        let ok = run_info_script(&container()).unwrap();
        assert!(ok.contains("head -c 200000 /srv/shop/compose.yml"));
        let mut pt = container();
        pt.working_dir = "/data/compose/185".into();
        pt.config_files = "/data/compose/185/docker-compose.yml".into();
        assert!(
            !run_info_script(&pt).unwrap().contains("===FILE"),
            "Portainer files are not on the host"
        );
        assert!(
            !ok.contains("sudo docker"),
            "read-only fetch never prompts for sudo"
        );
    }
}
