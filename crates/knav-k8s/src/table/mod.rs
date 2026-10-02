//! Any resource shown through the server's Table view: the columns `kubectl get`
//! prints, including custom resources' printer columns.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kube::{
    Client,
    api::{Api, ApiResource, DynamicObject},
};
use serde_json::Value;
use tokio::task::JoinHandle;

use super::*;
use crate::describe::{Col, Tone};

mod watch;

/// Seconds between refreshes of listed API types, set from the config.
static REFRESH_SECONDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(2);

pub fn set_refresh_seconds(seconds: u64) {
    REFRESH_SECONDS.store(seconds.max(1), Ordering::Relaxed);
}

#[derive(Clone)]
pub(super) struct TableColumn {
    pub(super) name: &'static str,
    /// 0 is shown always; higher is wide-only (`kubectl get -o wide`).
    pub(super) priority: i64,
    pub(super) numeric: bool,
}

pub(super) struct TableRow {
    pub(super) cells: Vec<String>,
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) uid: String,
    pub(super) owners: Vec<String>,
    pub(super) created: Option<k8s_openapi::jiff::Timestamp>,
    pub(super) labels: String,
}

#[derive(Default)]
pub(super) struct TableData {
    pub(super) columns: Vec<TableColumn>,
    pub(super) rows: Vec<TableRow>,
    pub(super) error: Option<String>,
    /// More pages of the first load are still coming.
    pub(super) loading: bool,
    /// The namespace the rows were fetched for (`None`: all).
    pub(super) scope: Option<String>,
    pub(super) complete: bool,
}

impl TableData {
    /// Whether the rows held include everything a fetch of `namespace` would return.
    fn covers(&self, namespace: Option<&str>) -> bool {
        self.complete && (self.scope.is_none() || self.scope.as_deref() == namespace)
    }
}

/// A fetched object and when it was fetched.
type Fetched = (std::time::Instant, serde_yaml::Value);

/// One resource type shown through the server's Table view, kept current by a watch
/// while it is open. With a namespace selected, only that namespace is fetched.
pub struct TableKind {
    data: Arc<Mutex<TableData>>,
    wide: AtomicBool,
    handle: JoinHandle<()>,
    client: Client,
    resource: ApiResource,
    namespaced: bool,
    /// Full objects fetched for the info view and actions, cached.
    objects: Arc<Mutex<HashMap<(String, String), Fetched>>>,
    /// Objects being fetched for their state, so a redraw doesn't ask again.
    fetching: Arc<Mutex<std::collections::HashSet<(String, String)>>>,
    /// The namespace to fetch (`None` for all), and a nudge to refetch when it changes.
    scope: Arc<Mutex<Option<String>>>,
    changed: Arc<tokio::sync::Notify>,
}

impl Drop for TableKind {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl TableKind {
    pub fn start(client: Client, api: &ApiInfo) -> Self {
        let resource = api.resource();
        let data = Arc::new(Mutex::new(TableData::default()));
        let scope = Arc::new(Mutex::new(None::<String>));
        let changed = Arc::new(tokio::sync::Notify::new());
        let handle = {
            let (client, resource) = (client.clone(), resource.clone());
            let (data, scope, changed) = (Arc::clone(&data), Arc::clone(&scope), Arc::clone(&changed));
            let namespaced = api.namespaced;
            tokio::spawn(async move {
                loop {
                    let started = std::time::Instant::now();
                    let wanted = if namespaced { scope.lock().ok().and_then(|s| s.clone()) } else { None };
                    // A change of namespace drops the fetch in flight and starts over.
                    let listed = tokio::select! {
                        listed = refresh_table(&client, &resource, &data, wanted.as_deref()) => listed,
                        _ = changed.notified() => continue,
                    };
                    // The watch keeps the rows current until it fails, gets too old or the
                    // namespace changes; then the list is read again.
                    let mut ended = listed.is_none();
                    if let Some(mut version) = listed {
                        ended = tokio::select! {
                            end = watch::watch_table(&client, &resource, &data, wanted.as_deref(), &mut version) => end,
                            _ = changed.notified() => false,
                        };
                    }
                    if ended {
                        // Repeated failures back off, so the server isn't hammered with lists.
                        let pause = Duration::from_secs(REFRESH_SECONDS.load(Ordering::Relaxed)).max(started.elapsed() * 3);
                        tokio::select! {
                            _ = tokio::time::sleep(pause) => {}
                            _ = changed.notified() => {}
                        }
                    }
                }
            })
        };
        TableKind { data, wide: AtomicBool::new(false), handle, client, resource, namespaced: api.namespaced, objects: Arc::default(), fetching: Arc::default(), scope, changed }
    }

