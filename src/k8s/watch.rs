//! Live watches: one reflector per kind, a change counter, and the sorted view of a store.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use futures::StreamExt;
use kube::{
    Client, Resource,
    api::Api,
    runtime::{WatchStreamExt, reflector, watcher},
};
use serde::de::DeserializeOwned;
use tokio::task::JoinHandle;

/// Bumped on every change any watch sees, so the UI can tell when to recompute.
static CHANGES: AtomicU64 = AtomicU64::new(0);

pub fn changes() -> u64 {
    CHANGES.load(Ordering::Relaxed)
}

/// Watches every `K` in the cluster into an in-memory store, reconnecting with backoff.
pub fn watch_store<K>(client: Client) -> (reflector::Store<K>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let api: Api<K> = Api::all(client);
    let (reader, writer) = reflector::store();
    let stream = watcher(api, watcher::Config::default()).default_backoff().reflect(writer).applied_objects();
    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {
            CHANGES.fetch_add(1, Ordering::Relaxed);
        }
    });
    (reader, handle)
}

/// Everything in the store ordered by namespace then name, so a selected row keeps
/// pointing at the same object between refreshes (the store has no order of its own).
pub fn sorted<K>(store: &reflector::Store<K>) -> Vec<Arc<K>>
where
    K: Resource + Clone,
    K::DynamicType: Eq + std::hash::Hash + Clone,
{
    let mut items = store.state();
    items.sort_by(|a, b| {
        let (a, b) = (a.meta(), b.meta());
        (a.namespace.as_deref(), a.name.as_deref()).cmp(&(b.namespace.as_deref(), b.name.as_deref()))
    });
    items
}
