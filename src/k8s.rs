use std::sync::Arc;

use anyhow::Result;
use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Event, Node, Pod},
};
use kube::{
    Client, Resource,
    api::{Api, LogParams},
    runtime::{WatchStreamExt, reflector, watcher},
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::{sync::mpsc, task::JoinHandle};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Overview,
    Pods,
    Deployments,
    Nodes,
    Namespaces,
    ReplicaSets,
    StatefulSets,
    DaemonSets,
    Jobs,
    CronJobs,
    ConfigMaps,
    Secrets,
    Hpas,
    Services,
    Endpoints,
    Ingresses,
    NetworkPolicies,
    Pvcs,
    Pvs,
    StorageClasses,
    ServiceAccounts,
    Roles,
    RoleBindings,
    ClusterRoles,
    ClusterRoleBindings,
}

impl ResourceKind {
    pub const ALL: [ResourceKind; 25] = [
        ResourceKind::Overview,
        ResourceKind::Nodes,
        ResourceKind::Namespaces,
        ResourceKind::Pods,
        ResourceKind::Deployments,
        ResourceKind::ReplicaSets,
        ResourceKind::StatefulSets,
        ResourceKind::DaemonSets,
        ResourceKind::Jobs,
        ResourceKind::CronJobs,
        ResourceKind::ConfigMaps,
        ResourceKind::Secrets,
        ResourceKind::Hpas,
        ResourceKind::Services,
        ResourceKind::Endpoints,
        ResourceKind::Ingresses,
        ResourceKind::NetworkPolicies,
        ResourceKind::Pvcs,
        ResourceKind::Pvs,
        ResourceKind::StorageClasses,
        ResourceKind::ServiceAccounts,
        ResourceKind::Roles,
        ResourceKind::RoleBindings,
        ResourceKind::ClusterRoles,
        ResourceKind::ClusterRoleBindings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ResourceKind::Overview => "Overview",
            ResourceKind::Pods => "Pods",
            ResourceKind::Deployments => "Deployments",
            ResourceKind::Nodes => "Nodes",
            ResourceKind::Namespaces => "Namespaces",
            ResourceKind::ReplicaSets => "ReplicaSets",
            ResourceKind::StatefulSets => "StatefulSets",
            ResourceKind::DaemonSets => "DaemonSets",
            ResourceKind::Jobs => "Jobs",
            ResourceKind::CronJobs => "CronJobs",
            ResourceKind::ConfigMaps => "ConfigMaps",
            ResourceKind::Secrets => "Secrets",
            ResourceKind::Hpas => "HPAs",
            ResourceKind::Services => "Services",
            ResourceKind::Endpoints => "Endpoints",
            ResourceKind::Ingresses => "Ingresses",
            ResourceKind::NetworkPolicies => "NetworkPolicies",
            ResourceKind::Pvcs => "PVCs",
            ResourceKind::Pvs => "PVs",
            ResourceKind::StorageClasses => "StorageClasses",
            ResourceKind::ServiceAccounts => "ServiceAccounts",
            ResourceKind::Roles => "Roles",
            ResourceKind::RoleBindings => "RoleBindings",
            ResourceKind::ClusterRoles => "ClusterRoles",
            ResourceKind::ClusterRoleBindings => "ClusterRoleBindings",
        }
    }
}

pub struct PodRow {
    pub namespace: String,
    pub name: String,
    pub phase: String,
    pub restarts: i32,
    pub containers: Vec<ContainerInfo>,
    /// "ready/total" containers, e.g. "2/3" — standard in both k9s and
    /// Freelens' pod lists.
    pub ready: String,
    pub node: String,
    pub age: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContainerStatusKind {
    Running,
    Waiting,
    Terminated,
    Unknown,
}

#[derive(Clone)]
pub struct ContainerInfo {
    pub name: String,
    pub status: ContainerStatusKind,
    pub reason: Option<String>,
    pub restarts: i32,
}

pub async fn connect() -> Result<Client> {
    Ok(Client::try_default().await?)
}

/// Per-container state, straight from `status.containerStatuses` — this is
/// what actually knows about crash-looping, unlike the pod-level `phase`
/// (Kubernetes has no CrashLoopBackOff *phase*, only a waiting *reason* on
/// the container; a crash-looping pod's phase is still just "Running").
pub fn containers_for(pod: &Pod) -> Vec<ContainerInfo> {
    let statuses = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.clone())
        .unwrap_or_default();