    fn shows(&self, column: &TableColumn) -> bool {
        // NAME and AGE have columns of their own in the list.
        !column.name.eq_ignore_ascii_case("name") && !column.name.eq_ignore_ascii_case("age") && (column.priority == 0 || self.wide.load(Ordering::Relaxed))
    }
}

/// Rows per request; the rest follow with a continue token.
const PAGE: usize = 500;

/// The path of `resource`'s list, in `namespace` or across all of them.
pub(super) fn list_path(resource: &ApiResource, namespace: Option<&str>) -> String {
    let root = if resource.group.is_empty() { format!("/api/{}", resource.version) } else { format!("/apis/{}/{}", resource.group, resource.version) };
    match namespace {
        Some(ns) => format!("{root}/namespaces/{}/{}", percent_encode(ns), resource.plural),
        None => format!("{root}/{}", resource.plural),
    }
}

/// A page of rows, with the token for the next and the version of the list they belong to.
type Page = (Vec<TableColumn>, Vec<TableRow>, Option<String>, Option<String>);

async fn fetch_page(client: &Client, resource: &ApiResource, namespace: Option<&str>, token: Option<&str>) -> anyhow::Result<Page> {
    let base = list_path(resource, namespace);
    let path = match token {
        Some(token) => format!("{base}?limit={PAGE}&continue={}", percent_encode(token)),
        None => format!("{base}?limit={PAGE}"),
    };
    let request = http::Request::get(path).header(http::header::ACCEPT, "application/json;as=Table;g=meta.k8s.io;v=v1").body(Vec::new())?;
    let table: Value = serde_json::from_str(&client.request_text(request).await?)?;
    let next = table.get("metadata").and_then(|m| m.get("continue")).and_then(|c| c.as_str()).filter(|c| !c.is_empty()).map(str::to_string);
    let version = table.get("metadata").and_then(|m| m.get("resourceVersion")).and_then(|v| v.as_str()).map(str::to_string);
    let (columns, rows) = parse_table(&table)?;
    Ok((columns, rows, next, version))
}

pub(crate) fn percent_encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Reads every page and returns the list's version for a watch to continue from (`None`
/// on failure). Rows appear page by page only when what is held can't stand in meanwhile.
async fn refresh_table(client: &Client, resource: &ApiResource, data: &Mutex<TableData>, namespace: Option<&str>) -> Option<String> {
    let in_place = data.lock().map(|d| d.columns.is_empty() || !d.covers(namespace)).unwrap_or(true);
    if in_place && let Ok(mut d) = data.lock() {
        d.rows.clear();
        d.complete = false;
        d.scope = namespace.map(str::to_string);
    }
    let (mut columns, mut rows, mut token) = (Vec::new(), Vec::new(), None::<String>);
    let mut version = None;
    loop {
        match fetch_page(client, resource, namespace, token.as_deref()).await {
            Ok((page_columns, page_rows, next, page_version)) => {
                version = page_version.or(version);
                if in_place {
                    if let Ok(mut d) = data.lock() {
                        if d.columns.is_empty() {
                            d.columns = page_columns;
                        }
                        d.rows.extend(page_rows);
                        d.error = None;
                        d.loading = next.is_some();
                    }
                } else {
                    if columns.is_empty() {
                        columns = page_columns;
                    }
                    rows.extend(page_rows);
                }
                token = next;
                if token.is_none() {
                    break;
                }
            }
            Err(e) => {
                if let Ok(mut d) = data.lock() {
                    d.error = Some(format!("{e:#}"));
                    d.loading = false;
                }
                return None;
            }
        }
    }
    if let Ok(mut d) = data.lock() {
        if in_place {
            watch::ensure_ordered(&mut d.rows);
            d.complete = true;
        } else {
            watch::ensure_ordered(&mut rows);
            *d = TableData { columns, rows, error: None, loading: false, scope: namespace.map(str::to_string), complete: true };
        }
    }
    version
}

fn cell_text(cell: &Value) -> String {
    match cell {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub(super) fn parse_table(table: &Value) -> anyhow::Result<(Vec<TableColumn>, Vec<TableRow>)> {
    let columns = table
        .get("columnDefinitions")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .map(|c| TableColumn {
            name: leak(&c.get("name").and_then(|n| n.as_str()).unwrap_or("").to_uppercase()),
            priority: c.get("priority").and_then(|p| p.as_i64()).unwrap_or(0),
            numeric: matches!(c.get("type").and_then(|t| t.as_str()), Some("integer" | "number")),
        })
        .collect();
    let rows = table
        .get("rows")
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
        .map(|row| {
            let meta = row.get("object").and_then(|o| o.get("metadata"));
            let text = |key: &str| meta.and_then(|m| m.get(key)).and_then(|v| v.as_str()).map(str::to_string);
            let cells: Vec<String> = row.get("cells").and_then(|c| c.as_array()).into_iter().flatten().map(cell_text).collect();
            let labels = meta.and_then(|m| m.get("labels")).and_then(|l| l.as_object()).map(|l| {
                l.iter().map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or(""))).collect::<Vec<_>>().join(",")
            });
            TableRow {
                name: text("name").or_else(|| cells.first().cloned()).unwrap_or_default(),
                namespace: text("namespace").unwrap_or_else(|| "-".into()),
                uid: text("uid").unwrap_or_default(),
                owners: meta
                    .and_then(|m| m.get("ownerReferences"))
                    .and_then(|o| o.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|o| o.get("uid")?.as_str().map(str::to_string))
                    .collect(),
                created: text("creationTimestamp").and_then(|t| t.parse().ok()),
                labels: labels.filter(|l| !l.is_empty()).unwrap_or_else(|| "-".into()),
                cells,
            }
        })
        .collect();
    Ok((columns, rows))
}

/// The columns a custom resource's state may be in, the most telling first. Flux and
/// cert-manager say READY, Argo CD HEALTH and SYNC STATUS.
const STATUS_COLUMNS: [&str; 7] = ["STATUS", "PHASE", "STATE", "HEALTH", "HEALTH STATUS", "SYNC STATUS", "READY"];

/// A row's state from its status column. A `True`/`False` READY reads as words.
fn status_note(text: &str, ready_column: bool) -> (Tone, String) {
    match (ready_column, text) {
        (true, "True") => (Tone::Good, "Ready".into()),
        (true, "False") => (Tone::Bad, "Not ready".into()),
        (true, "Unknown") => (Tone::Warn, "Ready unknown".into()),
        _ => (cell_tone(text), text.to_string()),
    }
}

fn cell_tone(text: &str) -> Tone {
    match text {
        "Ready" | "Active" | "Bound" | "Available" | "Running" | "Healthy" | "Complete" | "Succeeded" | "Established" | "Approved" => Tone::Good,
        "Pending" | "Terminating" | "Progressing" | "Unknown" | "Released" | "Waiting" => Tone::Warn,
        "Failed" | "Error" | "NotReady" | "Lost" | "CrashLoopBackOff" | "ImagePullBackOff" | "Evicted" | "OOMKilled" | "Denied" => Tone::Bad,
        "Completed" => Tone::Muted,
        _ => Tone::Plain,
    }
}

impl CatalogKind for TableKind {
    fn count(&self) -> usize {
        self.data.lock().map(|d| d.rows.len()).unwrap_or(0)
    }

