use std::sync::Arc;

use anyhow::Result;
use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Event, Node, Pod},
};
use kube::{
    Client, Resource,
    api::{Api, ApiResource, DynamicObject, LogParams},
    runtime::{WatchStreamExt, reflector, watcher},
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::{sync::mpsc, task::JoinHandle};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// The Custom Resources picker — every discovered CRD kind
    /// (group/kind/scope), not object instances, not filtered by group.
    CustomResourceList,
    /// Same picker, filtered to one API group — the group string is
    /// already `&'static str` (leaked once at discovery, see
    /// `discover_crds`), so no extra registry lookup is needed here
    /// either, same reasoning as `CustomResource`'s label.
    CustomResourceGroup(&'static str),
    /// One specific CRD kind's instances — `usize` indexes into
    /// `Catalog`'s discovered CRD list, the label is carried alongside
    /// since it's a runtime string, not one of this enum's compile-time
    /// variants like every other kind's `label()`.
    CustomResource(usize, &'static str),
}

impl ResourceKind {
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
            ResourceKind::CustomResourceList => "Custom Resources",
            ResourceKind::CustomResourceGroup(group) => group,
            ResourceKind::CustomResource(_, label) => label,
        }
    }

    /// Whether this kind's objects live in a namespace — false for the
    /// cluster-scoped built-ins, where a row has no namespace to act on.
    pub fn is_namespaced(self) -> bool {
        !matches!(
            self,
            ResourceKind::Overview
                | ResourceKind::Nodes
                | ResourceKind::Pvs
                | ResourceKind::StorageClasses
                | ResourceKind::ClusterRoles
                | ResourceKind::ClusterRoleBindings
                | ResourceKind::CustomResourceList
                | ResourceKind::CustomResourceGroup(_)
        )
    }

    /// What Enter on a row of this kind drills into, if anything: a
    /// Deployment's ReplicaSets, a ReplicaSet's/StatefulSet's/
    /// DaemonSet's/Job's/Service's Pods, a CronJob's Jobs, a Namespace's
    /// Pods (and every later query narrowed to that namespace). Pods and
    /// Nodes drill too, but into their own popups rather than a list.
    pub fn drill_target(self) -> Option<ResourceKind> {
        match self {
            ResourceKind::Deployments => Some(ResourceKind::ReplicaSets),
            ResourceKind::ReplicaSets
            | ResourceKind::StatefulSets
            | ResourceKind::DaemonSets
            | ResourceKind::Jobs
            | ResourceKind::Services
            | ResourceKind::Namespaces => Some(ResourceKind::Pods),
            ResourceKind::CronJobs => Some(ResourceKind::Jobs),
            _ => None,
        }
    }

    /// Whether Enter on a row of this kind opens its manifest (`d`):
    /// everything that doesn't drill somewhere else.
    pub fn opens_spec_on_enter(self) -> bool {
        self.drill_target().is_none()
            && !matches!(
                self,
                ResourceKind::Overview
                    | ResourceKind::Pods
                    | ResourceKind::Nodes
                    | ResourceKind::CustomResourceList
                    | ResourceKind::CustomResourceGroup(_)
            )
    }

    /// The reverse of `label()` — for the fixed, compile-time-known kinds
    /// only (never `CustomResource`, which needs a live index and can't
    /// be reconstructed from its label alone). The join key between the
    /// (label, count) tuples the Overview catalog/menu render and the
    /// enum `current_kind` actually switches on.
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "Pods" => Some(ResourceKind::Pods),
            "Deployments" => Some(ResourceKind::Deployments),
            "Nodes" => Some(ResourceKind::Nodes),
            "Namespaces" => Some(ResourceKind::Namespaces),
            "ReplicaSets" => Some(ResourceKind::ReplicaSets),
            "StatefulSets" => Some(ResourceKind::StatefulSets),
            "DaemonSets" => Some(ResourceKind::DaemonSets),
            "Jobs" => Some(ResourceKind::Jobs),
            "CronJobs" => Some(ResourceKind::CronJobs),
            "ConfigMaps" => Some(ResourceKind::ConfigMaps),
            "Secrets" => Some(ResourceKind::Secrets),
            "HPAs" => Some(ResourceKind::Hpas),
            "Services" => Some(ResourceKind::Services),
            "Endpoints" => Some(ResourceKind::Endpoints),
            "Ingresses" => Some(ResourceKind::Ingresses),
            "NetworkPolicies" => Some(ResourceKind::NetworkPolicies),
            "PVCs" => Some(ResourceKind::Pvcs),
            "PVs" => Some(ResourceKind::Pvs),
            "StorageClasses" => Some(ResourceKind::StorageClasses),
            "ServiceAccounts" => Some(ResourceKind::ServiceAccounts),
            "Roles" => Some(ResourceKind::Roles),
            "RoleBindings" => Some(ResourceKind::RoleBindings),
            "ClusterRoles" => Some(ResourceKind::ClusterRoles),
            "ClusterRoleBindings" => Some(ResourceKind::ClusterRoleBindings),
            "Custom Resources" => Some(ResourceKind::CustomResourceList),
            _ => None,
        }
    }

    /// Resolves a `:command` (already lowercased/trimmed by the caller)
    /// to the kind it switches to — the full lowercase name (spaces
    /// removed) always works, plus k9s-style short aliases for the ones
    /// worth typing quickly. Returns `None` for anything unrecognized;
    /// the caller just no-ops rather than erroring, same as an unknown
    /// command in a shell alias you half-remember.
    pub fn from_command(cmd: &str) -> Option<Self> {
        match cmd {
            "overview" | "home" => Some(ResourceKind::Overview),
            "pods" | "pod" | "po" => Some(ResourceKind::Pods),
            "deployments" | "deployment" | "deploy" | "dep" => Some(ResourceKind::Deployments),
            "nodes" | "node" | "no" => Some(ResourceKind::Nodes),
            "namespaces" | "namespace" | "ns" => Some(ResourceKind::Namespaces),
            "replicasets" | "replicaset" | "rs" => Some(ResourceKind::ReplicaSets),
            "statefulsets" | "statefulset" | "sts" => Some(ResourceKind::StatefulSets),
            "daemonsets" | "daemonset" | "ds" => Some(ResourceKind::DaemonSets),
            "jobs" | "job" => Some(ResourceKind::Jobs),
            "cronjobs" | "cronjob" | "cj" => Some(ResourceKind::CronJobs),
            "configmaps" | "configmap" | "cm" => Some(ResourceKind::ConfigMaps),
            "secrets" | "secret" | "sec" => Some(ResourceKind::Secrets),
            "hpas" | "hpa" => Some(ResourceKind::Hpas),
            "services" | "service" | "svc" => Some(ResourceKind::Services),
            "endpoints" | "endpoint" | "ep" => Some(ResourceKind::Endpoints),
            "ingresses" | "ingress" | "ing" => Some(ResourceKind::Ingresses),
            "networkpolicies" | "networkpolicy" | "netpol" => Some(ResourceKind::NetworkPolicies),
            "pvcs" | "pvc" => Some(ResourceKind::Pvcs),
            "pvs" | "pv" => Some(ResourceKind::Pvs),
            "storageclasses" | "storageclass" | "sc" => Some(ResourceKind::StorageClasses),
            "serviceaccounts" | "serviceaccount" | "sa" => Some(ResourceKind::ServiceAccounts),
            "roles" | "role" => Some(ResourceKind::Roles),
            "rolebindings" | "rolebinding" | "rb" => Some(ResourceKind::RoleBindings),
            "clusterroles" | "clusterrole" | "cr" => Some(ResourceKind::ClusterRoles),
            "clusterrolebindings" | "clusterrolebinding" | "crb" => Some(ResourceKind::ClusterRoleBindings),
            "customresources" | "customresource" | "crds" | "crd" => Some(ResourceKind::CustomResourceList),
            _ => None,
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

/// One kubeconfig context, for the cluster picker — just enough to list
/// and identify it. `cluster`/`namespace` are shown alongside the name
/// since two contexts can share a name pattern (e.g. "prod-us"/"prod-eu")
/// but point at very different clusters, which the name alone wouldn't
/// make obvious.
pub struct ContextInfo {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub is_current: bool,
}

/// Every context in the kubeconfig (`$KUBECONFIG` or `~/.kube/config`,
/// same resolution `kube` itself uses) — the picker's whole candidate
/// list. Ordering matches the file, same as `kubectl config get-contexts`.
pub fn list_contexts() -> Result<Vec<ContextInfo>> {
    let kubeconfig = kube::config::Kubeconfig::read()?;
    let current = kubeconfig.current_context.clone();
    Ok(kubeconfig
        .contexts
        .into_iter()
        .map(|c| {
            let cluster = c.context.as_ref().map(|ctx| ctx.cluster.clone()).unwrap_or_default();
            let user = c.context.as_ref().and_then(|ctx| ctx.user.clone()).unwrap_or_default();
            let is_current = current.as_deref() == Some(c.name.as_str());
            ContextInfo { name: c.name, cluster, user, is_current }
        })
        .collect())
}

/// Connects to a specific kubeconfig context by name, or (`None`) whatever
/// `kube` itself would infer — in-cluster config if running inside a pod,
/// else the kubeconfig's own `current-context`. The same "infer" path
/// `connect` already used, just exposed so a chosen context can override it.
pub async fn connect_to_context(context: Option<&str>) -> Result<Client> {
    let config = match context {
        Some(name) => {
            kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions { context: Some(name.to_string()), ..Default::default() })
                .await?
        }
        None => kube::Config::infer().await?,
    };
    Ok(Client::try_from(config)?)
}

/// Fails fast, with a readable message, if the API server can't be
/// reached — without this, an unreachable cluster just hangs forever in
/// the reflectors' initial list (which retry silently), and knav never
/// draws anything. `context` is only for the message. Returns the
/// server's version (`v1.35.5+k3s1`).
pub async fn ensure_reachable(client: &Client, context: Option<&str>) -> Result<String> {
    let label = match context {
        Some(name) => name.to_string(),
        None => list_contexts()
            .ok()
            .and_then(|c| c.into_iter().find(|c| c.is_current).map(|c| c.name))
            .unwrap_or_else(|| "the current context".to_string()),
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), client.apiserver_version()).await {
        Ok(Ok(info)) => Ok(info.git_version),
        Ok(Err(e)) => anyhow::bail!(
            "can't reach cluster '{label}': {e}\n\nIs it running? Try another context with `knav -c <name>`, or set `startup.mode = \"menu\"` to pick one at launch."
        ),
        Err(_) => anyhow::bail!(
            "can't reach cluster '{label}': timed out after 5s\n\nIs it running? Try another context with `knav -c <name>`, or set `startup.mode = \"menu\"` to pick one at launch."
        ),
    }
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

/// Kubernetes only defines two event severities — there's no distinct
/// "Error" type, just `Normal`/`Warning` — so filtering/coloring can only
/// ever be grounded in these two, not a fabricated third bucket.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EventSeverity {
    Normal,
    Warning,
}

