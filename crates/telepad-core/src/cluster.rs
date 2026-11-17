//! Docker Swarm and Kubernetes, as seen from one host: model and parsers for
//! the `===SWARM===` and `===KUBE===` sections of `script.sh`. Pure, no I/O.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Cluster {
    pub swarm: Option<Swarm>,
    pub kube: Option<Kube>,
    /// kubectl is installed but the cluster cannot be read (no kubeconfig access).
    pub kube_error: Option<String>,
}

impl Cluster {
    /// Something in the cluster needs attention (only managers / kubectl hosts know).
    pub fn degraded(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(s) = &self.swarm {
            v.extend(s.problems());
        }
        if let Some(k) = &self.kube {
            v.extend(k.problems());
        }
        v
    }

    /// Short badge for the overview: "Swarm manager", "Swarm worker", "k8s".
    pub fn badge(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(s) = &self.swarm {
            parts.push(if s.manager {
                "Swarm manager"
            } else {
                "Swarm worker"
            });
        }
        if self.kube.is_some() || self.kube_error.is_some() {
            parts.push("k8s");
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Identifies the cluster across hosts, so several managers show it once.
    pub fn key(&self) -> Option<String> {
        let swarm = self
            .swarm
            .as_ref()
            .filter(|s| s.manager && !s.cluster_id.is_empty())
            .map(|s| format!("swarm:{}", s.cluster_id));
        let kube = self
            .kube
            .as_ref()
            .filter(|k| !k.cluster_id.is_empty())
            .map(|k| format!("k8s:{}", k.cluster_id));
        swarm.or(kube)
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Swarm {
    pub manager: bool,
    pub cluster_id: String,
    pub node_id: String,
    pub nodes: Vec<SwarmNode>,
    /// (stack name, number of services)
    pub stacks: Vec<(String, u32)>,
    pub services: Vec<SwarmService>,
    /// Tasks that should run but do not (pending, starting, failed...).
    pub failing_tasks: Vec<SwarmTask>,
}

impl Swarm {
    pub fn problems(&self) -> Vec<String> {
        let mut v = Vec::new();
        for n in &self.nodes {
            if n.status != "Ready" {
                v.push(format!("node {} is {}", n.hostname, n.status));
            }
        }
        for s in &self.services {
            if s.degraded() {
                v.push(format!(
                    "service {} has {}/{} replicas",
                    s.name, s.running, s.desired
                ));
            }
        }
        v
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct SwarmNode {
    pub id: String,
    pub hostname: String,
    /// Ready / Down / Unknown
    pub status: String,
    /// Active / Pause / Drain
    pub availability: String,
    /// Leader / Reachable / Unreachable, empty for workers.
    pub manager_status: String,
    pub engine: String,
    pub is_self: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct SwarmService {
    pub id: String,
    pub name: String,
    /// Stack it belongs to (`<stack>_<service>`), empty if created by hand.
    pub stack: String,
    pub mode: String,
    pub image: String,
    pub ports: String,
    pub running: u32,
    pub desired: u32,
}

impl SwarmService {
    pub fn degraded(&self) -> bool {
        self.running < self.desired
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct SwarmTask {
    pub name: String,
    pub node: String,
    pub state: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Kube {
    /// `kubectl`, `k3s kubectl` or `microk8s kubectl`.
    pub tool: String,
    /// UID of the kube-system namespace (stable per cluster).
    pub cluster_id: String,
    pub nodes: Vec<KubeNode>,
    pub workloads: Vec<Workload>,
    pub pods_total: u32,
    /// Not running, waiting (CrashLoopBackOff, ImagePullBackOff...) or restarting a lot.
    pub problem_pods: Vec<Pod>,
}

impl Kube {
    pub fn namespaces(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .workloads
            .iter()
            .map(|w| w.namespace.as_str())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    pub fn problems(&self) -> Vec<String> {
        let mut v = Vec::new();
        for n in &self.nodes {
            if !n.ready {
                v.push(format!("node {} is not ready", n.name));
            }
        }
        for w in &self.workloads {
            if w.degraded() {
                v.push(format!(
                    "{} {}/{} has {}/{} ready",
                    w.kind.singular(),
                    w.namespace,
                    w.name,
                    w.ready,
                    w.desired
                ));
            }
        }
        if !self.problem_pods.is_empty() {
            v.push(format!("{} pod(s) with problems", self.problem_pods.len()));
        }
        v
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct KubeNode {
    pub name: String,
    pub ready: bool,
    pub version: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub enum WorkloadKind {
    #[default]
    Deployment,
    StatefulSet,
    DaemonSet,
}

impl WorkloadKind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "deployments" => Some(Self::Deployment),
            "statefulsets" => Some(Self::StatefulSet),
            "daemonsets" => Some(Self::DaemonSet),
            _ => None,
        }
    }

    /// kubectl resource name (`deployment/<name>`).
    pub fn singular(self) -> &'static str {
        match self {
            Self::Deployment => "deployment",
            Self::StatefulSet => "statefulset",
            Self::DaemonSet => "daemonset",
        }
    }

    /// DaemonSets have one pod per node and cannot be scaled.
    pub fn scalable(self) -> bool {
        self != Self::DaemonSet
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Workload {
    pub kind: WorkloadKind,
    pub namespace: String,
    pub name: String,
    pub ready: u32,
    pub desired: u32,
}

impl Workload {
    pub fn degraded(&self) -> bool {
        self.ready < self.desired
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Pod {
    pub namespace: String,
    pub name: String,
    pub phase: String,
    pub restarts: u32,
    /// Waiting reason, e.g. CrashLoopBackOff.
    pub reason: String,
}

fn field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// `1/3` or `1/3 (max 2 per node)` → (1, 3).
fn replicas(s: &str) -> (u32, u32) {
    let first = s.split_whitespace().next().unwrap_or("");
    let (a, b) = first.split_once('/').unwrap_or(("0", "0"));
    (a.parse().unwrap_or(0), b.parse().unwrap_or(0))
}

/// `===SWARM===`: `info=<state>|<manager>|<cluster id>|<node id>`, then
/// `node=`, `stack=`, `service=`, `task=` lines with `docker ... --format '{{json .}}'`.
pub fn parse_swarm(lines: &[String]) -> Option<Swarm> {
    let info = lines.iter().find_map(|l| l.strip_prefix("info="))?;
    let mut it = info.split('|');
    if it.next()? != "active" {
        return None;
    }
    let mut sw = Swarm {
        manager: it.next() == Some("true"),
        cluster_id: it.next().unwrap_or("").to_string(),
        node_id: it.next().unwrap_or("").to_string(),
        ..Default::default()
    };
    for line in lines {
        let Some((kind, json)) = line.split_once('=') else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(json) else {
            continue;
        };
        match kind {
            "node" => sw.nodes.push(SwarmNode {
                id: field(&v, "ID").into(),
                hostname: field(&v, "Hostname").into(),
                status: field(&v, "Status").into(),
                availability: field(&v, "Availability").into(),
                manager_status: field(&v, "ManagerStatus").into(),
                engine: field(&v, "EngineVersion").into(),
                is_self: v.get("Self").and_then(Value::as_bool).unwrap_or(false),
            }),
            "stack" => sw.stacks.push((
                field(&v, "Name").into(),
                field(&v, "Services").parse().unwrap_or(0),
            )),
            "service" => {
                let (running, desired) = replicas(field(&v, "Replicas"));
                sw.services.push(SwarmService {
                    id: field(&v, "ID").into(),
                    name: field(&v, "Name").into(),
                    mode: field(&v, "Mode").into(),
                    image: field(&v, "Image").into(),
                    ports: field(&v, "Ports").into(),
                    running,
                    desired,
                    stack: String::new(),
                })
            }
            "task" => sw.failing_tasks.push(SwarmTask {
                name: field(&v, "Name").into(),
                node: field(&v, "Node").into(),
                state: field(&v, "CurrentState").into(),
                error: field(&v, "Error").into(),
            }),
            _ => {}
        }
    }
    // `docker stack deploy` names services `<stack>_<service>`; longest stack name wins.
    let mut stacks: Vec<&str> = sw.stacks.iter().map(|(n, _)| n.as_str()).collect();
    stacks.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for s in &mut sw.services {
        if let Some(st) = stacks.iter().find(|st| {
            s.name
                .strip_prefix(**st)
                .is_some_and(|r| r.starts_with('_'))
        }) {
            s.stack = st.to_string();
        }
    }
    Some(sw)
}

/// `===KUBE===`: `ERR ...` (kubectl without access), or `tool=`, `id=`,
/// `node=name|Ready status|version`, `wl=kind|ns|name|ready|desired`,
/// `pod=ns|name|phase|restart counts|waiting reasons` (problem pods only), `pods=N`.
pub fn parse_kube(lines: &[String]) -> (Option<Kube>, Option<String>) {
    let err = lines
        .iter()
        .find_map(|l| l.strip_prefix("ERR "))
        .map(String::from);
    let Some(tool) = lines.iter().find_map(|l| l.strip_prefix("tool=")) else {
        return (None, err);
    };
    let mut k = Kube {
        tool: tool.to_string(),
        ..Default::default()
    };
    let num = |s: &str| s.trim().parse::<u32>().unwrap_or(0);
    for line in lines {
        let Some((kind, rest)) = line.split_once('=') else {
            continue;
        };
        let f: Vec<&str> = rest.split('|').collect();
        match (kind, f.as_slice()) {
            ("id", [id]) => k.cluster_id = id.to_string(),
            ("node", [name, ready, version, ..]) => k.nodes.push(KubeNode {
                name: name.to_string(),
                ready: *ready == "True",
                version: version.to_string(),
            }),
            ("wl", [kind, ns, name, ready, desired, ..]) => {
                if let Some(kind) = WorkloadKind::parse(kind) {
                    k.workloads.push(Workload {
                        kind,
                        namespace: ns.to_string(),
                        name: name.to_string(),
                        ready: num(ready),
                        desired: num(desired),
                    });
                }
            }
            ("pod", [ns, name, phase, restarts, reasons, ..]) => k.problem_pods.push(Pod {
                namespace: ns.to_string(),
                name: name.to_string(),
                phase: phase.to_string(),
                restarts: restarts.split_whitespace().map(num).sum(),
                reason: reasons.split_whitespace().next().unwrap_or("").to_string(),
            }),
            ("pods", [n]) => k.pods_total = num(n),
            _ => {}
        }
    }
    (Some(k), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::sections;

    fn fixture(name: &str) -> String {
        let p = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(p).unwrap_or_default()
    }

    #[test]
    fn swarm_manager_fixture() {
        let s = sections(&fixture("swarm_manager.txt"));
        let sw = parse_swarm(&s["SWARM"]).expect("swarm");
        assert!(sw.manager);
        assert_eq!(sw.cluster_id, "q1w2e3r4t5y6u7i8o9p0asdfg");
        assert_eq!(sw.nodes.len(), 3);
        assert!(sw.nodes[0].is_self && sw.nodes[0].manager_status == "Leader");
        assert_eq!(sw.nodes[2].status, "Down");
        assert_eq!(
            sw.stacks,
            [("shop".to_string(), 2), ("shop_api".to_string(), 1)]
        );
        let stacks: Vec<&str> = sw.services.iter().map(|s| s.stack.as_str()).collect();
        assert_eq!(
            stacks,
            ["shop", "shop", "shop_api", ""],
            "longest prefix wins"
        );
        assert_eq!((sw.services[1].running, sw.services[1].desired), (1, 3));
        assert_eq!(sw.services[3].desired, 2, "global with constraints");
        assert_eq!(sw.failing_tasks.len(), 1);
        assert!(sw.failing_tasks[0].error.contains("no suitable node"));
        assert_eq!(
            sw.problems(),
            [
                "node worker-02 is Down",
                "service shop_web has 1/3 replicas"
            ]
        );
        let c = Cluster {
            swarm: Some(sw),
            ..Default::default()
        };
        assert_eq!(c.badge().as_deref(), Some("Swarm manager"));
        assert_eq!(c.key().as_deref(), Some("swarm:q1w2e3r4t5y6u7i8o9p0asdfg"));
    }

    #[test]
    fn swarm_inactive_or_worker() {
        let lines = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_swarm(&lines(&["info=inactive|false||"])), None);
        assert_eq!(parse_swarm(&[]), None);
        let w = parse_swarm(&lines(&["info=active|false||n0d3"])).expect("worker");
        assert!(!w.manager && w.nodes.is_empty());
        let c = Cluster {
            swarm: Some(w),
            ..Default::default()
        };
        assert_eq!(c.badge().as_deref(), Some("Swarm worker"));
        assert_eq!(c.key(), None, "workers cannot see the cluster");
    }

    #[test]
    fn kube_fixture() {
        let s = sections(&fixture("kube_k3s.txt"));
        let (k, err) = parse_kube(&s["KUBE"]);
        assert_eq!(err, None);
        let k = k.expect("kube");
        assert_eq!(k.tool, "k3s kubectl");
        assert_eq!(k.cluster_id, "6b3f0c1e-2d4a-4b5c-9e8f-7a6b5c4d3e2f");
        assert_eq!(k.nodes.len(), 2);
        assert!(k.nodes[0].ready && !k.nodes[1].ready);
        assert_eq!(k.workloads.len(), 4);
        assert_eq!(k.namespaces(), ["kube-system", "shop"]);
        assert_eq!(k.pods_total, 14);
        assert_eq!(k.problem_pods.len(), 2);
        assert_eq!(k.problem_pods[0].reason, "CrashLoopBackOff");
        assert_eq!(k.problem_pods[0].restarts, 42);
        assert_eq!(
            k.problems(),
            [
                "node edge-02 is not ready",
                "deployment shop/web has 1/2 ready",
                "2 pod(s) with problems"
            ]
        );
    }

    #[test]
    fn kube_without_access() {
        let lines = vec!["ERR kubectl found but no access to the cluster".to_string()];
        let (k, err) = parse_kube(&lines);
        assert!(k.is_none());
        assert!(err.unwrap().contains("no access"));
        let c = Cluster {
            kube_error: Some("x".into()),
            ..Default::default()
        };
        assert_eq!(c.badge().as_deref(), Some("k8s"));
    }
}
