use std::sync::Arc;

use futures::StreamExt;
use kube::{
    Client, Resource,
    api::Api,
    runtime::{WatchStreamExt, reflector, watcher},
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::task::JoinHandle;

use super::*;

/// Any object as a generic value tree for the detail view, without `managedFields`
/// (kubectl-apply bookkeeping, unreadable).
pub fn manifest_value<T: Serialize>(item: &T) -> serde_yaml::Value {
    let mut value = serde_yaml::to_value(item).unwrap_or(serde_yaml::Value::Null);
    if let Some(metadata) = value.get_mut("metadata").and_then(|m| m.as_mapping_mut()) {
        metadata.remove("managedFields");
    }
    value
}

/// A row for kinds without a specialized table: enough to list and identify an
/// object. Cluster-scoped kinds show `-` as namespace.
#[derive(Clone)]
pub struct GenericRow {
    pub namespace: String,
    pub name: String,
    pub age: String,
    pub age_secs: i64,
    /// Kind-specific columns (see `describe`), between NAME and AGE.
    pub extras: Vec<crate::k8s::describe::Col>,
    /// A short coloured status for the bottom bar.
    pub status: crate::k8s::describe::Note,
    pub uid: String,
    /// UIDs of this object's owners (`ownerReferences`), what lets a
    /// Deployment's ReplicaSets, or a ReplicaSet's Pods, be found.
    pub owners: Vec<String>,
    /// `k=v,k=v`, for the wide view.
    pub labels: String,
}

/// Works for typed structs and `DynamicObject` alike, since `Resource::meta()` only
/// reads `self`. `k=v,k=v` in key order, `-` when there are none.
pub fn label_text(labels: Option<&std::collections::BTreeMap<String, String>>) -> String {
    match labels.filter(|l| !l.is_empty()) {
        Some(l) => l.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","),
        None => "-".into(),
    }
}

pub fn generic_row<K: kube::Resource + crate::k8s::describe::Extras>(item: &K) -> GenericRow {
    let meta = item.meta();
    let namespace = meta.namespace.clone().unwrap_or_else(|| "-".into());
    let name = meta.name.clone().unwrap_or_default();
    let age = meta.creation_timestamp.as_ref().map(|t| humanize_age(t.0)).unwrap_or_else(|| "-".into());
    let uid = meta.uid.clone().unwrap_or_default();
    let owners = meta.owner_references.iter().flatten().map(|o| o.uid.clone()).collect();
    let age_secs = age_seconds(meta.creation_timestamp.as_ref());
    let (extras, status) = item.extras();
    let labels = label_text(meta.labels.as_ref());
    GenericRow { namespace, name, age, age_secs, extras, status, uid, owners, labels }
}

/// Same live-watch pattern as `watch_pods`/`watch_deployments`, generic
/// over any typed k8s-openapi resource, used for every catalog kind that
/// doesn't need specialized fields.
pub fn watch_generic<K>(client: Client) -> (reflector::Store<K>, JoinHandle<()>)
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
{
    let api: Api<K> = Api::all(client);
    let (reader, writer) = reflector::store();
    let stream = watcher(api, watcher::Config::default()).default_backoff().reflect(writer).applied_objects();
    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {}
    });
    (reader, handle)
}

/// Sorted snapshot, same reasoning as `snapshot`/`snapshot_deployments`,
/// generic over anything `reflector::store` can hold (typed resources and
/// `DynamicObject` alike).
pub fn snapshot_generic<K>(store: &reflector::Store<K>) -> Vec<Arc<K>>
where
    K: Resource + Clone,
    K::DynamicType: Eq + std::hash::Hash + Clone,
{
    let mut items = store.state();
    items.sort_by(|a, b| {
        let key = |x: &Arc<K>| (x.meta().namespace.clone().unwrap_or_default(), x.meta().name.clone().unwrap_or_default());
        key(a).cmp(&key(b))
    });
    items
}
