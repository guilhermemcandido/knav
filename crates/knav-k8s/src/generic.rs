
use serde::Serialize;

use super::*;

/// Any object as a value tree for the detail view, without `managedFields`.
pub fn manifest_value<T: Serialize>(item: &T) -> serde_yaml::Value {
    let mut value = serde_yaml::to_value(item).unwrap_or(serde_yaml::Value::Null);
    if let Some(metadata) = value.get_mut("metadata").and_then(|m| m.as_mapping_mut()) {
        metadata.remove("managedFields");
    }
    value
}

/// A row for kinds without a specialized table. Cluster-scoped kinds show `-`
/// as namespace.
#[derive(Clone)]
pub struct GenericRow {
    pub namespace: String,
    pub name: String,
    pub age: String,
    pub age_secs: i64,
    /// Kind-specific columns, between NAME and AGE.
    pub extras: Vec<crate::describe::Col>,
    pub status: crate::describe::Note,
    pub uid: String,
    /// UIDs of this object's owners, to find a Deployment's ReplicaSets and so on.
    pub owners: Vec<String>,
    pub labels: String,
}

/// Labels as `k=v,k=v` in key order, `-` when there are none.
pub fn label_text(labels: Option<&std::collections::BTreeMap<String, String>>) -> String {
    match labels.filter(|l| !l.is_empty()) {
        Some(l) => l.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","),
        None => "-".into(),
    }
}

pub fn generic_row<K: kube::Resource + crate::describe::Extras>(item: &K) -> GenericRow {
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

impl AgeRow for GenericRow {
    fn age_secs(&self) -> i64 {
        self.age_secs
    }
    fn set_age(&mut self, age: String, secs: i64) {
        (self.age, self.age_secs) = (age, secs);
    }
}

pub use super::watch::sorted as snapshot_generic;