    statuses
        .into_iter()
        .map(|cs| {
            let (status, reason) = match cs.state {
                Some(s) if s.running.is_some() => (ContainerStatusKind::Running, None),
                Some(s) if s.waiting.is_some() => {
                    (ContainerStatusKind::Waiting, s.waiting.and_then(|w| w.reason))
                }
                Some(s) if s.terminated.is_some() => {
                    (ContainerStatusKind::Terminated, s.terminated.and_then(|t| t.reason))
                }
                _ => (ContainerStatusKind::Unknown, None),
            };
            ContainerInfo { name: cs.name, status, reason, restarts: cs.restart_count }
        })
        .collect()
}

pub fn row_for(pod: &Pod) -> PodRow {
    let namespace = pod.metadata.namespace.clone().unwrap_or_default();
    let name = pod.metadata.name.clone().unwrap_or_default();
    let status = pod.status.clone().unwrap_or_default();
    let phase = status.phase.unwrap_or_else(|| "Unknown".into());
    let container_statuses = status.container_statuses.unwrap_or_default();
    let restarts = container_statuses.iter().map(|c| c.restart_count).sum();
    let ready_count = container_statuses.iter().filter(|c| c.ready).count();
    let ready = format!("{ready_count}/{}", container_statuses.len());
    let containers = containers_for(pod);
    let node = pod.spec.as_ref().and_then(|s| s.node_name.clone()).unwrap_or_else(|| "-".into());
    let age = pod
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| humanize_age(t.0))
        .unwrap_or_else(|| "-".into());

    PodRow { namespace, name, phase, restarts, containers, ready, node, age }
}

/// A short "5m"/"3h"/"2d" style duration, matching kubectl/k9s's AGE
/// column convention (single dominant unit, not a full breakdown).
fn humanize_age(created: k8s_openapi::jiff::Timestamp) -> String {
    let secs = (k8s_openapi::jiff::Timestamp::now().as_second() - created.as_second()).max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Starts a background watch on every Pod in the cluster and keeps an
/// in-memory store up to date as events arrive — no polling, no manual
/// refresh. Returns the live-updating reader immediately; call
/// `.snapshot()` on it whenever you need the current list for rendering.
/// Reconnects with backoff automatically if the watch connection drops
/// (`WatchStreamExt::default_backoff`).
pub fn watch_pods(client: Client) -> (reflector::Store<Pod>, JoinHandle<()>) {
    let api: Api<Pod> = Api::all(client);
    let (reader, writer) = reflector::store();

    let stream = watcher(api, watcher::Config::default())
        .default_backoff()
        .reflect(writer)
        .applied_objects();

    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {
            // Nothing to do per-event — `reader.snapshot()` already
            // reflects it, since `reflect(writer)` updates the store.
        }
    });

    (reader, handle)
}

/// A stable, sorted snapshot of every pod currently in the store. Sorted
/// so a selected row index stays pointing at the same pod across ticks
/// (the store itself has no defined order).
pub fn snapshot(store: &reflector::Store<Pod>) -> Vec<Arc<Pod>> {
    let mut pods = store.state();
    pods.sort_by(|a, b| {
        let key = |p: &Arc<Pod>| {
            (
                p.metadata.namespace.clone().unwrap_or_default(),
                p.metadata.name.clone().unwrap_or_default(),
            )
        };
        key(a).cmp(&key(b))
    });
    pods
}

/// Any k8s object as a generic value tree, for the collapsible detail
/// view — works for any resource kind, not just Pods. Strips
/// `managedFields` — it's the huge, unreadable `f:` block kubectl-apply
/// machinery uses internally and isn't useful to a human looking at "what
/// is this."
pub fn manifest_value<T: Serialize>(item: &T) -> serde_yaml::Value {
    let mut value = serde_yaml::to_value(item).unwrap_or(serde_yaml::Value::Null);
    if let Some(metadata) = value.get_mut("metadata").and_then(|m| m.as_mapping_mut()) {
        metadata.remove("managedFields");
    }
    value
}

