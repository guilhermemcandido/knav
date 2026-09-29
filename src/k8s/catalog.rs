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

use crate::k8s::{self, ResourceKind};

use std::collections::BTreeMap;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};


/// One built-in kind: a cheap count from the start, the full live watch only once needed.
struct Entry {
    kind: ResourceKind,
    label: &'static str,
    /// From a metadata-only watch; nothing but the number is kept.
    count: Arc<AtomicUsize>,
    start: Box<dyn Fn(&Client) -> Box<dyn k8s::CatalogKind> + Send + Sync>,
    full: Option<Box<dyn k8s::CatalogKind>>,
}

/// What the loaded extensions add to the catalog, as plain data built by
/// `extensions::Registry::index`, so this module never depends on extensions.
#[derive(Default)]
pub struct ExtensionIndex {
    pub kinds: Vec<IndexedKind>,
    /// Categories with a dashboard, whether or not their extension is enabled.
    pub dashboards: Vec<&'static str>,
}

/// One kind an extension adds.
pub struct IndexedKind {
    pub extension: String,
    pub group: String,
    pub kind: String,
    pub category: String,
    pub view: Option<k8s::details::ViewTemplate>,
}

impl ExtensionIndex {
    /// The kinds of the extensions turned on in `enabled`.
    pub fn enabled<'a, 'b>(&'a self, enabled: &'b [String]) -> impl Iterator<Item = &'a IndexedKind> + use<'a, 'b> {
        self.kinds.iter().filter(move |k| enabled.contains(&k.extension))
    }
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
    /// Every loaded extension (bundled and from `~/.config/knav/extensions/`),
    /// enabled or not; `sections` only uses the enabled ones.
    pub(crate) extensions: ExtensionIndex,
    /// Helm releases: a field-selected watch of Secrets (see `k8s::HelmStore`),
    /// started once "helm" is enabled (see `ensure_helm`) — not one of the
    /// fixed `entries` above, since whether it runs at all is a toggle, not
    /// "has anything asked for it yet".
    helm: Option<Box<dyn k8s::CatalogKind>>,
}