    fn loaded(&self) -> bool {
        self.data.lock().map(|d| d.complete || d.error.is_some()).unwrap_or(true)
    }

    fn set_wide(&self, wide: bool) {
        self.wide.store(wide, Ordering::Relaxed);
    }

    fn set_namespace(&self, namespace: Option<&str>) {
        if !self.namespaced {
            return;
        }
        if let Ok(mut scope) = self.scope.lock()
            && scope.as_deref() != namespace
        {
            *scope = namespace.map(str::to_string);
            self.changed.notify_one();
            // Rows of another namespace don't belong here: show loading, not a gap.
            if let Ok(mut data) = self.data.lock()
                && !data.covers(namespace)
            {
                data.loading = true;
            }
        }
    }

    fn headers(&self) -> Vec<&'static str> {
        self.data.lock().map(|d| d.columns.iter().filter(|c| self.shows(c)).map(|c| c.name).collect()).unwrap_or_default()
    }

    fn rows(&self) -> Vec<Arc<GenericRow>> {
        let Ok(data) = self.data.lock() else { return Vec::new() };
        let shown: Vec<usize> = (0..data.columns.len()).filter(|&i| self.shows(&data.columns[i])).collect();
        if data.rows.is_empty()
            && let Some(error) = &data.error
        {
            // A resource we can't list (forbidden, gone) says so in the list itself.
            let extras = shown.iter().map(|&i| Col { header: data.columns[i].name, text: String::new(), tone: Tone::Plain, sort: None }).collect();
            return vec![Arc::new(GenericRow {
                namespace: "-".into(),
                name: format!("⚠ {error}"),
                age: "-".into(),
                age_secs: 0,
                extras,
                status: Some((Tone::Bad, "cannot list".into())),
                uid: String::new(),
                owners: Vec::new(),
                labels: String::new(),
            })];
        }
        let status_column = STATUS_COLUMNS.iter().find_map(|name| data.columns.iter().position(|c| c.name == *name));
        let ready_column = status_column.is_some_and(|i| data.columns[i].name == "READY");
        let mut rows = crate::par_map(&data.rows, |row| {
            Arc::new({
                let extras = shown
                    .iter()
                    .map(|&i| {
                        let column = &data.columns[i];
                        let text = row.cells.get(i).cloned().unwrap_or_default();
                        Col { header: column.name, tone: if Some(i) == status_column { cell_tone(&text) } else { Tone::Plain }, sort: column.numeric.then(|| text.parse().ok()).flatten(), text }
                    })
                    .collect();
                let status = status_column.and_then(|i| row.cells.get(i)).filter(|t| !t.is_empty()).map(|text| status_note(text, ready_column));
                GenericRow {
                    namespace: row.namespace.clone(),
                    name: row.name.clone(),
                    age: row.created.map(humanize_age).unwrap_or_else(|| "-".into()),
                    age_secs: row.created.map(|t| (k8s_openapi::jiff::Timestamp::now().as_second() - t.as_second()).max(0)).unwrap_or(i64::MAX),
                    extras,
                    status,
                    uid: row.uid.clone(),
                    owners: row.owners.clone(),
                    labels: row.labels.clone(),
                }
            })
            });
        if data.loading {
            rows.push(Arc::new(GenericRow { namespace: "-".into(), name: format!("… loading more ({} so far)", data.rows.len()), age: "-".into(), age_secs: i64::MAX, extras: shown.iter().map(|&i| Col { header: data.columns[i].name, text: String::new(), tone: Tone::Plain, sort: None }).collect(), status: Some((Tone::Muted, "loading".into())), uid: String::new(), owners: Vec::new(), labels: String::new() }));
        }
        rows
    }

