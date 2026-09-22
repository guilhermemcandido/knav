//! How many objects each resource type has, for the lists of types (custom resources, API
//! resources), so a type is not opened blind. Each type costs one request for a single item:
//! the API server reports how many more there would be.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use kube::Client;
use serde_json::Value;
use tokio::{sync::Notify, task::JoinHandle};

use super::ApiInfo;

/// What is known about one type's object count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    /// Not asked for yet.
    Loading,
    /// The server would not say (no permission, or it does not report a total).
    Unknown,
    Known(usize),
}

impl Count {
    pub fn text(self) -> String {
        match self {
            Count::Loading => "…".into(),
            Count::Unknown => "?".into(),
            Count::Known(n) => n.to_string(),
        }
    }

    /// The number to sort by; unknown counts sort first.
    pub fn sort_key(self) -> i64 {
        match self {
            Count::Known(n) => n as i64,
            _ => -1,
        }
    }
}

/// The counts found so far, shared with the lists that show them. They are kept per namespace,
/// so going back to one shows its numbers at once.
#[derive(Clone, Default)]
pub struct InstanceCounts {
    map: Arc<Mutex<HashMap<(Option<String>, String), (Count, Instant)>>>,
    /// The namespace the lists show (`None`: all).
    scope: Arc<Mutex<Option<String>>>,
}

pub fn count_key(group: &str, plural: &str) -> String {
    format!("{group}/{plural}")
}

impl InstanceCounts {
    fn scope(&self) -> Option<String> {
        self.scope.lock().ok().and_then(|s| s.clone())
    }

    pub fn get(&self, group: &str, plural: &str) -> Count {
        let key = (self.scope(), count_key(group, plural));
        self.map.lock().ok().and_then(|m| m.get(&key).map(|(c, _)| *c)).unwrap_or(Count::Loading)
    }

    fn stale(&self, key: &str) -> bool {
        let key = (self.scope(), key.to_string());
        self.map.lock().ok().and_then(|m| m.get(&key).map(|(_, at)| at.elapsed() > FRESH)).unwrap_or(true)
    }
}

/// Counting on demand: only the types on screen (and one screen more) are asked about, a few at a
/// time, and each answer is trusted for a minute. Clusters with thousands of custom resource
/// types are never asked about all at once.
pub struct Counter {
    scope: Arc<Mutex<Option<String>>>,
    wanted: Arc<Mutex<Vec<String>>>,
    changed: Arc<Notify>,
    task: JoinHandle<()>,
}

impl Drop for Counter {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// How many types are asked about at once.
const CONCURRENCY: usize = 8;
/// How long a count is trusted before it is asked for again.
const FRESH: Duration = Duration::from_secs(60);
/// How often the wanted types are looked at again when nothing changed.
const TICK: Duration = Duration::from_secs(5);

impl Counter {
    /// `types` is every type that can be counted, by `count_key`.
    pub fn start(client: Client, types: HashMap<String, ApiInfo>, counts: InstanceCounts) -> Self {
        let scope = Arc::new(Mutex::new(None::<String>));
        let wanted = Arc::new(Mutex::new(Vec::<String>::new()));
        let changed = Arc::new(Notify::new());
        let task = {
            let (scope, wanted, changed) = (Arc::clone(&scope), Arc::clone(&wanted), Arc::clone(&changed));
            tokio::spawn(async move {
                loop {
                    let namespace = scope.lock().ok().and_then(|s| s.clone());
                    let due: Vec<&ApiInfo> = wanted
                        .lock()
                        .map(|w| w.iter().filter(|k| counts.stale(k)).filter_map(|k| types.get(k)).filter(|a| a.verbs.is_empty() || a.verbs.iter().any(|v| v == "list")).collect())
                        .unwrap_or_default();
                    if due.is_empty() {
                        tokio::select! {
                            _ = tokio::time::sleep(TICK) => {}
                            _ = changed.notified() => {}
                        }
                        continue;
                    }
                    let round = futures::stream::iter(due).for_each_concurrent(CONCURRENCY, |api| {
                        let (client, counts, namespace) = (client.clone(), counts.clone(), namespace.clone());
                        async move {
                            let found = fetch_count(&client, api, namespace.as_deref()).await;
                            if let Ok(mut map) = counts.map.lock() {
                                map.insert((namespace.clone(), count_key(api.group, api.plural)), (found, Instant::now()));
                            }
                        }
                    });
                    // A change of what is wanted or of the namespace drops the round and starts over.
                    tokio::select! {
                        _ = round => {}
                        _ = changed.notified() => {}
                    }
                }
            })
        };
        Counter { scope, wanted, changed, task }
    }

