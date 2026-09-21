//! Every resource the API server offers, browsable like k9s: discovery lists them
//! and each is shown through the server's Table view, the columns `kubectl get`
//! prints, printer columns of custom resources included.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use kube::{
    Client,
    api::{Api, ApiResource, DynamicObject},
    discovery::{Discovery, Scope, verbs},
};
use serde_json::Value;
use tokio::task::JoinHandle;

use super::*;
use crate::k8s::describe::{Col, Tone};

/// One listable resource type, from discovery.
#[derive(Clone, Debug)]
pub struct ApiInfo {
    pub group: &'static str,
    pub version: String,
    pub kind: &'static str,
    pub plural: &'static str,
    pub namespaced: bool,
    pub verbs: Vec<String>,
}

impl ApiInfo {
    fn resource(&self) -> ApiResource {
        ApiResource {
            group: self.group.to_string(),
            version: self.version.clone(),
            api_version: if self.group.is_empty() { self.version.clone() } else { format!("{}/{}", self.group, self.version) },
            kind: self.kind.to_string(),
            plural: self.plural.to_string(),
        }
    }

    /// `pods`, or `deployments.apps` for a grouped one.
    pub fn qualified_name(&self) -> String {
        if self.group.is_empty() { self.plural.to_string() } else { format!("{}.{}", self.plural, self.group) }
    }
}

impl From<&CrdInfo> for ApiInfo {
    fn from(crd: &CrdInfo) -> Self {
        ApiInfo { group: crd.group, version: crd.version.clone(), kind: crd.kind, plural: leak(&crd.plural), namespaced: crd.namespaced, verbs: Vec::new() }
    }
}

/// Strings shown as column headers or kind labels are `&'static str` all over
/// the UI, so each distinct one is leaked once (a bounded set: the cluster's
/// resource names and column titles).
fn leak(text: &str) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut map = INTERNED.get_or_init(Default::default).lock().expect("interner lock");
    if let Some(found) = map.get(text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
    map.insert(text.to_string(), leaked);
    leaked
}

/// Everything the API server lets us list, one entry per resource (its
/// recommended version), sorted by group then kind. Empty if discovery fails.
pub async fn discover_apis(client: &Client) -> Vec<ApiInfo> {
    let Ok(discovery) = Discovery::new(client.clone()).run().await else { return Vec::new() };
    let mut apis: Vec<ApiInfo> = discovery
        .groups()
        .flat_map(|group| group.recommended_resources())
        .filter(|(resource, caps)| !resource.plural.contains('/') && caps.supports_operation(verbs::LIST))
        .map(|(resource, caps)| ApiInfo {
            group: leak(&resource.group),
            version: resource.version.clone(),
            kind: leak(&resource.kind),
            plural: leak(&resource.plural),
            namespaced: caps.scope == Scope::Namespaced,
            verbs: caps.operations.clone(),
        })
        .collect();
    apis.sort_by(|a, b| (a.group, a.kind).cmp(&(b.group, b.kind)));
    apis.dedup_by(|a, b| a.group == b.group && a.plural == b.plural);
    apis
}

/// The list of every resource type (`:api`); Enter on a row opens it.
pub struct ApiList {
    pub apis: Vec<ApiInfo>,
    /// How many objects each type has, filled in by the background counter.
    pub counts: InstanceCounts,
}

impl CatalogKind for ApiList {
    fn count(&self) -> usize {
        self.apis.len()
    }

    fn rows(&self) -> Vec<GenericRow> {
        self.apis
            .iter()
            .map(|api| {
                let col = |header, text: String| Col { header, text, tone: Tone::Plain, sort: None };
                let count = self.counts.get(api.group, api.plural);
                GenericRow {
                    namespace: "-".into(),
                    name: api.plural.to_string(),
                    age: "-".into(),
                    age_secs: 0,
                    extras: vec![
                        col("GROUP", if api.group.is_empty() { "core".into() } else { api.group.to_string() }),
                        col("VERSION", api.version.clone()),
                        col("KIND", api.kind.to_string()),
                        Col { header: "COUNT", text: count.text(), tone: if count == Count::Known(0) { Tone::Muted } else { Tone::Plain }, sort: Some(count.sort_key()) },
                        col("NAMESPACED", api.namespaced.to_string()),
                        col("VERBS", api.verbs.join(",")),
                    ],
                    status: None,
                    uid: api.qualified_name(),
                    owners: Vec::new(),
                    labels: String::new(),
                }
            })
            .collect()
    }

    /// The resource described as an object, so info and YAML can show it.
    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value> {
        let api = self.apis.get(index)?;
        Some(serde_yaml::to_value(serde_json::json!({
            "apiVersion": "meta.k8s.io/v1",
            "kind": "APIResource",
            "metadata": {"name": api.plural},
            "spec": {"group": api.group, "version": api.version, "kind": api.kind, "plural": api.plural, "namespaced": api.namespaced, "verbs": api.verbs},
        })).ok()?)
    }