    fn state_of(&self, index: usize) -> crate::describe::Note {
        let key = {
            let data = self.data.lock().ok()?;
            let row = data.rows.get(index)?;
            (row.namespace.clone(), row.name.clone())
        };
        // A stale copy still answers; spec_at refreshes it when the object is opened.
        if let Some((_, value)) = self.objects.lock().ok().and_then(|o| o.get(&key).cloned()) {
            return object_state(&value);
        }
        let mut fetching = self.fetching.lock().ok()?;
        if fetching.insert(key.clone()) {
            let (client, resource, namespaced) = (self.client.clone(), self.resource.clone(), self.namespaced && key.0 != "-");
            let (objects, fetching) = (Arc::clone(&self.objects), Arc::clone(&self.fetching));
            tokio::spawn(async move {
                if let Some(object) = fetch_object(client, &resource, namespaced, &key.0, &key.1).await
                    && let Ok(mut objects) = objects.lock()
                {
                    objects.insert(key.clone(), (std::time::Instant::now(), object));
                }
                if let Ok(mut fetching) = fetching.lock() {
                    fetching.remove(&key);
                }
            });
        }
        None
    }

    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value> {
        let (namespace, name) = {
            let data = self.data.lock().ok()?;
            let row = data.rows.get(index)?;
            (row.namespace.clone(), row.name.clone())
        };
        let key = (namespace.clone(), name.clone());
        let cached = self.objects.lock().ok().and_then(|o| o.get(&key).cloned());
        // A held copy is used at once; an old one is refreshed in the background.
        if let Some((fetched, value)) = cached {
            if fetched.elapsed() > STALE_AFTER
                && let Ok(mut objects) = self.objects.lock()
            {
                objects.insert(key.clone(), (std::time::Instant::now(), value.clone()));
                let (client, resource, namespaced, objects) = (self.client.clone(), self.resource.clone(), self.namespaced && namespace != "-", Arc::clone(&self.objects));
                tokio::spawn(async move {
                    if let Some(fresh) = fetch_object(client, &resource, namespaced, &namespace, &name).await
                        && let Ok(mut objects) = objects.lock()
                    {
                        objects.insert(key, (std::time::Instant::now(), fresh));
                    }
                });
            }
            return Some(value);
        }
        let (client, resource, namespaced) = (self.client.clone(), self.resource.clone(), self.namespaced && namespace != "-");
        let object = tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(fetch_object(client, &resource, namespaced, &namespace, &name)))?;
        if let Ok(mut objects) = self.objects.lock() {
            objects.insert(key, (std::time::Instant::now(), object.clone()));
        }
        Some(object)
    }
}

