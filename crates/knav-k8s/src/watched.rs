use std::sync::Arc;

use kube::{
    Client, Resource,
    runtime::reflector,
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::task::JoinHandle;

use super::*;

/// Type-erased handle to a watched kind's store, so `Catalog` can hold many kinds
/// in one `Vec` and read counts, rows and manifests without a match per kind.
pub trait CatalogKind: Send + Sync {
    fn count(&self) -> usize;
    /// Whether the first full list has arrived, so what is read is not just partial.
    fn ready(&self) -> bool {
        true
    }
    /// Resolves once the first list has arrived.
    fn wait_ready(&self) -> futures::future::BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }
    fn rows(&self) -> Vec<Arc<GenericRow>>;
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
    /// Narrows what is fetched to one namespace (`None`: all), for kinds that fetch on demand.
    fn set_namespace(&self, _namespace: Option<&str>) {}
}

pub struct WatchedKind<K: Resource<DynamicType = ()> + Clone + 'static> {
    /// The sorted objects with their rows, following the watch one change at a time.
    kept: Kept<K, GenericRow>,
}

impl<K: Resource<DynamicType = ()> + Clone + Send + Sync + crate::describe::Extras + 'static> WatchedKind<K> {
    pub fn new(store: reflector::Store<K>, feed: Arc<Feed>) -> Self {
        WatchedKind { kept: Kept::new(store, feed, generic_row::<K>) }
    }

    fn items(&self) -> Arc<Vec<Item<K, GenericRow>>> {
        self.kept.items()
    }
}

impl<K> CatalogKind for WatchedKind<K>
where
    K: Resource<DynamicType = ()> + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + crate::describe::Extras + Default + 'static,
{
    fn count(&self) -> usize {
        self.kept.store.len()
    }

    fn ready(&self) -> bool {
        use futures::FutureExt;
        self.kept.store.wait_until_ready().now_or_never().is_some_and(|r| r.is_ok())
    }

    fn wait_ready(&self) -> futures::future::BoxFuture<'static, ()> {
        let store = self.kept.store.clone();
        Box::pin(async move {
            let _ = store.wait_until_ready().await;
        })
    }

    fn headers(&self) -> Vec<&'static str> {
        K::headers()
    }

    fn rows(&self) -> Vec<Arc<GenericRow>> {
        self.items().iter().map(|(_, row)| Arc::clone(row)).collect()
    }

    fn manifests(&self, namespace: Option<&str>) -> Vec<serde_yaml::Value> {
        use kube::ResourceExt;
        self.items().iter().map(|(item, _)| item).filter(|item| namespace.is_none() || item.namespace().is_none() || item.namespace().as_deref() == namespace).map(|item| manifest_value(item.as_ref())).collect()
    }

    fn health(&self) -> Option<Health> {
        let mut health = Health::default();
        let mut any = false;
        for (item, _) in self.items().iter() {
            if let Some(tone) = item.tone() {
                any = true;
                health.add(tone);
            }
        }
        any.then_some(health)
    }

    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value> {
        self.items().get(index).map(|(item, _)| manifest_value(item.as_ref()))
    }
}

/// Spawns a live watch for kind `K` and boxes it as a `CatalogKind`,
/// the one-liner most Catalog entries use.
pub fn watch_kind<K>(client: Client) -> (Box<dyn CatalogKind>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + crate::describe::Extras + Default + 'static,
{
    let (store, feed, handle) = super::watch::watch_live::<K>(client);
    (Box::new(WatchedKind::new(store, feed)), handle)
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
