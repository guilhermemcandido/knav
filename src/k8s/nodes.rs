
use futures::StreamExt;
use k8s_openapi::api::core::v1::Node;
use kube::{
    Client,
    api::Api,
    runtime::{WatchStreamExt, reflector, watcher},
};
use tokio::task::JoinHandle;

use super::*;

pub fn watch_nodes(client: Client) -> (reflector::Store<Node>, JoinHandle<()>) {
    let api: Api<Node> = Api::all(client);
    let (reader, writer) = reflector::store();
    let stream = watcher(api, watcher::Config::default()).default_backoff().reflect(writer).applied_objects();
    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {}
    });
    (reader, handle)
}

/// One node's own capacity — the Node detail view's gauges need a single
/// node's numbers, not the cluster-wide sum `node_allocatable_sum` gives
/// the Overview.
pub struct NodeCapacity {
    pub cpu_millicores: i64,
    pub memory_bytes: i64,
    pub pods: i64,
}

pub fn node_capacity(node: &Node) -> NodeCapacity {
    let allocatable = node.status.as_ref().and_then(|s| s.allocatable.as_ref());
    let get = |key: &str, parse: &dyn Fn(&str) -> i64| -> i64 {
        allocatable.and_then(|a| a.get(key)).map(|q| parse(&q.0)).unwrap_or(0)
    };
    NodeCapacity {
        cpu_millicores: get("cpu", &crate::k8s::metrics::parse_cpu_millicores),
        memory_bytes: get("memory", &crate::k8s::metrics::parse_memory_bytes),
        pods: get("pods", &|s| s.parse().unwrap_or(0)),
    }
}

/// A row for the Nodes list — unlike the ~20 generic Namespace/Name/Age
/// kinds, Nodes gets its own specialized columns so usage is visible
/// right there in the list, not just after drilling into one
/// (`cpu_millicores`/`memory_bytes` are `None` when metrics-server isn't
/// installed, same "unavailable" fallback as everywhere else).
pub struct NodeRow {
    pub name: String,
    pub ready: bool,
    /// `false` when the node is cordoned (`spec.unschedulable`) — kubectl
    /// shows this by appending ",SchedulingDisabled" to STATUS rather than
    /// a separate column, the same convention `draw_nodes_table` follows.
    pub schedulable: bool,
    pub roles: String,
    pub version: String,
    pub cpu_millicores: Option<i64>,
    pub cpu_capacity: i64,
    pub memory_bytes: Option<i64>,
    pub memory_capacity: i64,
    pub pod_count: usize,
    pub pod_capacity: i64,
    pub taints: usize,
    /// Address, OS, kernel and runtime, for the wide view.
    pub internal_ip: String,
    pub os_image: String,
    pub kernel: String,
    pub runtime: String,
    pub age: String,
    pub age_secs: i64,
}

/// The `node-role.kubernetes.io/<role>` label convention kubectl itself
/// reads for the ROLES column — there's no dedicated API field for this,
/// just labels a role-assigning controller (or `kubeadm`/`k3s` at join
/// time) sets.
fn node_roles(node: &Node) -> String {
    let mut roles: Vec<&str> = node
        .metadata
        .labels
        .iter()
        .flatten()
        .filter_map(|(k, _)| k.strip_prefix("node-role.kubernetes.io/"))
        .filter(|r| !r.is_empty())
        .collect();
    roles.sort_unstable();
    roles.dedup();
    if roles.is_empty() { "<none>".to_string() } else { roles.join(",") }
}