/// A custom resource's state from its conditions, the way Flux, cert-manager and most
/// operators report it, else from `status.phase` or `status.state`.
pub fn object_state(object: &serde_yaml::Value) -> crate::describe::Note {
    let status = object.get("status")?;
    let conditions = status.get("conditions").and_then(serde_yaml::Value::as_sequence).cloned().unwrap_or_default();
    let field = |c: &serde_yaml::Value, key: &str| c.get(key).and_then(serde_yaml::Value::as_str).unwrap_or("").to_string();
    let condition = |kind: &str| conditions.iter().find(|c| field(c, "type") == kind).map(|c| (field(c, "status"), field(c, "reason")));
    if condition("Stalled").is_some_and(|(s, _)| s == "True") {
        return Some((Tone::Bad, "Stalled".into()));
    }
    if condition("Reconciling").is_some_and(|(s, _)| s == "True") {
        return Some((Tone::Warn, "Reconciling".into()));
    }
    if let Some((state, reason)) = ["Ready", "Available", "Healthy", "Synced"].iter().find_map(|kind| condition(kind)) {
        let progressing = ["Progress", "Reconcil", "Pending", "Install", "Upgrad"].iter().any(|word| reason.contains(word));
        return Some(match state.as_str() {
            "True" => (Tone::Good, "Ready".into()),
            "False" if progressing => (Tone::Warn, "Reconciling".into()),
            "False" => (Tone::Bad, "Not ready".into()),
            _ => (Tone::Warn, "Reconciling".into()),
        });
    }
    let phase = status.get("phase").or_else(|| status.get("state")).and_then(serde_yaml::Value::as_str)?;
    Some((cell_tone(phase), phase.to_string()))
}

/// How long a fetched object is trusted before it is fetched again.
const STALE_AFTER: Duration = Duration::from_secs(5);