/// Which events the dedicated Events browser shows — cycled with a/w/n.
/// Only `Normal`/`Warning` exist because that's all Kubernetes itself
/// defines for an Event's `type`; there's no separate "Error" to filter
/// on.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum EventFilter {
    #[default]
    All,
    Warnings,
    Normal,
}

impl EventFilter {
    pub fn label(self) -> &'static str {
        match self {
            EventFilter::All => "all",
            EventFilter::Warnings => "warnings",
            EventFilter::Normal => "normal",
        }
    }

    pub fn matches(self, entry: &EventEntry) -> bool {
        match self {
            EventFilter::All => true,
            EventFilter::Warnings => entry.severity == EventSeverity::Warning,
            EventFilter::Normal => entry.severity == EventSeverity::Normal,
        }
    }
}

/// One row in the Events feed: every cluster Event in chronological
/// order (not filtered to Warnings — see `EventFilter` for how the
/// dedicated Events browser narrows that down), plus each node's own
/// problem conditions folded in as synthetic Warning-severity entries
/// (there's no Event object for "this node has been NotReady for an
/// hour," but it's exactly the kind of thing this feed should surface).
#[derive(Clone)]
pub struct EventEntry {
    pub message: String,
    pub reason: String,
    pub object: String,
    pub kind: String,
    pub age: String,
    pub age_secs: i64,
    pub severity: EventSeverity,
}