pub struct DeploymentRow {
    pub namespace: String,
    pub name: String,
    /// "ready/desired" replicas, e.g. "2/3" — kubectl/k9s convention.
    pub ready: String,
    pub up_to_date: i32,
    pub available: i32,
    pub age: String,
}

pub fn row_for_deployment(dep: &Deployment) -> DeploymentRow {
    let namespace = dep.metadata.namespace.clone().unwrap_or_default();
    let name = dep.metadata.name.clone().unwrap_or_default();
    let status = dep.status.clone().unwrap_or_default();
    let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
    let ready = format!("{}/{desired}", status.ready_replicas.unwrap_or(0));
    let up_to_date = status.updated_replicas.unwrap_or(0);
    let available = status.available_replicas.unwrap_or(0);
    let age = dep
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| humanize_age(t.0))
        .unwrap_or_else(|| "-".into());

    DeploymentRow { namespace, name, ready, up_to_date, available, age }
}

/// Same live-watch pattern as `watch_pods`, for Deployments — see there
/// for why a reflector instead of polling.
pub fn watch_deployments(client: Client) -> (reflector::Store<Deployment>, JoinHandle<()>) {
    let api: Api<Deployment> = Api::all(client);
    let (reader, writer) = reflector::store();

    let stream = watcher(api, watcher::Config::default())
        .default_backoff()
        .reflect(writer)
        .applied_objects();

    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {}
    });

    (reader, handle)
}

/// Sorted snapshot, same reasoning as `snapshot` for Pods — kept as a
/// separate small function rather than a generic one across resource
/// kinds; with just two kinds so far, a shared-trait abstraction would be
/// more machinery than the ~10 lines it'd save.
pub fn snapshot_deployments(store: &reflector::Store<Deployment>) -> Vec<Arc<Deployment>> {
    let mut deployments = store.state();
    deployments.sort_by(|a, b| {
        let key = |d: &Arc<Deployment>| {
            (
                d.metadata.namespace.clone().unwrap_or_default(),
                d.metadata.name.clone().unwrap_or_default(),
            )
        };
        key(a).cmp(&key(b))
    });
    deployments
}

/// Starts a live-following log stream for one container, sending lines
/// back over an unbounded channel as they arrive. Caller is responsible
/// for aborting the returned handle when done (e.g. when the log view is
/// closed) — otherwise the stream just keeps running against the API
/// server in the background.
pub fn stream_logs(
    client: Client,
    namespace: String,
    pod: String,
    container: String,
) -> (mpsc::UnboundedReceiver<String>, JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel();

    let handle = tokio::spawn(async move {
        let api: Api<Pod> = Api::namespaced(client, &namespace);
        let lp = LogParams {
            container: Some(container),
            follow: true,
            // Timestamps come from the API server itself, not the app —
            // more trustworthy than "when did knav happen to read this
            // line," and it's the actual diagnostic detail ("when did
            // this break") the project's whole thesis cares about.
            timestamps: true,
            ..Default::default()
        };

        let mut lines = match api.log_stream(&pod, &lp).await {
            Ok(stream) => stream.lines(),
            Err(e) => {
                let _ = tx.send(format!("[failed to start log stream: {e}]"));
                return;
            }
        };

        loop {
            match lines.next().await {
                Some(Ok(line)) => {
                    if tx.send(line).is_err() {
                        break; // receiver dropped — view was closed
                    }
                }
                Some(Err(e)) => {
                    let _ = tx.send(format!("[log stream error: {e}]"));
                    break;
                }
                None => break, // stream ended (container exited)
            }
        }
    });

    (rx, handle)
}

/// One row in the Cluster Issues panel — a Node warning condition or a
/// Kubernetes Warning-type Event, unified. Matches what Freelens's own
/// "Cluster Issues" panel shows (verified against its actual source,
/// `cluster-issues.tsx`): Node conditions + Warning events, not a
/// container-status breakdown — Warning events catch far more (failed
/// scheduling, volume mount failures, probe failures) than just
/// crash-looping containers would.
pub struct Warning {
    pub message: String,
    pub object: String,
    pub kind: String,
    pub age: String,
    pub age_secs: i64,
}

