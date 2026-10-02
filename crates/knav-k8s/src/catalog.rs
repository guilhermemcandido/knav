//! The resource catalog: one live watch per built-in kind, plus lazily-watched CRDs.

use std::collections::HashMap;

use k8s_openapi::api::{
    apps::v1::{DaemonSet, ReplicaSet, StatefulSet},
    autoscaling::v2::HorizontalPodAutoscaler,
    batch::v1::{CronJob, Job},
    core::v1::{ConfigMap, Endpoints, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Secret, Service, ServiceAccount},
    networking::v1::{Ingress, NetworkPolicy},
    rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding},
    storage::v1::StorageClass,
};
use kube::{Client, runtime::reflector::Store};

use crate::{ResourceKind};

use std::collections::BTreeMap;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};


/// Starts a kind's full watch.
type StartWatch = Box<dyn Fn(&Client) -> Box<dyn crate::CatalogKind> + Send + Sync>;

/// One built-in kind: a cheap count from the start, the full live watch only once needed.
struct Entry {
    kind: ResourceKind,
    label: &'static str,
    /// From a metadata-only watch, keeping just the number.
    count: Arc<AtomicUsize>,
    start: StartWatch,
    full: Option<Box<dyn crate::CatalogKind>>,
}

/// What the loaded extensions add, as plain data from `Registry::index`,
/// so this crate never depends on the extensions crate.
#[derive(Default)]
pub struct ExtensionIndex {
    pub kinds: Vec<IndexedKind>,
    /// Categories with a dashboard, whether or not their extension is enabled.
    pub dashboards: Vec<&'static str>,
}

pub struct IndexedKind {
    pub extension: String,
    pub group: String,
    pub kind: String,
    pub category: String,
    pub view: Option<crate::details::ViewTemplate>,
}

impl ExtensionIndex {
    pub fn enabled<'a, 'b>(&'a self, enabled: &'b [String]) -> impl Iterator<Item = &'a IndexedKind> + use<'a, 'b> {
        self.kinds.iter().filter(move |k| enabled.contains(&k.extension))
    }
}

/// Every built-in kind besides Pods and Deployments. Counts are always live, but a kind's
/// objects are watched only once it is opened, so the Overview never downloads every Secret.
pub struct Catalog {
    client: Client,
    entries: Vec<Entry>,
    /// Every discovered CRD kind, watched only once it is opened.
    pub crds: Vec<crate::CrdInfo>,
    crd_watches: HashMap<usize, Box<dyn crate::CatalogKind>>,
    /// Every resource type the API server lists, fetched as a Table once opened.
    pub apis: Vec<crate::ApiInfo>,
    api_list: crate::ApiList,
    /// Object counts per type, counted in the background once a type list is opened.
    pub counts: crate::InstanceCounts,
    counter: Option<crate::Counter>,
    api_tables: HashMap<usize, crate::TableKind>,
    pub extensions: ExtensionIndex,
    /// Helm releases, from a watch of the release Secrets. Kept apart from `entries`
    /// because it runs only while the Helm extension is on.
    helm: Option<Box<dyn crate::CatalogKind>>,
}

