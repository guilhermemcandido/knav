//! The objects the relations diagram and workload logs need around one object, fetched
//! on demand: its namespace only, in parallel, and names only for ConfigMaps and Secrets.

use std::fmt::Debug;
use std::time::Duration;

use futures::future::{BoxFuture, FutureExt, join_all};
use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::{
    apps::v1::{DaemonSet, ReplicaSet, StatefulSet},
    autoscaling::v2::HorizontalPodAutoscaler,
    batch::v1::{CronJob, Job},
    core::v1::{ConfigMap, Node, PersistentVolume, PersistentVolumeClaim, Secret, Service, ServiceAccount},
    networking::v1::Ingress,
    storage::v1::StorageClass,
};
use kube::{Client, Resource, api::{Api, ListParams}, core::PartialObjectMeta};
use serde::{Serialize, de::DeserializeOwned};
use serde_yaml::Value;

/// A kind that took longer than this is left out rather than holding up the rest.
const TIMEOUT: Duration = Duration::from_secs(10);

/// A kind that can't be listed (no access, too slow) is left out.
pub struct Fetched {
    pub manifests: Vec<Value>,
}

type Fetch = BoxFuture<'static, Result<Vec<Value>, ()>>;

/// Everything `list` returns, page by page.
async fn pages<K: Clone, T: Clone>(api: Api<K>, list: impl Fn(Api<K>, ListParams) -> BoxFuture<'static, kube::Result<kube::core::ObjectList<T>>>, to_value: fn(&T) -> Value) -> Result<Vec<Value>, ()> {
    let mut out = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let mut params = ListParams::default().limit(500);
        params.continue_token = token.take();
        let page = list(api.clone(), params).await.map_err(|_| ())?;
        out.extend(page.items.iter().map(to_value));
        match page.metadata.continue_.filter(|t| !t.is_empty()) {
            Some(next) => token = Some(next),
            None => return Ok(out),
        }
    }
}

fn api<K: Resource<DynamicType = (), Scope = NamespaceResourceScope>>(client: &Client, namespace: Option<&str>) -> Api<K> {
    match namespace {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    }
}

fn timed(work: impl std::future::Future<Output = Result<Vec<Value>, ()>> + Send + 'static) -> Fetch {
    async move { tokio::time::timeout(TIMEOUT, work).await.unwrap_or(Err(())) }.boxed()
}

/// Whole objects of a namespaced kind.
fn full<K>(client: &Client, namespace: Option<&str>) -> Fetch
where
    K: Resource<DynamicType = (), Scope = NamespaceResourceScope> + Clone + DeserializeOwned + Serialize + Debug + Send + Sync + 'static,
{
    let api = api::<K>(client, namespace);
    timed(pages(api, |api, params| async move { api.list(&params).await }.boxed(), |item| crate::manifest_value(item)))
}

/// Only the metadata of a namespaced kind: enough to name it and follow its owners.
/// Some API servers and proxies refuse metadata-only lists, so a full list is the
/// fallback, with ConfigMap and Secret payloads dropped.
fn names<K>(client: &Client, namespace: Option<&str>) -> Fetch
where
    K: Resource<DynamicType = (), Scope = NamespaceResourceScope> + Clone + DeserializeOwned + Serialize + Debug + Send + Sync + 'static,
{
    let api = api::<K>(client, namespace);
    timed(async move {
        match pages(api.clone(), |api, params| async move { api.list_metadata(&params).await }.boxed(), metadata_value::<K>).await {
            Ok(values) => Ok(values),
            Err(()) => pages(api, |api, params| async move { api.list(&params).await }.boxed(), |item| crate::relations::slim(crate::manifest_value(item))).await,
        }
    })
}

fn metadata_value<K: Resource<DynamicType = ()>>(item: &PartialObjectMeta<K>) -> Value {
    let mut value = serde_yaml::Mapping::new();
    value.insert("apiVersion".into(), K::api_version(&()).to_string().into());
    value.insert("kind".into(), K::kind(&()).to_string().into());
    let mut metadata = serde_yaml::to_value(&item.metadata).unwrap_or(Value::Null);
    if let Some(map) = metadata.as_mapping_mut() {
        map.remove("managedFields");
    }
    value.insert("metadata".into(), metadata);
    Value::Mapping(value)
}

/// One cluster-scoped object by name, when it exists.
fn one<K>(client: &Client, name: String) -> Fetch
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + Serialize + Debug + Send + Sync + 'static,
{
    let api: Api<K> = Api::all(client.clone());
    timed(async move { api.get_opt(&name).await.map(|found| found.iter().map(crate::manifest_value).collect()).map_err(|_| ()) })
}

/// One object of the type `api` describes, by name; for a box `R` didn't load.
pub async fn get_object(client: &Client, api: &crate::ApiInfo, namespace: Option<&str>, name: &str) -> Option<Value> {
    use kube::api::DynamicObject;
    let resource = api.resource();
    let objects: Api<DynamicObject> = match namespace.filter(|_| api.namespaced) {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &resource),
        None => Api::all_with(client.clone(), &resource),
    };
    tokio::time::timeout(TIMEOUT, objects.get_opt(name)).await.ok()?.ok().flatten().map(|o| crate::manifest_value(&o))
}