pub fn node_row(node: &Node, usage: Option<&crate::k8s::metrics::NodeUsage>, pod_count: usize) -> NodeRow {
    let name = node.metadata.name.clone().unwrap_or_default();
    let ready = node
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
        .map(|c| c.status == "True")
        .unwrap_or(false);
    let schedulable = !node.spec.as_ref().and_then(|s| s.unschedulable).unwrap_or(false);
    let version = node
        .status
        .as_ref()
        .and_then(|s| s.node_info.as_ref())
        .map(|i| i.kubelet_version.clone())
        .unwrap_or_else(|| "-".into());
    let capacity = node_capacity(node);
    let age = node.metadata.creation_timestamp.as_ref().map(|t| humanize_age(t.0)).unwrap_or_else(|| "-".into());

    let info = node.status.as_ref().and_then(|s| s.node_info.as_ref());
    let internal_ip = node
        .status
        .as_ref()
        .and_then(|s| s.addresses.as_ref())
        .and_then(|a| a.iter().find(|a| a.type_ == "InternalIP"))
        .map(|a| a.address.clone())
        .unwrap_or_else(|| "-".into());
    NodeRow {
        internal_ip,
        os_image: info.map(|i| i.os_image.clone()).unwrap_or_else(|| "-".into()),
        kernel: info.map(|i| i.kernel_version.clone()).unwrap_or_else(|| "-".into()),
        runtime: info.map(|i| i.container_runtime_version.clone()).unwrap_or_else(|| "-".into()),
        name,
        ready,
        schedulable,
        roles: node_roles(node),
        version,
        cpu_millicores: usage.map(|u| u.cpu_millicores),
        cpu_capacity: capacity.cpu_millicores,
        memory_bytes: usage.map(|u| u.memory_bytes),
        memory_capacity: capacity.memory_bytes,
        pod_count,
        pod_capacity: capacity.pods,
        taints: node.spec.as_ref().and_then(|s| s.taints.as_ref()).map_or(0, Vec::len),
        age,
        age_secs: age_seconds(node.metadata.creation_timestamp.as_ref()),
    }
}

/// One node condition, unfiltered — unlike `node_warnings` (which only
/// surfaces *problem* conditions for the Cluster Issues panel), the node
/// detail view is a diagnostic screen that should show the full picture,
/// healthy conditions included.
pub struct NodeConditionRow {
    pub type_: String,
    pub status: String,
    pub reason: String,
}

/// Everything about a node Freelens shows on its own node detail page
/// beyond what's already in `NodeRow`/the CPU-Memory-Pods gauges: full
/// condition list, taints, schedulability, addresses, and the
/// OS/kernel/runtime/kubelet versions from `status.nodeInfo`.
pub struct NodeDetailInfo {
    pub roles: String,
    pub schedulable: bool,
    pub kubelet_version: String,
    pub os_image: String,
    pub kernel_version: String,
    pub container_runtime: String,
    pub internal_ip: String,
    pub external_ip: String,
    /// Pre-formatted as `key=value:Effect` (or `key:Effect` with no
    /// value) — kubectl's own taint display convention.
    pub taints: Vec<String>,
    pub conditions: Vec<NodeConditionRow>,
}

pub fn node_detail_info(node: &Node) -> NodeDetailInfo {
    let status = node.status.as_ref();
    let node_info = status.and_then(|s| s.node_info.as_ref());
    let addresses = status.and_then(|s| s.addresses.as_ref());
    let find_addr = |type_: &str| {
        addresses.and_then(|addrs| addrs.iter().find(|a| a.type_ == type_)).map(|a| a.address.clone()).unwrap_or_else(|| "-".into())
    };
    let taints = node
        .spec
        .as_ref()
        .and_then(|s| s.taints.as_ref())
        .map(|taints| {
            taints
                .iter()
                .map(|t| match &t.value {
                    Some(v) if !v.is_empty() => format!("{}={}:{}", t.key, v, t.effect),
                    _ => format!("{}:{}", t.key, t.effect),
                })
                .collect()
        })
        .unwrap_or_default();
    let conditions = status
        .and_then(|s| s.conditions.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|c| NodeConditionRow { type_: c.type_, status: c.status, reason: c.reason.unwrap_or_else(|| "-".into()) })
        .collect();

    NodeDetailInfo {
        roles: node_roles(node),
        schedulable: !node.spec.as_ref().and_then(|s| s.unschedulable).unwrap_or(false),
        kubelet_version: node_info.map(|i| i.kubelet_version.clone()).unwrap_or_else(|| "-".into()),
        os_image: node_info.map(|i| i.os_image.clone()).unwrap_or_else(|| "-".into()),
        kernel_version: node_info.map(|i| i.kernel_version.clone()).unwrap_or_else(|| "-".into()),
        container_runtime: node_info.map(|i| i.container_runtime_version.clone()).unwrap_or_else(|| "-".into()),
        internal_ip: find_addr("InternalIP"),
        external_ip: find_addr("ExternalIP"),
        taints,
        conditions,
    }
}
