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

/// Tells the UI something changed that no reflector reported (a table kind's watch).
pub fn note_change() {
    CHANGES.fetch_add(1, Ordering::Relaxed);
}

pub fn changes() -> u64 {
    CHANGES.load(Ordering::Relaxed)
}

/// Whether the API server streams a watch's first list (Kubernetes 1.27+ with the
/// WatchList feature), found once per session by `detect_streaming`.
static STREAMING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How every watch starts: a streamed first list when the server can, else one list
/// from its cache (`resourceVersion=0`), not a page-by-page read from etcd.
fn first_list() -> watcher::Config {
    if STREAMING.load(Ordering::Relaxed) { watcher::Config::default().streaming_lists() } else { watcher::Config::default().any_semantic() }
}

/// Tries a streamed list of Namespaces, a small kind, and remembers whether it finished.
/// An older server may ignore the request rather than refuse it, so only the end of the
/// initial list counts; an error or no end within a few seconds means plain lists.
/// `KNAV_PLAIN_LISTS=1` skips it, for a cluster that misbehaves with streaming.
pub async fn detect_streaming(client: &Client) {
    let plain = std::env::var_os("KNAV_PLAIN_LISTS").is_some();
    STREAMING.store(!plain && streams(client).await, Ordering::Relaxed);
}

async fn streams(client: &Client) -> bool {
    use k8s_openapi::api::core::v1::Namespace;
    let api: Api<Namespace> = Api::all(client.clone());
    let mut stream = watcher(api, watcher::Config::default().streaming_lists()).boxed();
    let finished = async {
        while let Some(event) = stream.next().await {
            match event {
                Ok(watcher::Event::InitDone) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        false
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), finished).await.unwrap_or(false)
}

/// Watches every `K` in the cluster into an in-memory store, reconnecting with backoff.
pub fn watch_store<K>(client: Client) -> (reflector::Store<K>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let (store, _, handle) = watch_live(client);
    (store, handle)
}

/// `watch_store` that starts listing only once `after` resolves, so a big, busy kind
/// (events) doesn't compete with the lists the first screen waits for.
pub fn watch_store_after<K>(client: Client, after: impl std::future::Future<Output = ()> + Send + 'static) -> (reflector::Store<K>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let api: Api<K> = Api::all(client);
    let (reader, writer) = reflector::store();
    let handle = tokio::spawn(async move {
        after.await;
        let stream = watcher(api, first_list()).default_backoff().modify(|object| object.meta_mut().managed_fields = None).reflect(writer);
        let mut stream = stream.applied_objects().boxed();
        while stream.next().await.is_some() {
            CHANGES.fetch_add(1, Ordering::Relaxed);
        }
    });
    (reader, handle)
}

/// Like `watch_store`, for objects matching `field_selector` only, so a kind with
/// few interesting objects (Helm's release Secrets) isn't downloaded whole.
pub fn watch_store_selected<K>(client: Client, field_selector: &str) -> (reflector::Store<K>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let api: Api<K> = Api::all(client);
    let (reader, writer) = reflector::store();
    let stream = watcher(api, first_list().fields(field_selector)).default_backoff().reflect(writer);
    let handle = tokio::spawn(async move {
        let mut stream = stream.applied_objects().boxed();
        while stream.next().await.is_some() {
            CHANGES.fetch_add(1, Ordering::Relaxed);
        }
    });
    (reader, handle)
}

/// Like `watch_store`, plus a queue of the objects changed since it was last taken.
pub fn watch_live<K>(client: Client) -> (reflector::Store<K>, Arc<Feed>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    watch_live_in(client, None)
}

/// `watch_live` limited to one namespace, through a field selector, so it works for any
/// kind and a big cluster's other namespaces are never fetched.
pub fn watch_live_in<K>(client: Client, namespace: Option<&str>) -> (reflector::Store<K>, Arc<Feed>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let api: Api<K> = Api::all(client);
    let config = match namespace {
        Some(ns) => first_list().fields(&format!("metadata.namespace={ns}")),
        None => first_list(),
    };
    let (reader, writer) = reflector::store();
    let feed = Arc::new(Feed::default());
    let noted = Arc::clone(&feed);
    // After the reflector, so a queued key always finds its object in the store.
    // managedFields are often a third of an object and nothing shows them.
    let stream = watcher(api, config).default_backoff().modify(|object| object.meta_mut().managed_fields = None).reflect(writer).inspect(move |event| {
        if let Ok(event) = event {
            noted.note(event);
        }
    });
    let handle = tokio::spawn(async move {
        let mut stream = stream.applied_objects().boxed();
        while stream.next().await.is_some() {
            CHANGES.fetch_add(1, Ordering::Relaxed);
        }
    });
    (reader, feed, handle)
}

pub type Key = (Option<String>, String);

/// Which objects a watch touched since the last `take`. A relist, or more changes
/// than are worth tracking, means everything.
pub struct Feed {
    /// Whether everything may have changed, and the keys touched otherwise.
    pending: std::sync::Mutex<(bool, std::collections::HashSet<Key>)>,
}

/// Past this many pending keys a full rebuild is cheaper than replaying them.
const MAX_PENDING: usize = 4096;

impl Default for Feed {
    fn default() -> Self {
        Feed { pending: std::sync::Mutex::new((true, Default::default())) }
    }
}

impl Feed {
    pub fn note<K: Resource>(&self, event: &watcher::Event<K>) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        match event {
            watcher::Event::Apply(object) | watcher::Event::Delete(object) => {
                if !pending.0 {
                    let meta = object.meta();
                    pending.1.insert((meta.namespace.clone(), meta.name.clone().unwrap_or_default()));
                    if pending.1.len() > MAX_PENDING {
                        *pending = (true, Default::default());
                    }
                }
            }
            // A relist replaces the store's contents wholesale.
            _ => *pending = (true, Default::default()),
        }
    }

    /// What changed since the last call: `None` when everything may have.
    pub fn take(&self) -> Option<Vec<Key>> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let (all, keys) = std::mem::take(&mut *pending);
        (!all).then(|| keys.into_iter().collect())
    }
}

/// Everything in the store by namespace then name, so a selected row keeps pointing
/// at the same object between refreshes.
pub fn sorted<K>(store: &reflector::Store<K>) -> Vec<Arc<K>>
where
    K: Resource + Clone + Send + Sync,
    K::DynamicType: Eq + std::hash::Hash + Clone,
{
    let mut items = store.state();
    super::parallel::par_sort_by(&mut items, |a, b| {
        let (a, b) = (a.meta(), b.meta());
        (a.namespace.as_deref(), a.name.as_deref()).cmp(&(b.namespace.as_deref(), b.name.as_deref()))
    });
    items
}

#[cfg(test)]
mod streaming_probe {
    #[tokio::test]
    #[ignore = "needs a cluster"]
    async fn the_current_cluster_says_whether_it_streams() {
        let client = kube::Client::try_default().await.unwrap();
        println!("streams lists: {}", super::streams(&client).await);
    }
}