impl Catalog {
    pub(crate) fn spawn(client: &Client, node_store: Store<Node>, node_feed: Arc<k8s::Feed>, extensions: ExtensionIndex) -> Self {
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
        let nodes = Entry { kind: ResourceKind::Nodes, label: "Nodes", count: Arc::default(), start: Box::new(|_| unreachable!("nodes are watched from the start")), full: Some(Box::new(k8s::WatchedKind::new(node_store, node_feed))) };
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
            crds: Vec::new(),
            crd_watches: HashMap::new(),
            api_list: k8s::ApiList { apis: Vec::new(), counts: counts.clone() },
            counts,
            counter: None,
            apis: Vec::new(),
            api_tables: HashMap::new(),
            extensions,
            helm: None,
        }
    }

    /// Fills in what discovery found. It runs while the built-in kinds already load, and
    /// nothing reads these before it is done (the loading screen waits for it).
    pub(crate) fn set_types(&mut self, apis: Vec<k8s::ApiInfo>, crds: Vec<k8s::CrdInfo>) {
        self.api_list.apis = apis.clone();
        self.apis = apis;
        self.crds = crds;
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
        if kind == ResourceKind::HelmReleases {
            return self.helm.as_ref().map_or(0, |h| h.count());
        }
        self.entries.iter().find(|e| e.kind == kind).map(|e| e.full.as_ref().map_or_else(|| e.count.load(Ordering::Relaxed), |f| f.count())).unwrap_or(0)
    }

    /// The live watch for a built-in kind. `None` for Overview, Pods, Deployments and
    /// CRDs, which are not in `entries` (CRDs go through `resolve`).
    pub(crate) fn get(&self, kind: ResourceKind) -> Option<&dyn k8s::CatalogKind> {
        if kind == ResourceKind::HelmReleases {
            return self.helm.as_deref();
        }
        self.entries.iter().find(|e| e.kind == kind).and_then(|e| e.full.as_deref())
    }

    /// Starts the Helm watch as soon as "helm" is enabled, not just once its
    /// list is opened — cheap (a field-selected watch of the release Secrets
    /// only, see `k8s::HelmStore`), so its Overview count is never a stale or
    /// misleading 0 for something that's actually on. Call every tick, like
    /// `want_extension_counts`/`want_counts` for the CRD-backed extensions.
    pub(crate) fn ensure_helm(&mut self, client: &Client, extensions_enabled: &[String]) {
        if self.helm.is_none() && extensions_enabled.iter().any(|e| e == "helm") {
            self.helm = Some(Box::new(k8s::HelmStore::start(client.clone())));
        }
    }

    /// Resolves one CRD kind's watch directly by group+kind, starting it if
    /// needed — what a dashboard uses to pull several CRD kinds' data at
    /// once, since it isn't itself one list a user opens through `resolve`.
    /// Goes through the same `CustomResource(index, _)` path `resolve` does,
    /// so a watch this starts is the one a normal list of the same kind reuses.
    pub(crate) fn resolve_crd(&mut self, group: &str, kind: &str, client: &Client) -> Option<&dyn k8s::CatalogKind> {
        let index = self.crds.iter().position(|c| c.group == group && c.kind == kind)?;
        let label = self.crds[index].kind;
        self.resolve(ResourceKind::CustomResource(index, label), client)
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
            ResourceKind::HelmReleases => {
                if self.helm.is_none() {
                    self.helm = Some(Box::new(k8s::HelmStore::start(client.clone())));
                }
                self.helm.as_deref()
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

    /// Every category and kind name `sections` would currently show, counts
    /// dropped — what the Layout tab reorders/hides. `overview_layout::resolve` uses
    /// this instead of its own fixed default so a category that only exists
    /// once an extension is enabled (Helm, Flux, ...) is still editable, not
    /// just the built-in set.
    pub(crate) fn layout_names(&self, extensions_enabled: &[String]) -> Vec<(&'static str, Vec<&'static str>)> {
        self.sections(0, 0, extensions_enabled).into_iter().map(|(category, items)| (category, items.into_iter().map(|(name, _)| name).collect())).collect()
    }

    /// The view template an enabled extension declares for `manifest`'s kind,
    /// if any (group from its `apiVersion`, up to the `/`) — what
    /// `k8s::details::details` renders for it instead of the generic dump.
    pub(crate) fn view_for<'a>(&'a self, extensions_enabled: &[String], manifest: &serde_yaml::Value) -> Option<&'a k8s::details::ViewTemplate> {
        let api_version = manifest.get("apiVersion")?.as_str()?;
        let kind = manifest.get("kind")?.as_str()?;
        let group = api_version.rsplit_once('/').map(|(g, _)| g).unwrap_or("");
        self.extensions.enabled(extensions_enabled).find(|k| k.group == group && k.kind == kind).and_then(|k| k.view.as_ref())
    }

    /// Merges in the live-reflector counts for Pods/Deployments so
    /// callers get one complete catalog instead of two partial ones, plus
    /// one section per category an enabled extension asked for (see
    /// `extension_sections`), for CRD kinds the cluster actually has.
    pub(crate) fn sections(&self, pod_count: usize, deployment_count: usize, extensions_enabled: &[String]) -> Vec<(&'static str, Vec<(&'static str, usize)>)> {
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
                // "CustomResources" is the whole picker; one more tile per API group. Counts
                // are CRD kinds known from discovery, not live objects.
                std::iter::once(("CustomResources", self.crds.len()))
                    .chain(self.crd_groups().into_iter().map(|group| (group, self.crds.iter().filter(|c| c.group == group).count())))
                    .collect(),
            ),
        ];
        // After "CustomResources": these are optional, opt-in categories, not
        // built-ins, so they read as an addition past the fixed set rather than
        // interrupting it.
        let mut extension_sections = self.extension_sections(extensions_enabled);
        if extensions_enabled.iter().any(|e| e == "helm") {
            // Native (releases aren't a CRD, so `extension_sections` never
            // produces this tile on its own) alongside whatever `helm.cattle.io`
            // CRD kinds the manifest matched — one "Helm" box either way, not two.
            let releases = ("HelmReleases", self.count(ResourceKind::HelmReleases));
            match extension_sections.iter_mut().find(|(name, _)| *name == "Helm") {
                Some((_, items)) => items.push(releases),
                None => {
                    extension_sections.push(("Helm", vec![releases]));
                    extension_sections.sort_by_key(|(name, _)| *name);
                }
            }
        }
        // A dashboard tile, first in its category, but only once the category
        // is real (at least one of its CRD kinds is actually installed) — an
        // enabled extension whose CRDs aren't present still contributes
        // nothing, same as any other extension kind.
        for category in self.dashboard_categories() {
            if let Some((_, items)) = extension_sections.iter_mut().find(|(name, _)| *name == category) {
                items.insert(0, (category, 0));
            }
        }
        sections.extend(extension_sections);
        sections
    }

    /// One section per distinct category an enabled extension declared, listing
    /// only the kinds among them that the cluster actually has installed (an
    /// extension whose CRD isn't present contributes nothing, not an error).
    /// Counts come from the same background instance-counter the Custom
    /// Resources/API pickers use, so opening one doesn't start a new watch.
    fn extension_sections(&self, extensions_enabled: &[String]) -> Vec<(&'static str, Vec<(&'static str, usize)>)> {
        let mut by_category: BTreeMap<&'static str, Vec<(&'static str, usize)>> = BTreeMap::new();
        for ext_kind in self.extensions.enabled(extensions_enabled) {
            let Some(crd) = self.crds.iter().find(|c| c.group == ext_kind.group && c.kind == ext_kind.kind) else { continue };
            let category = k8s::leak(&ext_kind.category);
            let count = self.counts.get(crd.group, &crd.plural).known_or(0);
            by_category.entry(category).or_default().push((crd.kind, count));
        }
        by_category.into_iter().collect()
    }

    /// Extension kinds are counted the same lazy, budgeted way as any other
    /// CRD type: only while something wants them. The Overview always wants
    /// them, so they don't sit at "…" on the one screen most people leave open.
    pub(crate) fn want_extension_counts(&self, extensions_enabled: &[String]) -> Vec<String> {
        self.extensions
            .enabled(extensions_enabled)
            .filter_map(|ext_kind| self.crds.iter().find(|c| c.group == ext_kind.group && c.kind == ext_kind.kind))
            .map(|crd| k8s::count_key(crd.group, &crd.plural))
            .collect()
    }

    /// Every category with a dashboard right now (native or a loaded
    /// manifest's `[[extension.dashboard]]`), for the `:` command menu and
    /// the Overview tile lookup (see `extensions::dashboards::categories`).
    pub(crate) fn dashboard_categories(&self) -> Vec<&'static str> {
        self.extensions.dashboards.clone()
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
    /// first, else a discovered CRD group's tile, else one CRD kind an extension
    /// placed directly on the Overview (matched by its own `kind`, e.g.
    /// `Kustomization`, rather than by group like the Custom Resources picker).
    pub(crate) fn kind_for_tile_label(&self, label: &str) -> Option<ResourceKind> {
        ResourceKind::from_label(label)
            .or_else(|| self.crds.iter().find(|c| c.group == label).map(|c| ResourceKind::CustomResourceGroup(c.group)))
            .or_else(|| self.crds.iter().position(|c| c.kind == label).map(|i| ResourceKind::CustomResource(i, self.crds[i].kind)))
            .or_else(|| self.dashboard_categories().into_iter().find(|c| *c == label).map(ResourceKind::ExtensionDashboard))
    }
}