    fn headers(&self) -> Vec<&'static str> {
        vec!["GROUP", "VERSION", "KIND", "COUNT", "NAMESPACED", "VERBS"]
    }
}

#[derive(Clone)]
struct TableColumn {
    name: &'static str,
    /// 0 is shown always; higher is wide-only (`kubectl get -o wide`).
    priority: i64,
    numeric: bool,
}

struct TableRow {
    cells: Vec<String>,
    namespace: String,
    name: String,
    uid: String,
    owners: Vec<String>,
    created: Option<k8s_openapi::jiff::Timestamp>,
    labels: String,
}

#[derive(Default)]
struct TableData {
    columns: Vec<TableColumn>,
    rows: Vec<TableRow>,
    error: Option<String>,
    /// More pages of the first load are still coming.
    loading: bool,
    /// The namespace the rows were fetched for (`None`: all of them), and whether that is all of them.
    scope: Option<String>,
    complete: bool,
}

impl TableData {
    /// Whether the rows held include everything a fetch of `namespace` would return.
    fn covers(&self, namespace: Option<&str>) -> bool {
        self.complete && (self.scope.is_none() || self.scope.as_deref() == namespace)
    }
}

/// One resource type shown through the server's Table view, refreshed every
/// couple of seconds in the background for as long as it is open. With a namespace
/// selected only that namespace is fetched.
pub struct TableKind {
    data: Arc<Mutex<TableData>>,
    wide: AtomicBool,
    handle: JoinHandle<()>,
    client: Client,
    resource: ApiResource,
    namespaced: bool,
    /// Full objects fetched for the info view and actions, so asking again costs nothing.
    objects: Arc<Mutex<HashMap<(String, String), (std::time::Instant, serde_yaml::Value)>>>,
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
                    tokio::select! {
                        _ = refresh_table(&client, &resource, &data, wanted.as_deref()) => {}
                        _ = changed.notified() => continue,
                    }
                    // A big list takes long to fetch, so wait in proportion and never hammer the server.
                    let pause = Duration::from_secs(crate::config::tunables::tunables().api_refresh_seconds.max(1)).max(started.elapsed() * 3);
                    tokio::select! {
                        _ = tokio::time::sleep(pause) => {}
                        _ = changed.notified() => {}
                    }
                }
            })
        };
        TableKind { data, wide: AtomicBool::new(false), handle, client, resource, namespaced: api.namespaced, objects: Arc::default(), scope, changed }
    }

    fn shows(&self, column: &TableColumn) -> bool {
        // NAME and AGE have columns of their own in the list.
        !column.name.eq_ignore_ascii_case("name") && !column.name.eq_ignore_ascii_case("age") && (column.priority == 0 || self.wide.load(Ordering::Relaxed))
    }
}

/// Rows asked for per request; the server hands the rest over with a continue token.
const PAGE: usize = 500;

/// One page of the Table view of `resource`, and the token for the next page if there is one.
async fn fetch_page(client: &Client, resource: &ApiResource, namespace: Option<&str>, token: Option<&str>) -> anyhow::Result<(Vec<TableColumn>, Vec<TableRow>, Option<String>)> {
    let root = if resource.group.is_empty() { format!("/api/{}", resource.version) } else { format!("/apis/{}/{}", resource.group, resource.version) };
    let base = match namespace {
        Some(ns) => format!("{root}/namespaces/{}/{}", percent_encode(ns), resource.plural),
        None => format!("{root}/{}", resource.plural),
    };
    let path = match token {
        Some(token) => format!("{base}?limit={PAGE}&continue={}", percent_encode(token)),
        None => format!("{base}?limit={PAGE}"),
    };
    let request = http::Request::get(path).header(http::header::ACCEPT, "application/json;as=Table;g=meta.k8s.io;v=v1").body(Vec::new())?;
    let table: Value = serde_json::from_str(&client.request_text(request).await?)?;
    let next = table.get("metadata").and_then(|m| m.get("continue")).and_then(|c| c.as_str()).filter(|c| !c.is_empty()).map(str::to_string);
    let (columns, rows) = parse_table(&table)?;
    Ok((columns, rows, next))
}

fn percent_encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Reads every page of `namespace` (all when `None`). When what is held cannot stand in for the
/// new list, rows show up as pages arrive so a long list appears at once; otherwise the whole
/// snapshot is gathered and swapped in, so rows never jump around mid-fetch.
async fn refresh_table(client: &Client, resource: &ApiResource, data: &Mutex<TableData>, namespace: Option<&str>) {
    let in_place = data.lock().map(|d| d.columns.is_empty() || !d.covers(namespace)).unwrap_or(true);
    if in_place && let Ok(mut d) = data.lock() {
        d.rows.clear();
        d.complete = false;
        d.scope = namespace.map(str::to_string);
    }
    let (mut columns, mut rows, mut token) = (Vec::new(), Vec::new(), None::<String>);
    loop {
        match fetch_page(client, resource, namespace, token.as_deref()).await {
            Ok((page_columns, page_rows, next)) => {
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
                return;
            }
        }
    }
    if let Ok(mut d) = data.lock() {
        if in_place {
            d.complete = true;
        } else {
            *d = TableData { columns, rows, error: None, loading: false, scope: namespace.map(str::to_string), complete: true };
        }
    }
}

