//! Every resource the API server offers, browsable like k9s does: discovery
//! lists them, and each is shown through the server's own Table view
//! (`Accept: ...;as=Table`) — the same columns `kubectl get` prints, custom
//! resources' printer columns included.

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
use crate::describe::{Col, Tone};

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
                GenericRow {
                    namespace: "-".into(),
                    name: api.plural.to_string(),
                    age: "-".into(),
                    age_secs: 0,
                    extras: vec![
                        col("GROUP", if api.group.is_empty() { "core".into() } else { api.group.to_string() }),
                        col("VERSION", api.version.clone()),
                        col("KIND", api.kind.to_string()),
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

    fn spec_at(&self, _index: usize) -> Option<serde_yaml::Value> {
        None
    }

    fn headers(&self) -> Vec<&'static str> {
        vec!["GROUP", "VERSION", "KIND", "NAMESPACED", "VERBS"]
    }
}

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
}

/// One resource type shown through the server's Table view, refreshed every
/// couple of seconds in the background for as long as it is open.
pub struct TableKind {
    data: Arc<Mutex<TableData>>,
    wide: AtomicBool,
    handle: JoinHandle<()>,
    client: Client,
    resource: ApiResource,
    namespaced: bool,
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
        let handle = {
            let (client, resource, data) = (client.clone(), resource.clone(), Arc::clone(&data));
            tokio::spawn(async move {
                loop {
                    let result = fetch_table(&client, &resource).await;
                    if let Ok(mut data) = data.lock() {
                        match result {
                            Ok((columns, rows)) => *data = TableData { columns, rows, error: None },
                            Err(e) => data.error = Some(format!("{e:#}")),
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(crate::tunables::tunables().api_refresh_seconds.max(1))).await;
                }
            })
        };
        TableKind { data, wide: AtomicBool::new(false), handle, client, resource, namespaced: api.namespaced }
    }

    fn shows(&self, column: &TableColumn) -> bool {
        // NAME and AGE have columns of their own in the list.
        !column.name.eq_ignore_ascii_case("name") && !column.name.eq_ignore_ascii_case("age") && (column.priority == 0 || self.wide.load(Ordering::Relaxed))
    }
}

async fn fetch_table(client: &Client, resource: &ApiResource) -> anyhow::Result<(Vec<TableColumn>, Vec<TableRow>)> {
    let path = if resource.group.is_empty() { format!("/api/{}/{}", resource.version, resource.plural) } else { format!("/apis/{}/{}/{}", resource.group, resource.version, resource.plural) };
    let request = http::Request::get(path).header(http::header::ACCEPT, "application/json;as=Table;g=meta.k8s.io;v=v1").body(Vec::new())?;
    let text = client.request_text(request).await?;
    parse_table(&serde_json::from_str(&text)?)
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
        data.rows
            .iter()
            .map(|row| {
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
            })
            .collect()
    }

    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value> {
        let (namespace, name) = {
            let data = self.data.lock().ok()?;
            let row = data.rows.get(index)?;
            (row.namespace.clone(), row.name.clone())
        };
        let (client, resource) = (self.client.clone(), self.resource.clone());
        let namespaced = self.namespaced && namespace != "-";
        let object = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let api: Api<DynamicObject> = if namespaced { Api::namespaced_with(client, &namespace, &resource) } else { Api::all_with(client, &resource) };
                api.get(&name).await.ok()
            })
        })?;
        Some(manifest_value(&object))
    }
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
        let kind = TableKind { data: Arc::new(Mutex::new(data)), wide: AtomicBool::new(false), handle, client, resource: ApiResource::erase::<k8s_openapi::api::core::v1::Pod>(&()), namespaced: true };
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
        let (kind, _runtime) = kind_with(TableData { columns, rows, error: None });
        assert_eq!(kind.headers(), ["PRIORITYLEVEL", "MATCHINGPRECEDENCE"]);
        kind.set_wide(true);
        assert_eq!(kind.headers(), ["PRIORITYLEVEL", "MATCHINGPRECEDENCE", "SELECTOR"]);
        let rows = kind.rows();
        assert_eq!(rows[0].extras.len(), 3);
        assert_eq!(rows[0].extras[1].sort, Some(10000), "numbers sort as numbers");
    }

    #[test]
    fn a_resource_that_cannot_be_listed_says_why() {
        let (kind, _runtime) = kind_with(TableData { columns: Vec::new(), rows: Vec::new(), error: Some("forbidden".into()) });
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