/// What surrounds `target`, besides Pods and Deployments, which are always watched.
pub async fn surroundings(client: &Client, target: &Value) -> Fetched {
    let kind = target.get("kind").and_then(Value::as_str).unwrap_or("");
    let namespace = target.get("metadata").and_then(|m| m.get("namespace")).and_then(Value::as_str);
    let mut fetches: Vec<Fetch> = Vec::new();
    match namespace {
        Some(ns) => {
            let ns = Some(ns);
            fetches.extend([
                full::<ReplicaSet>(client, ns),
                full::<StatefulSet>(client, ns),
                full::<DaemonSet>(client, ns),
                full::<Job>(client, ns),
                full::<CronJob>(client, ns),
                full::<HorizontalPodAutoscaler>(client, ns),
                full::<Service>(client, ns),
                full::<Ingress>(client, ns),
                full::<PersistentVolumeClaim>(client, ns),
                full::<ServiceAccount>(client, ns),
                names::<ConfigMap>(client, ns),
                names::<Secret>(client, ns),
            ]);
            // The few cluster-scoped objects it points at: its node, volume, class.
            for (kind, name) in crate::relations::cluster_refs(target) {
                match kind.as_str() {
                    "Node" => fetches.push(one::<Node>(client, name)),
                    "PersistentVolume" => fetches.push(one::<PersistentVolume>(client, name)),
                    "StorageClass" => fetches.push(one::<StorageClass>(client, name)),
                    _ => {}
                }
            }
        }
        // A Node, volume or class is used from any namespace: owners by name only, so
        // pods fold up to their workloads without fetching every template.
        None => {
            fetches.extend([
                names::<ReplicaSet>(client, None),
                names::<StatefulSet>(client, None),
                names::<DaemonSet>(client, None),
                names::<Job>(client, None),
            ]);
            if kind != "Node" {
                fetches.push(full::<PersistentVolumeClaim>(client, None));
                fetches.push(timed({
                    let api: Api<PersistentVolume> = Api::all(client.clone());
                    async move { api.list(&ListParams::default()).await.map(|l| l.items.iter().map(crate::manifest_value).collect()).map_err(|_| ()) }
                }));
            }
        }
    }
    let mut fetched = Fetched { manifests: join_all(fetches).await.into_iter().flatten().flatten().collect() };
    let owners = owner_chain(client, target, &fetched.manifests).await;
    fetched.manifests.extend(owners);
    fetched
}

/// The owners above `target` that `have` lacks, of any kind, custom resources
/// included, fetched one by one up the chain.
async fn owner_chain(client: &Client, target: &Value, have: &[Value]) -> Vec<Value> {
    let uid_of = |v: &Value| v.get("metadata").and_then(|m| m.get("uid")).and_then(Value::as_str).map(String::from);
    let known: std::collections::HashMap<String, &Value> = have.iter().filter_map(|v| Some((uid_of(v)?, v))).collect();
    let namespace = target.get("metadata").and_then(|m| m.get("namespace")).and_then(Value::as_str).map(String::from);
    let mut found = Vec::new();
    let mut current = target.clone();
    for _ in 0..8 {
        let refs = current.get("metadata").and_then(|m| m.get("ownerReferences")).and_then(Value::as_sequence).cloned().unwrap_or_default();
        // The controller when there is one, as the diagram follows.
        let Some(owner) = refs.iter().find(|r| r.get("controller").and_then(Value::as_bool) == Some(true)).or(refs.first()) else { break };
        let field = |key: &str| owner.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        if let Some(existing) = known.get(&field("uid")) {
            current = (*existing).clone();
            continue;
        }
        let Some(next) = get_owner(client, &field("apiVersion"), &field("kind"), namespace.as_deref(), &field("name")).await else { break };
        found.push(next.clone());
        current = next;
    }
    found
}

/// One object by apiVersion, kind and name, through discovery, so it works for CRDs.
async fn get_owner(client: &Client, api_version: &str, kind: &str, namespace: Option<&str>, name: &str) -> Option<Value> {
    use kube::api::DynamicObject;
    use kube::discovery::{Scope, pinned_kind};
    let (group, version) = api_version.split_once('/').unwrap_or(("", api_version));
    let gvk = kube::core::GroupVersionKind::gvk(group, version, kind);
    let work = async {
        let (resource, caps) = pinned_kind(client, &gvk).await.ok()?;
        let api: Api<DynamicObject> = match (caps.scope, namespace) {
            (Scope::Namespaced, Some(ns)) => Api::namespaced_with(client.clone(), ns, &resource),
            _ => Api::all_with(client.clone(), &resource),
        };
        api.get_opt(name).await.ok().flatten().map(|object| crate::manifest_value(&object))
    };
    tokio::time::timeout(TIMEOUT, work).await.ok().flatten()
}