    /// The types to count now (the ones on screen); a different set starts a new round.
    pub fn want(&self, keys: Vec<String>) {
        if let Ok(mut wanted) = self.wanted.lock()
            && *wanted != keys
        {
            *wanted = keys;
            self.changed.notify_one();
        }
    }

    /// Counts objects of this namespace only (`None`: all of them); namespaced types only.
    pub fn set_namespace(&self, counts: &InstanceCounts, namespace: Option<&str>) {
        if let Ok(mut scope) = self.scope.lock()
            && scope.as_deref() != namespace
        {
            *scope = namespace.map(str::to_string);
            if let Ok(mut shown) = counts.scope.lock() {
                *shown = scope.clone();
            }
            self.changed.notify_one();
        }
    }
}

fn percent_encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

async fn fetch_count(client: &Client, api: &ApiInfo, namespace: Option<&str>) -> Count {
    let root = if api.group.is_empty() { format!("/api/{}", api.version) } else { format!("/apis/{}/{}", api.group, api.version) };
    let path = match namespace.filter(|_| api.namespaced) {
        Some(ns) => format!("{root}/namespaces/{}/{}?limit=1", percent_encode(ns), api.plural),
        None => format!("{root}/{}?limit=1", api.plural),
    };
    let Ok(request) = http::Request::get(path).header(http::header::ACCEPT, "application/json;as=PartialObjectMetadataList;g=meta.k8s.io;v=v1").body(Vec::new()) else { return Count::Unknown };
    let Ok(text) = client.request_text(request).await else { return Count::Unknown };
    let Ok(list) = serde_json::from_str::<Value>(&text) else { return Count::Unknown };
    parse_count(&list)
}

/// A one-item list page as a total: the item, plus how many the server says remain.
fn parse_count(list: &Value) -> Count {
    let shown = list.get("items").and_then(Value::as_array).map_or(0, Vec::len);
    let meta = list.get("metadata");
    let more = meta.and_then(|m| m.get("continue")).and_then(Value::as_str).is_some_and(|c| !c.is_empty());
    match (more, meta.and_then(|m| m.get("remainingItemCount")).and_then(Value::as_u64)) {
        (false, _) => Count::Known(shown),
        (true, Some(remaining)) => Count::Known(shown + remaining as usize),
        (true, None) => Count::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn one_page_that_ends_is_the_whole_count() {
        assert_eq!(parse_count(&json!({"items": [], "metadata": {}})), Count::Known(0));
        assert_eq!(parse_count(&json!({"items": [{}], "metadata": {}})), Count::Known(1));
    }

    #[test]
    fn a_page_with_more_adds_what_the_server_says_remains() {
        assert_eq!(parse_count(&json!({"items": [{}], "metadata": {"continue": "abc", "remainingItemCount": 41}})), Count::Known(42));
        assert_eq!(parse_count(&json!({"items": [{}], "metadata": {"continue": "abc"}})), Count::Unknown);
    }

    #[test]
    fn counts_have_short_text_and_sort_unknowns_first() {
        assert_eq!((Count::Loading.text(), Count::Unknown.text(), Count::Known(7).text()), ("…".to_string(), "?".to_string(), "7".to_string()));
        assert!(Count::Unknown.sort_key() < Count::Known(0).sort_key());
    }
}
