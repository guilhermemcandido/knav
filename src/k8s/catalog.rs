//! The resource catalog: one live watch per built-in kind, plus lazily-watched CRDs.

use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use crate::*;

/// One built-in kind: a cheap count from the start, the full live watch only once needed.
struct Entry {
    kind: ResourceKind,
    label: &'static str,
    /// From a metadata-only watch; nothing but the number is kept.
    count: Arc<AtomicUsize>,
    start: Box<dyn Fn(&Client) -> Box<dyn k8s::CatalogKind> + Send + Sync>,
    full: Option<Box<dyn k8s::CatalogKind>>,
}

/// Every built-in kind besides Pods and Deployments. Counts are always live; a kind's full
/// objects are watched only once it is opened (see `resolve`), so a cluster's Secrets and
/// ConfigMaps are not downloaded just to show the Overview.
pub(crate) struct Catalog {
    client: Client,
    entries: Vec<Entry>,
    /// Every discovered CRD kind, listed once at startup, watched lazily
    /// (see `resolve`) only once the user actually opens one.
    pub(crate) crds: Vec<k8s::CrdInfo>,
    crd_watches: HashMap<usize, Box<dyn k8s::CatalogKind>>,
    /// Every resource type the API server lists, from discovery; each is
    /// fetched (as a server-side Table) only once it is opened.
    pub(crate) apis: Vec<k8s::ApiInfo>,
    api_list: k8s::ApiList,
    /// Object counts per type for the type lists, counted in the background once one is opened.
    pub(crate) counts: k8s::InstanceCounts,
    counter: Option<k8s::Counter>,
    api_tables: HashMap<usize, k8s::TableKind>,
}

impl Catalog {
    pub(crate) fn spawn(client: &Client, node_store: Store<Node>, crds: Vec<k8s::CrdInfo>, apis: Vec<k8s::ApiInfo>) -> Self {
        macro_rules! kind {
            ($variant:ident, $label:literal, $ty:ty) => {
                Entry {
                    kind: ResourceKind::$variant,
                    label: $label,
                    count: k8s::watch_count::<$ty>(client.clone()),
                    start: Box::new(|client| k8s::watch_kind::<$ty>(client.clone()).0),
                    full: None,
                }
            };
        }
        let nodes = Entry { kind: ResourceKind::Nodes, label: "Nodes", count: Arc::default(), start: Box::new(|_| unreachable!("nodes are watched from the start")), full: Some(Box::new(k8s::WatchedKind::from_store(node_store))) };
        // Namespaces feed the namespace picker, so they are always held in full.
        let mut namespaces = kind!(Namespaces, "Namespaces", Namespace);
        namespaces.full = Some((namespaces.start)(client));
        let counts = k8s::InstanceCounts::default();
        Catalog {
            client: client.clone(),
            entries: vec![
                nodes,
                namespaces,
                kind!(ReplicaSets, "ReplicaSets", ReplicaSet),
                kind!(StatefulSets, "StatefulSets", StatefulSet),
                kind!(DaemonSets, "DaemonSets", DaemonSet),
                kind!(Jobs, "Jobs", Job),
                kind!(CronJobs, "CronJobs", CronJob),
                kind!(ConfigMaps, "ConfigMaps", ConfigMap),
                kind!(Secrets, "Secrets", Secret),
                kind!(Hpas, "HPAs", HorizontalPodAutoscaler),
                kind!(Services, "Services", Service),
                kind!(Endpoints, "Endpoints", Endpoints),
                kind!(Ingresses, "Ingresses", Ingress),
                kind!(NetworkPolicies, "NetworkPolicies", NetworkPolicy),
                kind!(Pvcs, "PVCs", PersistentVolumeClaim),
                kind!(Pvs, "PVs", PersistentVolume),
                kind!(StorageClasses, "StorageClasses", StorageClass),
                kind!(ServiceAccounts, "ServiceAccounts", ServiceAccount),
                kind!(Roles, "Roles", Role),
                kind!(RoleBindings, "RoleBindings", RoleBinding),
                kind!(ClusterRoles, "ClusterRoles", ClusterRole),
                kind!(ClusterRoleBindings, "ClusterRoleBindings", ClusterRoleBinding),
            ],
            crds,
            crd_watches: HashMap::new(),
            api_list: k8s::ApiList { apis: apis.clone(), counts: counts.clone() },
            counts,
            counter: None,
            apis,
            api_tables: HashMap::new(),
        }
    }

