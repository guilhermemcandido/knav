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

/// Watches every `K` in the cluster into an in-memory store, reconnecting with backoff.
pub fn watch_store<K>(client: Client) -> (reflector::Store<K>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let (store, _, handle) = watch_live(client);
    (store, handle)
}

/// As `watch_store`, but only objects matching `field_selector` (e.g. Helm's release
/// Secrets, `type=helm.sh/release.v1`) — for a kind that would otherwise mean pulling
/// down everything of that type just to keep the handful that matter.
pub fn watch_store_selected<K>(client: Client, field_selector: &str) -> (reflector::Store<K>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let api: Api<K> = Api::all(client);
    let (reader, writer) = reflector::store();
    let stream = watcher(api, watcher::Config::default().fields(field_selector)).default_backoff().reflect(writer);
    let handle = tokio::spawn(async move {
        let mut stream = stream.applied_objects().boxed();
        while stream.next().await.is_some() {
            CHANGES.fetch_add(1, Ordering::Relaxed);
        }
    });
    (reader, handle)
}

/// As `watch_store`, plus the queue of which objects changed since it was last taken.
pub fn watch_live<K>(client: Client) -> (reflector::Store<K>, Arc<Feed>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let api: Api<K> = Api::all(client);
    let (reader, writer) = reflector::store();
    let feed = Arc::new(Feed::default());
    let noted = Arc::clone(&feed);
    // After the reflector, so a queued key always finds its object in the store.
    // managedFields are often a third to a half of an object and nothing here shows them.
    let stream = watcher(api, watcher::Config::default()).default_backoff().modify(|object| object.meta_mut().managed_fields = None).reflect(writer).inspect(move |event| {
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

/// An object's place in the sort order: its namespace and name.
pub type Key = (Option<String>, String);

/// Which objects a watch touched since the last `take`, so a kept list can follow
/// them one by one. A relist, or more changes than are worth tracking, says "everything".
pub struct Feed {
    /// (everything may have changed, the keys touched otherwise)
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
    pub(crate) fn note<K: Resource>(&self, event: &watcher::Event<K>) {
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
            // A list in progress or finished replaces the store's contents wholesale.
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

/// Everything in the store ordered by namespace then name, so a selected row keeps
/// pointing at the same object between refreshes (the store has no order of its own).
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

/// How many counting watches may still be on their first list.
static STARTUP: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// How many `K` exist, kept up to date from a metadata-only watch: no spec or data is
/// downloaded, and only each object's uid is held (a Secret's contents never arrive), so it
/// is cheap to run for every kind just to show a count.
pub fn watch_count<K>(client: Client) -> Arc<std::sync::atomic::AtomicUsize>
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    use kube::core::PartialObjectMeta;
    use std::collections::HashSet;
    let api: Api<PartialObjectMeta<K>> = Api::all(client);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stream = watcher(api, watcher::Config::default()).default_backoff();
    let seen = Arc::clone(&count);
    tokio::spawn(async move {
        // Only a few first lists run at once, so opening a cluster does not send dozens together.
        let mut turn = STARTUP.acquire().await.ok();
        let (mut live, mut listing): (HashSet<String>, HashSet<String>) = Default::default();
        let uid = |object: &PartialObjectMeta<K>| object.metadata.uid.clone().unwrap_or_else(|| format!("{:?}/{:?}", object.metadata.namespace, object.metadata.name));
        let mut stream = stream.boxed();
        while let Some(event) = stream.next().await {
            if turn.is_some() && !matches!(event, Ok(watcher::Event::Init | watcher::Event::InitApply(_))) {
                turn = None;
            }
            match event {
                Ok(watcher::Event::Init) => listing.clear(),
                Ok(watcher::Event::InitApply(object)) => {
                    listing.insert(uid(&object));
                }
                Ok(watcher::Event::InitDone) => std::mem::swap(&mut live, &mut listing),
                Ok(watcher::Event::Apply(object)) => {
                    live.insert(uid(&object));
                }
                Ok(watcher::Event::Delete(object)) => {
                    live.remove(&uid(&object));
                }
                Err(_) => continue,
            }
            seen.store(live.len(), Ordering::Relaxed);
            CHANGES.fetch_add(1, Ordering::Relaxed);
        }
    });
    count
}