/// Node conditions worth surfacing: `Ready != True`, or any pressure/
/// unavailable condition that's `True`. A healthy node's only condition
/// is `Ready: True` — everything else here is inherently a problem
/// signal, unlike Pod phases which are fine in most states.
pub fn node_warnings(node: &Node) -> Vec<Warning> {
    let name = node.metadata.name.clone().unwrap_or_default();
    let conditions = node.status.as_ref().and_then(|s| s.conditions.clone()).unwrap_or_default();

    conditions
        .into_iter()
        .filter_map(|c| {
            let is_problem = match c.type_.as_str() {
                "Ready" => c.status != "True",
                _ => c.status == "True",
            };
            if !is_problem {
                return None;
            }
            let age = c
                .last_transition_time
                .as_ref()
                .map(|t| humanize_age(t.0))
                .unwrap_or_else(|| "-".into());
            let age_secs = c
                .last_transition_time
                .as_ref()
                .map(|t| k8s_openapi::jiff::Timestamp::now().as_second() - t.0.as_second())
                .unwrap_or(0);
            Some(Warning {
                message: c.message.unwrap_or_else(|| c.type_.clone()),
                object: name.clone(),
                kind: "Node".to_string(),
                age,
                age_secs,
            })
        })
        .collect()
}

pub fn warning_event(event: &Event) -> Option<Warning> {
    if event.type_.as_deref() != Some("Warning") {
        return None;
    }
    // For a repeated/aggregated event (the common case — the same
    // warning firing over and over, e.g. a crash loop), `series.
    // lastObservedTime` is the real "last seen" moment. `eventTime`/
    // `lastTimestamp` only capture the *first* occurrence — using those
    // alone made a warning from hours ago look current if it kept
    // recurring, which is exactly backwards for a "what needs attention
    // right now" panel.
    let timestamp = event
        .series
        .as_ref()
        .and_then(|s| s.last_observed_time.as_ref())
        .map(|t| t.0)
        .or(event.last_timestamp.as_ref().map(|t| t.0))
        .or(event.event_time.as_ref().map(|t| t.0))
        .or(event.first_timestamp.as_ref().map(|t| t.0));
    let age = timestamp.map(humanize_age).unwrap_or_else(|| "-".into());
    let age_secs =
        timestamp.map(|t| k8s_openapi::jiff::Timestamp::now().as_second() - t.as_second()).unwrap_or(0);

    Some(Warning {
        message: event.message.clone().unwrap_or_default(),
        object: event.involved_object.name.clone().unwrap_or_default(),
        kind: event.involved_object.kind.clone().unwrap_or_default(),
        age,
        age_secs,
    })
}

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

pub fn watch_events(client: Client) -> (reflector::Store<Event>, JoinHandle<()>) {
    let api: Api<Event> = Api::all(client);
    let (reader, writer) = reflector::store();
    let stream = watcher(api, watcher::Config::default()).default_backoff().reflect(writer).applied_objects();
    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {}
    });
    (reader, handle)
}

/// Resource usage, the resource-kind catalog, and the merged
/// newest-first Cluster Issues list — everything the Overview dashboard
/// needs. Deliberately has no pod/deployment/node counts of its own —
/// those live as regular entries in `catalog` instead of being
/// duplicated here.
pub struct Overview {
    pub warnings: Vec<Warning>,
    pub cpu_usage_millicores: i64,
    pub cpu_capacity_millicores: i64,
    pub memory_usage_bytes: i64,
    pub memory_capacity_bytes: i64,
    pub pod_capacity: i64,
    pub metrics_available: bool,
    /// (section title, [(kind label, live count)]) — assembled by the
    /// caller from whichever watches/pollers it's holding; this function
    /// just bundles it in alongside everything else.
    pub catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>,
}

fn node_allocatable_sum(nodes: &[Arc<Node>], key: &str, parse: impl Fn(&str) -> i64) -> i64 {
    nodes
        .iter()
        .filter_map(|n| n.status.as_ref()?.allocatable.as_ref()?.get(key))
        .map(|q| parse(&q.0))
        .sum()
}

