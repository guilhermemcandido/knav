//! The resource catalog: one live watch per built-in kind, plus lazily-watched CRDs.

use crate::*;

/// Every resource kind that gets a live watch + generic list/spec view but
/// no specialized row type (unlike Pods/Deployments). Nodes reuses the
/// existing `node_store` reflector instead of opening a second watch on
/// the same kind; everything else spawns its own.
pub(crate) struct Catalog {
    entries: Vec<(ResourceKind, &'static str, Box<dyn k8s::CatalogKind>)>,
    /// Every discovered CRD kind — listed once at startup, watched lazily
    /// (see `resolve`) only once the user actually opens one.
    pub(crate) crds: Vec<k8s::CrdInfo>,
    crd_watches: HashMap<usize, Box<dyn k8s::CatalogKind>>,
    /// Every resource type the API server lists, from discovery; each is
    /// fetched (as a server-side Table) only once it is opened.
    pub(crate) apis: Vec<k8s::ApiInfo>,
    api_list: k8s::ApiList,
    api_tables: HashMap<usize, k8s::TableKind>,
}

impl Catalog {
    pub(crate) fn spawn(client: &Client, node_store: Store<Node>, crds: Vec<k8s::CrdInfo>, apis: Vec<k8s::ApiInfo>) -> Self {
        macro_rules! kind {
            ($variant:ident, $label:literal, $ty:ty) => {{
                let (boxed, _handle) = k8s::watch_kind::<$ty>(client.clone());
                (ResourceKind::$variant, $label, boxed)
            }};
        }
        Catalog {
            entries: vec![
                (ResourceKind::Nodes, "Nodes", Box::new(k8s::WatchedKind::from_store(node_store))),
                kind!(Namespaces, "Namespaces", Namespace),
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
            api_list: k8s::ApiList { apis: apis.clone() },
            apis,
            api_tables: HashMap::new(),
        }
    }

    pub(crate) fn count(&self, kind: ResourceKind) -> usize {
        self.get(kind).map(|k| k.count()).unwrap_or(0)
    }

    /// Looks up the live watch for a built-in kind — `None` for Overview/
    /// Pods/Deployments/the CRD kinds, which aren't in `entries` (Pods/
    /// Deployments have their own specialized reflectors and row types;
    /// CRDs go through `resolve` instead since opening one may need to
    /// lazily start its watch).
    pub(crate) fn get(&self, kind: ResourceKind) -> Option<&dyn k8s::CatalogKind> {
        self.entries.iter().find(|(k, _, _)| *k == kind).map(|(_, _, b)| b.as_ref())
    }

    /// Like `get`, but also covers CRD kinds — starting their watch on
    /// first use ("watch on open", not eagerly for all installed CRDs).
    /// The one place `main::run` should go through to read rows/spec for
    /// whatever `current_kind` actually is.
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
            _ => self.get(kind),
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
                // "Custom Resources" itself is the whole unfiltered
                // picker; one further tile per discovered API group so
                // the (often long) flat list is organized the way
                // Freelens groups its own custom-resource menu. Every
                // count here is "how many CRD *kinds*", not a live
                // object count — known for free from discovery, no
                // watch needed, consistent with "list only, watch on
                // open".
                std::iter::once(("Custom Resources", self.crds.len()))
                    .chain(self.crd_groups().into_iter().map(|group| (group, self.crds.iter().filter(|c| c.group == group).count())))
                    .collect(),
            ),
        ]
    }

    /// Every distinct API group among the discovered CRDs, in the same
    /// order `discover_crds` already sorted them (group, then kind) —
    /// a simple adjacent-dedup instead of a `HashSet` keeps that order
    /// intact instead of scrambling it.
    pub(crate) fn crd_groups(&self) -> Vec<&'static str> {
        let mut groups: Vec<&'static str> = Vec::new();
        for crd in &self.crds {
            if groups.last() != Some(&crd.group) {
                groups.push(crd.group);
            }
        }
        groups
    }

    /// Resolves an Overview tile's/menu's label back to the `ResourceKind`
    /// it switches to. Tries the fixed kinds first (`ResourceKind::
    /// from_label`); a label that isn't one of those but does match a
    /// discovered CRD group must be that group's tile (the "Custom
    /// Resources" section is the only place such labels appear).
    pub(crate) fn kind_for_tile_label(&self, label: &str) -> Option<ResourceKind> {
        ResourceKind::from_label(label).or_else(|| self.crds.iter().find(|c| c.group == label).map(|c| ResourceKind::CustomResourceGroup(c.group)))
    }
}