async fn fetch_object(client: Client, resource: &ApiResource, namespaced: bool, namespace: &str, name: &str) -> Option<serde_yaml::Value> {
    let api: Api<DynamicObject> = if namespaced { Api::namespaced_with(client, namespace, resource) } else { Api::all_with(client, resource) };
    api.get(name).await.ok().map(|object| manifest_value(&object))
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_custom_resource_reads_its_state_from_conditions() {
        let state = |yaml: &str| super::object_state(&serde_yaml::from_str(yaml).unwrap()).map(|(_, text)| text);
        assert_eq!(state("status: {conditions: [{type: Ready, status: 'True'}]}").as_deref(), Some("Ready"));
        assert_eq!(state("status: {conditions: [{type: Ready, status: 'False', reason: ProgressingWithRetry}]}").as_deref(), Some("Reconciling"));
        assert_eq!(state("status: {conditions: [{type: Ready, status: 'False', reason: BuildFailed}]}").as_deref(), Some("Not ready"));
        assert_eq!(state("status: {conditions: [{type: Ready, status: 'True'}, {type: Reconciling, status: 'True'}]}").as_deref(), Some("Reconciling"));
        assert_eq!(state("status: {phase: Running}").as_deref(), Some("Running"));
        assert_eq!(state("spec: {replicas: 2}"), None);
    }

    #[test]
    fn a_ready_column_reads_as_words() {
        assert_eq!(super::status_note("True", true), (Tone::Good, "Ready".to_string()));
        assert_eq!(super::status_note("False", true), (Tone::Bad, "Not ready".to_string()));
        assert_eq!(super::status_note("Healthy", false).1, "Healthy");
    }

    use super::*;

    fn table() -> Value {
        serde_json::json!({
            "columnDefinitions": [
                {"name": "Name", "type": "string", "priority": 0},
                {"name": "PriorityLevel", "type": "string", "priority": 0},
                {"name": "MatchingPrecedence", "type": "integer", "priority": 0},
                {"name": "Age", "type": "string", "priority": 0},
                {"name": "Selector", "type": "string", "priority": 1}
            ],
            "rows": [
                {"cells": ["catch-all", "catch-all", 10000, "4d", null],
                 "object": {"metadata": {"name": "catch-all", "uid": "u1", "creationTimestamp": "2026-09-01T10:00:00Z", "labels": {"a": "b"}}}},
                {"cells": ["exempt", "exempt", 1, "4d", "x"],
                 "object": {"metadata": {"name": "exempt", "namespace": "ns", "uid": "u2"}}}
            ]
        })
    }

    /// No refresh task and no cluster, just the row and column shaping. The runtime
    /// is returned so the client stays valid.
    fn kind_with(data: TableData) -> (TableKind, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _entered = runtime.enter();
        let handle = runtime.spawn(async {});
        let client = Client::try_from(kube::Config::new("http://localhost:1".parse().unwrap())).unwrap();
        let kind = TableKind { data: Arc::new(Mutex::new(data)), wide: AtomicBool::new(false), handle, client, resource: ApiResource::erase::<k8s_openapi::api::core::v1::Pod>(&()), namespaced: true, objects: Arc::default(), fetching: Arc::default(), scope: Arc::default(), changed: Arc::default() };
        (kind, runtime)
    }

    #[test]
    fn the_servers_table_becomes_columns_and_rows() {
        let (columns, rows) = parse_table(&table()).unwrap();
        assert_eq!(columns.iter().map(|c| c.name).collect::<Vec<_>>(), ["NAME", "PRIORITYLEVEL", "MATCHINGPRECEDENCE", "AGE", "SELECTOR"]);
        assert!(columns[2].numeric && !columns[1].numeric);
        assert_eq!((rows[0].name.as_str(), rows[0].namespace.as_str()), ("catch-all", "-"));
        assert_eq!(rows[1].namespace, "ns");
        assert_eq!(rows[0].cells[2], "10000");
        assert_eq!(rows[0].cells[4], "", "a null cell is blank");
        assert_eq!(rows[0].labels, "a=b");
    }

    #[test]
    fn name_and_age_are_left_to_the_list_and_wide_columns_wait_for_wide() {
        let (columns, rows) = parse_table(&table()).unwrap();
        let (kind, _runtime) = kind_with(TableData { columns, rows, error: None, loading: false, scope: None, complete: true });
        assert_eq!(kind.headers(), ["PRIORITYLEVEL", "MATCHINGPRECEDENCE"]);
        kind.set_wide(true);
        assert_eq!(kind.headers(), ["PRIORITYLEVEL", "MATCHINGPRECEDENCE", "SELECTOR"]);
        let rows = kind.rows();
        assert_eq!(rows[0].extras.len(), 3);
        assert_eq!(rows[0].extras[1].sort, Some(10000), "numbers sort as numbers");
    }

    #[test]
    fn a_resource_that_cannot_be_listed_says_why() {
        let (kind, _runtime) = kind_with(TableData { columns: Vec::new(), rows: Vec::new(), error: Some("forbidden".into()), loading: false, scope: None, complete: true });
        let rows = kind.rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].name.contains("forbidden"));
    }

    #[test]
    fn status_words_are_coloured() {
        assert_eq!(cell_tone("Ready"), Tone::Good);
        assert_eq!(cell_tone("Pending"), Tone::Warn);
        assert_eq!(cell_tone("Failed"), Tone::Bad);
        assert_eq!(cell_tone("whatever"), Tone::Plain);
    }
}

#[cfg(test)]
mod live_table {
    use super::*;

    /// Against the current context: `cargo test live_table_follows -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_table_follows_changes() {
        let client = Client::try_default().await.unwrap();
        let api = ApiInfo { group: "", version: "v1".into(), kind: "ConfigMap", plural: "configmaps", namespaced: true, verbs: vec!["list".into()] };
        let kind = TableKind::start(client, &api);
        tokio::time::sleep(Duration::from_secs(2)).await;
        let before = kind.count();
        let name = format!("knav-watch-{}", std::process::id());
        std::process::Command::new("kubectl").args(["create", "configmap", &name, "-n", "default"]).output().unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let during = kind.count();
        std::process::Command::new("kubectl").args(["delete", "configmap", &name, "-n", "default"]).output().unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        println!("rows before {before}, with the new one {during}, after delete {}", kind.count());
        assert_eq!(during, before + 1);
        assert_eq!(kind.count(), before);
    }
}