    /// Starts counting the objects of every type (once), and follows the namespace shown.
    pub(crate) fn count_instances(&mut self, namespace: Option<&str>) {
        let counter = self.counter.get_or_insert_with(|| {
            // Custom resources by their own storage version, then every other type discovery lists
            // (a CRD served only in an older version is missing from discovery's preferred one).
            let mut types: HashMap<String, k8s::ApiInfo> = self.crds.iter().map(k8s::ApiInfo::from).map(|t| (k8s::count_key(t.group, t.plural), t)).collect();
            for api in &self.apis {
                types.entry(k8s::count_key(api.group, api.plural)).or_insert_with(|| api.clone());
            }
            k8s::Counter::start(self.client.clone(), types, self.counts.clone())
        });
        counter.set_namespace(&self.counts, namespace);
    }

    /// Names the types on screen, the only ones counted.
    pub(crate) fn want_counts(&mut self, keys: Vec<String>) {
        if let Some(counter) = &self.counter {
            counter.want(keys);
        }
    }

    /// Starts the full watch of a built-in kind if it is not running.
    pub(crate) fn ensure(&mut self, kind: ResourceKind) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.kind == kind)
            && entry.full.is_none()
        {
            entry.full = Some((entry.start)(&self.client));
        }
    }

    /// Like `ensure`, by the label an Overview tile carries.
    pub(crate) fn ensure_label(&mut self, label: &str) {
        if let Some(kind) = self.entries.iter().find(|e| e.label == label).map(|e| e.kind) {
            self.ensure(kind);
        }
    }

    /// Whether every built-in kind in `kinds` has its objects loaded.
    pub(crate) fn all_ready(&self, kinds: &[ResourceKind]) -> bool {
        kinds.iter().all(|k| self.get(*k).is_none_or(|f| f.ready()))
    }

    /// Health by kind label for every watched kind that has one, plus the kinds
    /// the caller computes from its own rows.
    pub(crate) fn health(&self, extra: impl IntoIterator<Item = (&'static str, k8s::Health)>) -> HashMap<&'static str, k8s::Health> {
        let mut map: HashMap<&'static str, k8s::Health> = self.entries.iter().filter_map(|e| e.full.as_ref()?.health().map(|h| (e.label, h))).collect();
        map.extend(extra);
        map
    }

    pub(crate) fn count(&self, kind: ResourceKind) -> usize {
        self.entries.iter().find(|e| e.kind == kind).map(|e| e.full.as_ref().map_or_else(|| e.count.load(Ordering::Relaxed), |f| f.count())).unwrap_or(0)
    }

    /// The live watch for a built-in kind. `None` for Overview, Pods, Deployments and
    /// CRDs, which are not in `entries` (CRDs go through `resolve`).
    pub(crate) fn get(&self, kind: ResourceKind) -> Option<&dyn k8s::CatalogKind> {
        self.entries.iter().find(|e| e.kind == kind).and_then(|e| e.full.as_deref())
    }

    /// Like `get`, but also covers CRD kinds, starting their watch on first use.
    pub(crate) fn resolve(&mut self, kind: ResourceKind, client: &Client) -> Option<&dyn k8s::CatalogKind> {
        match kind {
            // A custom resource's instances, with the printer columns its CRD defines.
            ResourceKind::CustomResource(index, _) => {
                if !self.crd_watches.contains_key(&index) {
                    let api = k8s::ApiInfo::from(self.crds.get(index)?);
                    self.crd_watches.insert(index, Box::new(k8s::TableKind::start(client.clone(), &api)));
                }
                self.crd_watches.get(&index).map(|b| b.as_ref())
            }
            ResourceKind::ApiResources => Some(&self.api_list),
            ResourceKind::Api(index, _) => {
                if !self.api_tables.contains_key(&index) {
                    let api = self.apis.get(index)?.clone();
                    self.api_tables.insert(index, k8s::TableKind::start(client.clone(), &api));
                }
                self.api_tables.get(&index).map(|t| t as &dyn k8s::CatalogKind)
            }
            _ => {
                self.ensure(kind);
                self.get(kind)
            }
        }
    }

    /// Merges in the live-reflector counts for Pods/Deployments so
    /// callers get one complete catalog instead of two partial ones.
    pub(crate) fn sections(&self, pod_count: usize, deployment_count: usize) -> Vec<(&'static str, Vec<(&'static str, usize)>)> {
        vec![
            (
                "Cluster",
                vec![("Nodes", self.count(ResourceKind::Nodes)), ("Namespaces", self.count(ResourceKind::Namespaces)), ("API Resources", self.apis.len())],
            ),
            (
                "Workloads",
                vec![
                    ("Pods", pod_count),
                    ("Deployments", deployment_count),
                    ("ReplicaSets", self.count(ResourceKind::ReplicaSets)),
                    ("StatefulSets", self.count(ResourceKind::StatefulSets)),
                    ("DaemonSets", self.count(ResourceKind::DaemonSets)),
                    ("Jobs", self.count(ResourceKind::Jobs)),
                    ("CronJobs", self.count(ResourceKind::CronJobs)),
                ],
            ),
            (
                "Config",
                vec![
                    ("ConfigMaps", self.count(ResourceKind::ConfigMaps)),
                    ("Secrets", self.count(ResourceKind::Secrets)),
                    ("HPAs", self.count(ResourceKind::Hpas)),
                ],
            ),
            (
                "Network",
                vec![
                    ("Services", self.count(ResourceKind::Services)),
                    ("Endpoints", self.count(ResourceKind::Endpoints)),
                    ("Ingresses", self.count(ResourceKind::Ingresses)),
                    ("NetworkPolicies", self.count(ResourceKind::NetworkPolicies)),
                ],
            ),
            (
                "Storage",
                vec![
                    ("PVCs", self.count(ResourceKind::Pvcs)),
                    ("PVs", self.count(ResourceKind::Pvs)),
                    ("StorageClasses", self.count(ResourceKind::StorageClasses)),
                ],
            ),
            (
                "Access Control",
                vec![
                    ("ServiceAccounts", self.count(ResourceKind::ServiceAccounts)),
                    ("Roles", self.count(ResourceKind::Roles)),
                    ("RoleBindings", self.count(ResourceKind::RoleBindings)),
                    ("ClusterRoles", self.count(ResourceKind::ClusterRoles)),
                    ("ClusterRoleBindings", self.count(ResourceKind::ClusterRoleBindings)),
                ],
            ),
            (
                "Custom Resources",
                // "Custom Resources" is the whole picker; one more tile per API group. Counts
                // are CRD kinds known from discovery, not live objects.
                std::iter::once(("Custom Resources", self.crds.len()))
                    .chain(self.crd_groups().into_iter().map(|group| (group, self.crds.iter().filter(|c| c.group == group).count())))
                    .collect(),
            ),
        ]
    }

    /// Every distinct API group among the CRDs, in the order `discover_crds` sorted
    /// them (adjacent dedup keeps that order).
    pub(crate) fn crd_groups(&self) -> Vec<&'static str> {
        let mut groups: Vec<&'static str> = Vec::new();
        for crd in &self.crds {
            if groups.last() != Some(&crd.group) {
                groups.push(crd.group);
            }
        }
        groups
    }

    /// Resolves an Overview tile or menu label to its `ResourceKind`: a fixed kind
    /// first, else a discovered CRD group's tile.
    pub(crate) fn kind_for_tile_label(&self, label: &str) -> Option<ResourceKind> {
        ResourceKind::from_label(label).or_else(|| self.crds.iter().find(|c| c.group == label).map(|c| ResourceKind::CustomResourceGroup(c.group)))
    }
}