impl Catalog {
    pub fn spawn(client: &Client, node_store: Store<Node>, node_feed: Arc<crate::Feed>, extensions: ExtensionIndex) -> Self {
        macro_rules! kind {
            ($variant:ident, $label:literal, $ty:ty) => {
                Entry {
                    kind: ResourceKind::$variant,
                    label: $label,
                    count: crate::watch_count::<$ty>(client.clone()),
                    start: Box::new(|client| crate::watch_kind::<$ty>(client.clone()).0),
                    full: None,
                }
            };
        }
        let nodes = Entry { kind: ResourceKind::Nodes, label: "Nodes", count: Arc::default(), start: Box::new(|_| unreachable!("nodes are watched from the start")), full: Some(Box::new(crate::WatchedKind::new(node_store, node_feed))) };
        // Namespaces feed the namespace picker, so they are always held in full.
        let mut namespaces = kind!(Namespaces, "Namespaces", Namespace);
        namespaces.full = Some((namespaces.start)(client));
        let counts = crate::InstanceCounts::default();
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
            crds: Vec::new(),
            crd_watches: HashMap::new(),
            api_list: crate::ApiList { apis: Vec::new(), counts: counts.clone() },
            counts,
            counter: None,
            apis: Vec::new(),
            api_tables: HashMap::new(),
            extensions,
            helm: None,
        }
    }

    /// Fills in what discovery found. Nothing reads these before then: the loading
    /// screen waits for discovery.
    pub fn set_types(&mut self, apis: Vec<crate::ApiInfo>, crds: Vec<crate::CrdInfo>) {
        self.api_list.apis = apis.clone();
        self.apis = apis;
        self.crds = crds;
    }

    /// Starts counting every type's objects (once), following the namespace shown.
    pub fn count_instances(&mut self, namespace: Option<&str>) {
        let counter = self.counter.get_or_insert_with(|| {
            // CRDs by their storage version first: one served only in an older
            // version is missing from discovery's preferred list.
            let mut types: HashMap<String, crate::ApiInfo> = self.crds.iter().map(crate::ApiInfo::from).map(|t| (crate::count_key(t.group, t.plural), t)).collect();
            for api in &self.apis {
                types.entry(crate::count_key(api.group, api.plural)).or_insert_with(|| api.clone());
            }
            crate::Counter::start(self.client.clone(), types, self.counts.clone())
        });
        counter.set_namespace(&self.counts, namespace);
    }

    /// Names the types on screen, the only ones counted.
    pub fn want_counts(&mut self, keys: Vec<String>) {
        if let Some(counter) = &self.counter {
            counter.want(keys);
        }
    }

    /// The list an object of `kind` lives in: a built-in one, or a CRD's by its kind.
    pub fn list_for(&self, kind: &str) -> Option<ResourceKind> {
        ResourceKind::from_owner_kind(kind).or_else(|| {
            let index = self.crds.iter().position(|c| c.kind == kind)?;
            Some(ResourceKind::CustomResource(index, self.crds[index].kind))
        })
    }

    pub fn ensure(&mut self, kind: ResourceKind) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.kind == kind)
            && entry.full.is_none()
        {
            entry.full = Some((entry.start)(&self.client));
        }
    }

    /// Like `ensure`, by the label an Overview tile carries.
    pub fn ensure_label(&mut self, label: &str) {
        if let Some(kind) = self.entries.iter().find(|e| e.label == label).map(|e| e.kind) {
            self.ensure(kind);
        }
    }

    /// Health by kind label for every watched kind that has one, plus the kinds
    /// the caller computes from its own rows.
    pub fn health(&self, extra: impl IntoIterator<Item = (&'static str, crate::Health)>) -> HashMap<&'static str, crate::Health> {
        let mut map: HashMap<&'static str, crate::Health> = self.entries.iter().filter_map(|e| e.full.as_ref()?.health().map(|h| (e.label, h))).collect();
        map.extend(extra);
        map
    }

    pub fn count(&self, kind: ResourceKind) -> usize {
        if kind == ResourceKind::HelmReleases {
            return self.helm.as_ref().map_or(0, |h| h.count());
        }
        self.entries.iter().find(|e| e.kind == kind).map(|e| e.full.as_ref().map_or_else(|| e.count.load(Ordering::Relaxed), |f| f.count())).unwrap_or(0)
    }

    /// The live watch for a built-in kind. `None` for the Overview, Pods, Deployments
    /// and CRDs, which are not in `entries` (CRDs go through `resolve`).
    pub fn get(&self, kind: ResourceKind) -> Option<&dyn crate::CatalogKind> {
        if kind == ResourceKind::HelmReleases {
            return self.helm.as_deref();
        }
        self.entries.iter().find(|e| e.kind == kind).and_then(|e| e.full.as_deref())
    }

    /// Starts the Helm watch as soon as Helm is enabled, not when its list opens,
    /// so its Overview count is right from the start. Cheap enough to call every tick.
    pub fn ensure_helm(&mut self, client: &Client, extensions_enabled: &[String]) {
        if self.helm.is_none() && extensions_enabled.iter().any(|e| e == "helm") {
            self.helm = Some(Box::new(crate::HelmStore::start(client.clone())));
        }
    }

    /// One CRD kind's watch by group and kind, started if needed. Dashboards use it,
    /// and it shares the watch a normal list of that kind would use.
    pub fn resolve_crd(&mut self, group: &str, kind: &str, client: &Client) -> Option<&dyn crate::CatalogKind> {
        let index = self.crds.iter().position(|c| c.group == group && c.kind == kind)?;
        let label = self.crds[index].kind;
        self.resolve(ResourceKind::CustomResource(index, label), client)
    }

    /// Like `get`, but also covers CRD kinds, starting their watch on first use.
    pub fn resolve(&mut self, kind: ResourceKind, client: &Client) -> Option<&dyn crate::CatalogKind> {
        match kind {
            ResourceKind::CustomResource(index, _) => {
                if !self.crd_watches.contains_key(&index) {
                    let api = crate::ApiInfo::from(self.crds.get(index)?);
                    self.crd_watches.insert(index, Box::new(crate::TableKind::start(client.clone(), &api)));
                }
                self.crd_watches.get(&index).map(|b| b.as_ref())
            }
            ResourceKind::HelmReleases => {
                if self.helm.is_none() {
                    self.helm = Some(Box::new(crate::HelmStore::start(client.clone())));
                }
                self.helm.as_deref()
            }
            ResourceKind::ApiResources => Some(&self.api_list),
            ResourceKind::Api(index, _) => {
                if !self.api_tables.contains_key(&index) {
                    let api = self.apis.get(index)?.clone();
                    self.api_tables.insert(index, crate::TableKind::start(client.clone(), &api));
                }
                self.api_tables.get(&index).map(|t| t as &dyn crate::CatalogKind)
            }
            _ => {
                self.ensure(kind);
                self.get(kind)
            }
        }
    }

    /// Every category and kind `sections` would show, without counts, for the Layout
    /// tab. Built from the live catalog so enabled extensions' categories are editable too.
    pub fn layout_names(&self, extensions_enabled: &[String]) -> Vec<(&'static str, Vec<&'static str>)> {
        self.sections(0, 0, extensions_enabled).into_iter().map(|(category, items)| (category, items.into_iter().map(|(name, _)| name).collect())).collect()
    }

    /// The view template an enabled extension declares for `manifest`'s kind, which
    /// `details::details` shows instead of the generic summary.
    pub fn view_for<'a>(&'a self, extensions_enabled: &[String], manifest: &serde_yaml::Value) -> Option<&'a crate::details::ViewTemplate> {
        let api_version = manifest.get("apiVersion")?.as_str()?;
        let kind = manifest.get("kind")?.as_str()?;
        let group = api_version.rsplit_once('/').map(|(g, _)| g).unwrap_or("");
        self.extensions.enabled(extensions_enabled).find(|k| k.group == group && k.kind == kind).and_then(|k| k.view.as_ref())
    }

    /// Every Overview category with its kinds and counts: the built-ins, then one
    /// section per category of the enabled extensions, for CRDs the cluster has.
    pub fn sections(&self, pod_count: usize, deployment_count: usize, extensions_enabled: &[String]) -> Vec<(&'static str, Vec<(&'static str, usize)>)> {
        let mut sections = vec![
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
                "CustomResources",
                // The picker, then one tile per API group, counting CRD kinds, not objects.
                std::iter::once(("CustomResources", self.crds.len()))
                    .chain(self.crd_groups().into_iter().map(|group| (group, self.crds.iter().filter(|c| c.group == group).count())))
                    .collect(),
            ),
        ];
        // Extensions go after CustomResources: optional additions past the fixed set.
        let mut extension_sections = self.extension_sections(extensions_enabled);
        if extensions_enabled.iter().any(|e| e == "helm") {
            // Releases aren't a CRD, so they join whatever helm.cattle.io kinds
            // matched, making one Helm box either way.
            let releases = ("HelmReleases", self.count(ResourceKind::HelmReleases));
            match extension_sections.iter_mut().find(|(name, _)| *name == "Helm") {
                Some((_, items)) => items.push(releases),
                None => {
                    extension_sections.push(("Helm", vec![releases]));
                    extension_sections.sort_by_key(|(name, _)| *name);
                }
            }
        }
        // A dashboard tile goes first in its category, once the category exists
        // (at least one of its CRDs is installed).
        for category in self.dashboard_categories() {
            if let Some((_, items)) = extension_sections.iter_mut().find(|(name, _)| *name == category) {
                items.insert(0, (category, 0));
            }
        }
        sections.extend(extension_sections);
        sections
    }

    /// One section per category of the enabled extensions, listing only kinds the
    /// cluster has installed. Counts come from the shared background counter.
    fn extension_sections(&self, extensions_enabled: &[String]) -> Vec<(&'static str, Vec<(&'static str, usize)>)> {
        let mut by_category: BTreeMap<&'static str, Vec<(&'static str, usize)>> = BTreeMap::new();
        for ext_kind in self.extensions.enabled(extensions_enabled) {
            let Some(crd) = self.crds.iter().find(|c| c.group == ext_kind.group && c.kind == ext_kind.kind) else { continue };
            let category = crate::leak(&ext_kind.category);
            let count = self.counts.get(crd.group, &crd.plural).known_or(0);
            by_category.entry(category).or_default().push((crd.kind, count));
        }
        by_category.into_iter().collect()
    }

    /// Count keys for the enabled extensions' kinds. The Overview always shows them,
    /// so they are always wanted.
    pub fn want_extension_counts(&self, extensions_enabled: &[String]) -> Vec<String> {
        self.extensions
            .enabled(extensions_enabled)
            .filter_map(|ext_kind| self.crds.iter().find(|c| c.group == ext_kind.group && c.kind == ext_kind.kind))
            .map(|crd| crate::count_key(crd.group, &crd.plural))
            .collect()
    }

    /// Every category with a dashboard, for the `:` menu and Overview tiles.
    pub fn dashboard_categories(&self) -> Vec<&'static str> {
        self.extensions.dashboards.clone()
    }

    /// Every distinct CRD API group, in discovery's sorted order.
    pub fn crd_groups(&self) -> Vec<&'static str> {
        let mut groups: Vec<&'static str> = Vec::new();
        for crd in &self.crds {
            if groups.last() != Some(&crd.group) {
                groups.push(crd.group);
            }
        }
        groups
    }

    /// The kind an Overview tile or menu label opens: a fixed kind, else a CRD
    /// group's tile, else a CRD kind an extension put on the Overview directly.
    pub fn kind_for_tile_label(&self, label: &str) -> Option<ResourceKind> {
        ResourceKind::from_label(label)
            .or_else(|| self.crds.iter().find(|c| c.group == label).map(|c| ResourceKind::CustomResourceGroup(c.group)))
            .or_else(|| self.crds.iter().position(|c| c.kind == label).map(|i| ResourceKind::CustomResource(i, self.crds[i].kind)))
            .or_else(|| self.dashboard_categories().into_iter().find(|c| *c == label).map(ResourceKind::ExtensionDashboard))
    }
}
