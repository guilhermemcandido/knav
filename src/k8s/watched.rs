
use futures::StreamExt;
use kube::{
    Client, Resource,
    api::{Api, ApiResource, DynamicObject},
    runtime::{WatchStreamExt, reflector, watcher},
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::task::JoinHandle;

use super::*;

/// Type-erased handle to a live-watched resource kind's store — lets
/// `Catalog` hold ~20 different `K`s in one `Vec` and treat them
/// uniformly (count for the Overview tile, rows for the list view, a
/// single object's manifest for the spec view) without a match arm per
/// kind at every call site.
pub trait CatalogKind: Send + Sync {
    fn count(&self) -> usize;
    fn rows(&self) -> Vec<GenericRow>;
    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value>;
    /// The kind's extra column headers (see `describe`), even with no rows.
    fn headers(&self) -> Vec<&'static str> {
        Vec::new()
    }
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
    K: Resource<DynamicType = ()> + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + crate::describe::Extras + Default + 'static,
{
    fn count(&self) -> usize {
        self.store.state().len()
    }

    fn headers(&self) -> Vec<&'static str> {
        K::headers()
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
    K: Resource<DynamicType = ()> + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + crate::describe::Extras + Default + 'static,
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
