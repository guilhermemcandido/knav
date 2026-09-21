use std::sync::Arc;

use kube::{
    Client, Resource,
    api::Api,
    runtime::reflector,
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::task::JoinHandle;

use super::*;

/// Type-erased handle to a watched kind's store, so `Catalog` can hold many kinds
/// in one `Vec` and read counts, rows and manifests without a match per kind.
pub trait CatalogKind: Send + Sync {
    fn count(&self) -> usize;
    fn rows(&self) -> Vec<GenericRow>;
    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value>;
    /// The kind's extra column headers (see `describe`), even with no rows.
    fn headers(&self) -> Vec<&'static str> {
        Vec::new()
    }
    /// Every object's manifest, in `namespace` (cluster-scoped objects always), for
    /// the relations view.
    fn manifests(&self, namespace: Option<&str>) -> Vec<serde_yaml::Value> {
        let _ = namespace;
        (0..self.count()).filter_map(|i| self.spec_at(i)).collect()
    }
    /// How the kind's objects are doing, for kinds that have a notion of it.
    fn health(&self) -> Option<Health> {
        None
    }
    /// Whether wide-only columns are wanted (only table-backed kinds have any).
    fn set_wide(&self, _wide: bool) {}
}

pub struct WatchedKind<K: Resource<DynamicType = ()> + Clone + 'static> {
    store: reflector::Store<K>,
    /// The sorted objects as of a change count, so asking again costs nothing until a watch reports.
    sorted: std::sync::Mutex<Option<(u64, Arc<Vec<Arc<K>>>)>>,
}

impl<K: Resource<DynamicType = ()> + Clone + 'static> WatchedKind<K> {
    pub fn from_store(store: reflector::Store<K>) -> Self {
        WatchedKind { store, sorted: std::sync::Mutex::new(None) }
    }

    fn items(&self) -> Arc<Vec<Arc<K>>> {
        let version = crate::k8s::changes();
        let mut slot = self.sorted.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            Some((seen, items)) if *seen == version => Arc::clone(items),
            _ => {
                let items = Arc::new(snapshot_generic(&self.store));
                *slot = Some((version, Arc::clone(&items)));
                items
            }
        }
    }
}

impl<K> CatalogKind for WatchedKind<K>
where
    K: Resource<DynamicType = ()> + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + crate::k8s::describe::Extras + Default + 'static,
{
    fn count(&self) -> usize {
        self.items().len()
    }

    fn headers(&self) -> Vec<&'static str> {
        K::headers()
    }

    fn rows(&self) -> Vec<GenericRow> {
        self.items().iter().map(|item| generic_row(item.as_ref())).collect()
    }

    fn manifests(&self, namespace: Option<&str>) -> Vec<serde_yaml::Value> {
        use kube::ResourceExt;
        self.items().iter().filter(|item| namespace.is_none() || item.namespace().is_none() || item.namespace().as_deref() == namespace).map(|item| manifest_value(item.as_ref())).collect()
    }

    fn health(&self) -> Option<Health> {
        let mut health = Health::default();
        let mut any = false;
        for item in self.items().iter() {
            if let Some(tone) = item.tone() {
                any = true;
                health.add(tone);
            }
        }
        any.then_some(health)
    }

    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value> {
        self.items().get(index).map(|item| manifest_value(item.as_ref()))
    }
}

/// Spawns a live watch for kind `K` and boxes it as a `CatalogKind`,
/// the one-liner most Catalog entries use.
pub fn watch_kind<K>(client: Client) -> (Box<dyn CatalogKind>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + crate::k8s::describe::Extras + Default + 'static,
{
    let (store, handle) = super::watch::watch_store::<K>(client);
    (Box::new(WatchedKind::from_store(store)), handle)
}

/// A discovered CRD kind: enough to build an `ApiResource` and list it. Found once
/// at startup, so a CRD installed later appears after a restart.
#[derive(Clone)]
pub struct CrdInfo {
    pub group: &'static str,
    pub kind: &'static str,
    pub plural: String,
    pub version: String,
    pub namespaced: bool,
}

/// Lists the installed CRDs. Prefers the storage version, and skips CRDs with no
/// served version. `group` and `kind` are leaked to `&'static str` once at startup
/// so `ResourceKind::CustomResource` can carry plain labels.
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
