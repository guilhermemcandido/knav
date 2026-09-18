//! What a drilled-into list is narrowed to: "the ReplicaSets owned by this
//! Deployment", "the Pods a Service selects".

use std::collections::BTreeMap;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Scope {
    /// Objects whose `ownerReferences` include this UID.
    Owner { uid: String, kind: String, name: String },
    /// Pods whose labels contain every pair of a Service's selector.
    Selector { labels: BTreeMap<String, String>, kind: String, name: String },
}

impl Scope {
    /// `Deployment/web` — shown in the header.
    pub(crate) fn label(&self) -> String {
        match self {
            Scope::Owner { kind, name, .. } | Scope::Selector { kind, name, .. } => format!("{kind}/{name}"),
        }
    }

    pub(crate) fn matches_meta(&self, meta: &ObjectMeta) -> bool {
        match self {
            Scope::Owner { uid, .. } => meta.owner_references.iter().flatten().any(|o| &o.uid == uid),
            Scope::Selector { labels, .. } => {
                !labels.is_empty() && labels.iter().all(|(k, v)| meta.labels.as_ref().and_then(|l| l.get(k)) == Some(v))
            }
        }
    }

    pub(crate) fn matches_row(&self, row: &k8s::GenericRow) -> bool {
        match self {
            Scope::Owner { uid, .. } => row.owners.contains(uid),
            Scope::Selector { .. } => false,
        }
    }
}

/// A Service's `spec.selector`, out of its manifest.
pub(crate) fn service_selector(manifest: &serde_yaml::Value) -> BTreeMap<String, String> {
    manifest
        .get("spec")
        .and_then(|s| s.get("selector"))
        .and_then(|s| s.as_mapping())
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default()
}

/// `ReplicaSets` -> `ReplicaSet`.
pub(crate) fn singular(kind: ResourceKind) -> String {
    kind.label().trim_end_matches('s').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;

    fn meta_owned_by(uid: &str) -> ObjectMeta {
        ObjectMeta { owner_references: Some(vec![OwnerReference { uid: uid.into(), ..Default::default() }]), ..Default::default() }
    }

    #[test]
    fn owner_scope_matches_only_that_owners_children() {
        let scope = Scope::Owner { uid: "abc".into(), kind: "Deployment".into(), name: "web".into() };
        assert!(scope.matches_meta(&meta_owned_by("abc")));
        assert!(!scope.matches_meta(&meta_owned_by("other")));
        assert!(!scope.matches_meta(&ObjectMeta::default()));
        assert_eq!(scope.label(), "Deployment/web");
    }

    #[test]
    fn selector_scope_needs_every_label_and_never_matches_everything() {
        let labels: BTreeMap<String, String> = [("app".to_string(), "web".to_string())].into();
        let scope = Scope::Selector { labels, kind: "Service".into(), name: "web".into() };
        let pod = ObjectMeta { labels: Some([("app".to_string(), "web".to_string()), ("x".to_string(), "y".to_string())].into()), ..Default::default() };
        assert!(scope.matches_meta(&pod));
        assert!(!scope.matches_meta(&ObjectMeta::default()));
        let empty = Scope::Selector { labels: BTreeMap::new(), kind: "Service".into(), name: "headless".into() };
        assert!(!empty.matches_meta(&pod));
    }

    #[test]
    fn service_selector_reads_spec_selector() {
        let manifest: serde_yaml::Value = serde_yaml::from_str("spec:\n  selector:\n    app: web\n    tier: fe\n").unwrap();
        let sel = service_selector(&manifest);
        assert_eq!(sel.get("app").map(String::as_str), Some("web"));
        assert_eq!(sel.len(), 2);
    }

    #[test]
    fn singular_drops_the_plural_s() {
        assert_eq!(singular(ResourceKind::ReplicaSets), "ReplicaSet");
        assert_eq!(singular(ResourceKind::CronJobs), "CronJob");
    }
}