pub fn overview(
    nodes: &[Arc<Node>],
    events: &[Arc<Event>],
    usage: Option<&crate::metrics::ClusterUsage>,
    catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>,
) -> Overview {
    let mut warnings: Vec<Warning> =
        nodes.iter().flat_map(|n| node_warnings(n)).chain(events.iter().filter_map(|e| warning_event(e))).collect();
    warnings.sort_by_key(|w| w.age_secs);

    Overview {
        warnings,
        cpu_usage_millicores: usage.map(|u| u.cpu_millicores).unwrap_or(0),
        cpu_capacity_millicores: node_allocatable_sum(nodes, "cpu", crate::metrics::parse_cpu_millicores),
        memory_usage_bytes: usage.map(|u| u.memory_bytes).unwrap_or(0),
        memory_capacity_bytes: node_allocatable_sum(nodes, "memory", crate::metrics::parse_memory_bytes),
        pod_capacity: node_allocatable_sum(nodes, "pods", |s| s.parse().unwrap_or(0)),
        metrics_available: usage.is_some(),
        catalog,
    }
}

/// A row for any resource kind that doesn't get a specialized table (i.e.
/// everything except Pods/Deployments) — just enough to list and identify
/// an object. Cluster-scoped kinds (Nodes, ClusterRoles, PVs, ...) show
/// "-" for namespace rather than getting a different column set; one
/// generic table for ~20 kinds is worth the small loss of kubectl's
/// per-kind columns.
#[derive(Clone)]
pub struct GenericRow {
    pub namespace: String,
    pub name: String,
    pub age: String,
}

pub fn generic_row<K: kube::Resource<DynamicType = ()>>(item: &K) -> GenericRow {
    let meta = item.meta();
    let namespace = meta.namespace.clone().unwrap_or_else(|| "-".into());
    let name = meta.name.clone().unwrap_or_default();
    let age = meta.creation_timestamp.as_ref().map(|t| humanize_age(t.0)).unwrap_or_else(|| "-".into());
    GenericRow { namespace, name, age }
}

/// Same live-watch pattern as `watch_pods`/`watch_deployments`, generic
/// over any typed k8s-openapi resource — used for every catalog kind that
/// doesn't need specialized fields.
pub fn watch_generic<K>(client: Client) -> (reflector::Store<K>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let api: Api<K> = Api::all(client);
    let (reader, writer) = reflector::store();
    let stream = watcher(api, watcher::Config::default()).default_backoff().reflect(writer).applied_objects();
    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {}
    });
    (reader, handle)
}

/// Sorted snapshot, generic over any typed k8s-openapi resource — same
/// reasoning as `snapshot`/`snapshot_deployments`.
pub fn snapshot_generic<K: Resource<DynamicType = ()> + Clone>(store: &reflector::Store<K>) -> Vec<Arc<K>> {
    let mut items = store.state();
    items.sort_by(|a, b| {
        let key = |x: &Arc<K>| (x.meta().namespace.clone().unwrap_or_default(), x.meta().name.clone().unwrap_or_default());
        key(a).cmp(&key(b))
    });
    items
}

/// Type-erased handle to a live-watched resource kind's store — lets
/// `Catalog` hold ~20 different `K`s in one `Vec` and treat them
/// uniformly (count for the Overview tile, rows for the list view, a
/// single object's manifest for the spec view) without a match arm per
/// kind at every call site.
pub trait CatalogKind: Send + Sync {
    fn count(&self) -> usize;
    fn rows(&self) -> Vec<GenericRow>;
    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value>;
}

pub struct WatchedKind<K: Resource<DynamicType = ()> + Clone + 'static> {
    store: reflector::Store<K>,
}

impl<K: Resource<DynamicType = ()> + Clone + 'static> WatchedKind<K> {
    pub fn from_store(store: reflector::Store<K>) -> Self {
        WatchedKind { store }
    }
}

impl<K> CatalogKind for WatchedKind<K>
where
    K: Resource<DynamicType = ()> + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    fn count(&self) -> usize {
        self.store.state().len()
    }

    fn rows(&self) -> Vec<GenericRow> {
        snapshot_generic(&self.store).iter().map(|item| generic_row(item.as_ref())).collect()
    }

    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value> {
        snapshot_generic(&self.store).get(index).map(|item| manifest_value(item.as_ref()))
    }
}

/// Spawns a live watch for kind `K` and boxes it as a `CatalogKind` —
/// the one-liner most Catalog entries use.
pub fn watch_kind<K>(client: Client) -> (Box<dyn CatalogKind>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let (store, handle) = watch_generic::<K>(client);
    (Box::new(WatchedKind::from_store(store)), handle)
}