/// Node conditions worth surfacing: `Ready != True`, or any pressure/
/// unavailable condition that's `True`. A healthy node's only condition
/// is `Ready: True` — everything else here is inherently a problem
/// signal, unlike Pod phases which are fine in most states.
pub fn node_warnings(node: &Node) -> Vec<EventEntry> {
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
            Some(EventEntry {
                message: c.message.unwrap_or_else(|| c.type_.clone()),
                reason: c.type_.clone(),
                object: name.clone(),
                kind: "Node".to_string(),
                age,
                age_secs,
                severity: EventSeverity::Warning,
            })
        })
        .collect()
}

/// Every cluster Event, Normal and Warning alike — unlike the old
/// warnings-only feed, this doesn't filter by type at all, so the Events
/// panel reflects what's actually happening on the cluster, not just what
/// went wrong.
pub fn event_entry(event: &Event) -> EventEntry {
    let severity = if event.type_.as_deref() == Some("Warning") { EventSeverity::Warning } else { EventSeverity::Normal };
    // For a repeated/aggregated event (the common case — the same
    // message firing over and over, e.g. a crash loop), `series.
    // lastObservedTime` is the real "last seen" moment. `eventTime`/
    // `lastTimestamp` only capture the *first* occurrence — using those
    // alone made an event from hours ago look current if it kept
    // recurring, which is exactly backwards for a "what's happening
    // right now" feed.
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

    EventEntry {
        message: event.message.clone().unwrap_or_default(),
        reason: event.reason.clone().unwrap_or_default(),
        object: event.involved_object.name.clone().unwrap_or_default(),
        kind: event.involved_object.kind.clone().unwrap_or_default(),
        age,
        age_secs,
        severity,
    }
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
/// newest-first Events feed — everything the Overview dashboard needs.
/// Deliberately has no pod/deployment/node counts of its own — those
/// live as regular entries in `catalog` instead of being duplicated here.
pub struct Overview {
    pub events: Vec<EventEntry>,
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
        cpu_millicores: get("cpu", &crate::metrics::parse_cpu_millicores),
        memory_bytes: get("memory", &crate::metrics::parse_memory_bytes),
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
    pub age: String,
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

pub fn node_row(node: &Node, usage: Option<&crate::metrics::NodeUsage>, pod_count: usize) -> NodeRow {
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

    NodeRow {
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
        age,
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

pub fn overview(
    nodes: &[Arc<Node>],
    events: &[Arc<Event>],
    usage: Option<&crate::metrics::ClusterUsage>,
    catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>,
) -> Overview {
    let mut feed: Vec<EventEntry> = nodes.iter().flat_map(|n| node_warnings(n)).chain(events.iter().map(|e| event_entry(e))).collect();
    feed.sort_by_key(|e| e.age_secs);

    Overview {
        events: feed,
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
    pub uid: String,
    /// UIDs of this object's owners (`ownerReferences`) — what lets a
    /// Deployment's ReplicaSets, or a ReplicaSet's Pods, be found.
    pub owners: Vec<String>,
}

/// Not pinned to `DynamicType = ()` — `Resource::meta()` only reads
/// `self`, so this works identically for a typed k8s-openapi struct and
/// for a `DynamicObject` (used for CRDs, whose `DynamicType` is
/// `ApiResource` since the schema isn't known at compile time).
pub fn generic_row<K: kube::Resource>(item: &K) -> GenericRow {
    let meta = item.meta();
    let namespace = meta.namespace.clone().unwrap_or_else(|| "-".into());
    let name = meta.name.clone().unwrap_or_default();
    let age = meta.creation_timestamp.as_ref().map(|t| humanize_age(t.0)).unwrap_or_else(|| "-".into());
    let uid = meta.uid.clone().unwrap_or_default();
    let owners = meta.owner_references.iter().flatten().map(|o| o.uid.clone()).collect();
    GenericRow { namespace, name, age, uid, owners }
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

/// Sorted snapshot — same reasoning as `snapshot`/`snapshot_deployments`,
/// generic over anything `reflector::store` can hold (typed resources and
/// `DynamicObject` alike).
pub fn snapshot_generic<K>(store: &reflector::Store<K>) -> Vec<Arc<K>>
where
    K: Resource + Clone,
    K::DynamicType: Eq + std::hash::Hash + Clone,
{
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

/// A discovered CRD kind — enough to build an `ApiResource` for it later
/// and to display it in the Custom Resources picker. Discovered once at
/// startup (see `discover_crds`); a CRD installed while knav is already
/// running won't appear until restart — deliberately not worth polling
/// for, since installing a CRD is rare compared to the objects of it
/// coming and going.
#[derive(Clone)]
pub struct CrdInfo {
    pub group: &'static str,
    pub kind: &'static str,
    pub plural: String,
    pub version: String,
    pub namespaced: bool,
}

/// Lists every installed CustomResourceDefinition and extracts just
/// enough to watch it later on demand. Prefers each CRD's storage version
/// (the one actually persisted) over just the first served one, since
/// that's the version guaranteed to round-trip correctly; a CRD with no
/// served version at all (disabled) is skipped. `group`/`kind` are leaked
/// to `&'static str` — a one-time, bounded-size leak (one CRD list, once,
/// at startup) that lets `ResourceKind::CustomResource` carry a plain
/// `&'static str` label like every other kind instead of needing a
/// registry lookup just to render a title.
pub async fn discover_crds(client: &Client) -> Vec<CrdInfo> {
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;

    let api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = match api.list(&Default::default()).await {
        Ok(list) => list.items,
        Err(_) => return Vec::new(),
    };

    let mut infos: Vec<CrdInfo> = crds
        .into_iter()
        .filter_map(|crd| {
            let spec = crd.spec;
            let version =
                spec.versions.iter().find(|v| v.storage).or_else(|| spec.versions.iter().find(|v| v.served))?.name.clone();
            Some(CrdInfo {
                group: Box::leak(spec.group.into_boxed_str()),
                kind: Box::leak(spec.names.kind.into_boxed_str()),
                plural: spec.names.plural,
                version,
                namespaced: spec.scope == "Namespaced",
            })
        })
        .collect();

    infos.sort_by(|a, b| (a.group, a.kind).cmp(&(b.group, b.kind)));
    infos
}

/// `CatalogKind` for a CRD's instances — a `DynamicObject` watch instead
/// of a typed one, since the schema isn't known at compile time. Separate
/// from `WatchedKind<K>` because `Api::all` (used for every typed kind)
/// requires `DynamicType = ()`; a dynamic resource's `Api` instead needs
/// an explicit `ApiResource` built from the CRD's group/version/kind.
pub struct WatchedDynamicKind {
    store: reflector::Store<DynamicObject>,
}

impl CatalogKind for WatchedDynamicKind {
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

/// Starts watching one CRD's instances cluster-wide — called lazily, the
/// first time the user actually opens that kind, not for every installed
/// CRD up front (a cluster with Flux/cert-manager/Prometheus Operator
/// etc. installed can easily have 50+ CRDs; eagerly watching all of them
/// just for tile counts nobody's looking at isn't worth the open
/// connections).
pub fn watch_crd(client: Client, crd: &CrdInfo) -> (Box<dyn CatalogKind>, JoinHandle<()>) {
    let resource = ApiResource {
        group: crd.group.to_string(),
        version: crd.version.clone(),
        api_version: if crd.group.is_empty() { crd.version.clone() } else { format!("{}/{}", crd.group, crd.version) },
        kind: crd.kind.to_string(),
        plural: crd.plural.clone(),
    };
    let api: Api<DynamicObject> = Api::all_with(client, &resource);
    // `reflector::store()` requires `K::DynamicType: Default`, which
    // `ApiResource` doesn't implement (unlike `()` for every typed kind) —
    // `Writer::new` takes the dynamic type directly instead.
    let writer = reflector::store::Writer::new(resource);
    let reader = writer.as_reader();
    let stream = watcher(api, watcher::Config::default()).default_backoff().reflect(writer).applied_objects();
    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {}
    });
    (Box::new(WatchedDynamicKind { store: reader }), handle)
}

#[cfg(test)]
mod resource_kind_tests {
    use super::*;

    #[test]
    fn namespaced_kinds_are_told_apart_from_cluster_scoped_ones() {
        assert!(ResourceKind::Pods.is_namespaced());
        assert!(ResourceKind::Namespaces.is_namespaced());
        assert!(!ResourceKind::Nodes.is_namespaced());
        assert!(!ResourceKind::ClusterRoles.is_namespaced());
    }

    #[test]
    fn enter_opens_the_spec_except_where_it_drills_elsewhere() {
        assert!(ResourceKind::ConfigMaps.opens_spec_on_enter());
        assert!(!ResourceKind::Services.opens_spec_on_enter());
        assert!(!ResourceKind::Deployments.opens_spec_on_enter());
        assert_eq!(ResourceKind::Deployments.drill_target(), Some(ResourceKind::ReplicaSets));
        assert_eq!(ResourceKind::CronJobs.drill_target(), Some(ResourceKind::Jobs));
        assert_eq!(ResourceKind::ConfigMaps.drill_target(), None);
        assert!(ResourceKind::CustomResource(0, "Widget").opens_spec_on_enter());
        assert!(!ResourceKind::CustomResourceList.opens_spec_on_enter());
        assert!(!ResourceKind::Pods.opens_spec_on_enter());
        assert!(!ResourceKind::Nodes.opens_spec_on_enter());
    }

    #[test]
    fn command_resolves_full_names_and_short_aliases() {
        assert_eq!(ResourceKind::from_command("pods"), Some(ResourceKind::Pods));
        assert_eq!(ResourceKind::from_command("po"), Some(ResourceKind::Pods));
        assert_eq!(ResourceKind::from_command("configmaps"), Some(ResourceKind::ConfigMaps));
        assert_eq!(ResourceKind::from_command("cm"), Some(ResourceKind::ConfigMaps));
        assert_eq!(ResourceKind::from_command("crd"), Some(ResourceKind::CustomResourceList));
    }

    #[test]
    fn command_rejects_unknown_input() {
        assert_eq!(ResourceKind::from_command("bogus"), None);
        assert_eq!(ResourceKind::from_command(""), None);
    }

    #[test]
    fn from_label_and_label_round_trip_for_fixed_kinds() {
        for kind in [ResourceKind::Pods, ResourceKind::ConfigMaps, ResourceKind::CustomResourceList, ResourceKind::ClusterRoleBindings] {
            assert_eq!(ResourceKind::from_label(kind.label()), Some(kind));
        }
    }
}
