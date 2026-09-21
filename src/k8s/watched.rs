
use kube::{
    Client, Resource,
    api::Api,
    runtime::reflector,
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
    /// Whether wide-only columns are wanted (only table-backed kinds have any).
    fn set_wide(&self, _wide: bool) {}
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