fn cell_text(cell: &Value) -> String {
    match cell {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn parse_table(table: &Value) -> anyhow::Result<(Vec<TableColumn>, Vec<TableRow>)> {
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

/// How a status-like cell should be coloured.
fn status_tone(text: &str) -> Tone {
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
            // Rows of another namespace are not this list: say it is loading rather than show a gap.
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

    fn rows(&self) -> Vec<GenericRow> {
        let Ok(data) = self.data.lock() else { return Vec::new() };
        let shown: Vec<usize> = (0..data.columns.len()).filter(|&i| self.shows(&data.columns[i])).collect();
        if data.rows.is_empty()
            && let Some(error) = &data.error
        {
            // A resource we can't list (forbidden, gone): say so in the list itself.
            let extras = shown.iter().map(|&i| Col { header: data.columns[i].name, text: String::new(), tone: Tone::Plain, sort: None }).collect();
            return vec![GenericRow {
                namespace: "-".into(),
                name: format!("⚠ {error}"),
                age: "-".into(),
                age_secs: 0,
                extras,
                status: Some((Tone::Bad, "cannot list".into())),
                uid: String::new(),
                owners: Vec::new(),
                labels: String::new(),
            }];
        }
        let status_column = data.columns.iter().position(|c| matches!(c.name, "STATUS" | "PHASE" | "STATE"));
        let mut rows = crate::k8s::par_map(&data.rows, |row| {
                let extras = shown
                    .iter()
                    .map(|&i| {
                        let column = &data.columns[i];
                        let text = row.cells.get(i).cloned().unwrap_or_default();
                        Col { header: column.name, tone: if Some(i) == status_column { status_tone(&text) } else { Tone::Plain }, sort: column.numeric.then(|| text.parse().ok()).flatten(), text }
                    })
                    .collect();
                let status = status_column.and_then(|i| row.cells.get(i)).map(|text| (status_tone(text), text.clone()));
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
            });
        if data.loading {
            rows.push(GenericRow { namespace: "-".into(), name: format!("… loading more ({} so far)", data.rows.len()), age: "-".into(), age_secs: i64::MAX, extras: shown.iter().map(|&i| Col { header: data.columns[i].name, text: String::new(), tone: Tone::Plain, sort: None }).collect(), status: Some((Tone::Muted, "loading".into())), uid: String::new(), owners: Vec::new(), labels: String::new() });
        }
        rows
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

/// How long a fetched object is trusted before it is fetched again.
const STALE_AFTER: Duration = Duration::from_secs(5);

async fn fetch_object(client: Client, resource: &ApiResource, namespaced: bool, namespace: &str, name: &str) -> Option<serde_yaml::Value> {
    let api: Api<DynamicObject> = if namespaced { Api::namespaced_with(client, namespace, resource) } else { Api::all_with(client, resource) };
    api.get(name).await.ok().map(|object| manifest_value(&object))
}


#[cfg(test)]
mod tests {
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

    /// No refresh task and no cluster: only the row/column shaping is under
    /// test. The runtime is returned so the client it needs stays valid.
    fn kind_with(data: TableData) -> (TableKind, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _entered = runtime.enter();
        let handle = runtime.spawn(async {});
        let client = Client::try_from(kube::Config::new("http://localhost:1".parse().unwrap())).unwrap();
        let kind = TableKind { data: Arc::new(Mutex::new(data)), wide: AtomicBool::new(false), handle, client, resource: ApiResource::erase::<k8s_openapi::api::core::v1::Pod>(&()), namespaced: true, objects: Arc::default(), scope: Arc::default(), changed: Arc::default() };
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
        assert_eq!(status_tone("Ready"), Tone::Good);
        assert_eq!(status_tone("Pending"), Tone::Warn);
        assert_eq!(status_tone("Failed"), Tone::Bad);
        assert_eq!(status_tone("whatever"), Tone::Plain);
    }

    #[test]
    fn the_list_of_apis_names_each_by_group() {
        let list = ApiList { apis: vec![ApiInfo { group: "apps", version: "v1".into(), kind: "Deployment", plural: "deployments", namespaced: true, verbs: vec!["list".into()] }] };
        let rows = list.rows();
        assert_eq!(rows[0].uid, "deployments.apps");
        assert_eq!(rows[0].extras[0].text, "apps");
        assert_eq!(rows[0].extras.len(), list.headers().len());
    }
}
