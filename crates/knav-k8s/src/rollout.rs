//! Rollout history and rolling back, the way `kubectl rollout undo` does: a
//! Deployment's from the ReplicaSets it owns, a StatefulSet's or DaemonSet's from
//! its ControllerRevisions.

use anyhow::{Context as _, Result, bail};
use k8s_openapi::api::{
    apps::v1::{ControllerRevision, DaemonSet, Deployment, ReplicaSet, StatefulSet},
    core::v1::PodTemplateSpec,
};
use kube::{
    Api, Client,
    api::{ListParams, Patch, PatchParams, PostParams},
};

const REVISION: &str = "deployment.kubernetes.io/revision";
const CHANGE_CAUSE: &str = "kubernetes.io/change-cause";
/// The label each ReplicaSet adds to its template; not part of what was deployed.
const HASH_LABEL: &str = "pod-template-hash";

pub struct Revision {
    pub number: i64,
    pub age: String,
    pub images: String,
    /// Why it was made, when someone said (`kubernetes.io/change-cause`).
    pub cause: String,
    /// How many pods it runs, known for a Deployment's revisions only.
    pub pods: Option<i32>,
    /// What the Deployment runs now.
    pub current: bool,
    /// The pod template as YAML, for comparing revisions.
    pub template: String,
}

/// Whether `kind` keeps a history to show.
pub fn has_history(kind: &str) -> bool {
    matches!(kind, "Deployment" | "StatefulSet" | "DaemonSet")
}

/// Every revision the object still has, newest first.
pub async fn history(client: &Client, kind: &str, namespace: &str, name: &str) -> Result<Vec<Revision>> {
    match kind {
        "Deployment" => deployment_history(client, namespace, name).await,
        "StatefulSet" | "DaemonSet" => controller_history(client, kind, namespace, name).await,
        _ => bail!("{kind} has no rollout history"),
    }
}

/// Rolls the object back to revision `number`.
pub async fn rollback(client: &Client, kind: &str, namespace: &str, name: &str, number: i64) -> Result<String> {
    match kind {
        "Deployment" => deployment_rollback(client, namespace, name, number).await,
        "StatefulSet" | "DaemonSet" => controller_rollback(client, kind, namespace, name, number).await,
        _ => bail!("{kind} has no rollout history"),
    }
}

async fn deployment_history(client: &Client, namespace: &str, name: &str) -> Result<Vec<Revision>> {
    let deployment = Api::<Deployment>::namespaced(client.clone(), namespace).get(name).await?;
    let owned = owned_replica_sets(client, namespace, &deployment).await?;
    Ok(revisions(&deployment, &owned))
}

/// Puts revision `number`'s pod template back on the Deployment, which rolls it out.
async fn deployment_rollback(client: &Client, namespace: &str, name: &str, number: i64) -> Result<String> {
    let api = Api::<Deployment>::namespaced(client.clone(), namespace);
    let mut deployment = api.get(name).await?;
    if revision_of(&deployment.metadata) == Some(number) {
        bail!("deployment/{name} already runs revision {number}");
    }
    let owned = owned_replica_sets(client, namespace, &deployment).await?;
    let source = owned.iter().find(|rs| revision_of(&rs.metadata) == Some(number)).with_context(|| format!("revision {number} is gone"))?;
    let template = deployed_template(source).context("that revision has no pod template")?;
    deployment.spec.as_mut().context("the Deployment has no spec")?.template = template;
    // Like `kubectl rollout undo`: the new revision says why the old one was made.
    let cause = source.metadata.annotations.as_ref().and_then(|a| a.get(CHANGE_CAUSE)).cloned();
    let annotations = deployment.metadata.annotations.get_or_insert_with(Default::default);
    match cause {
        Some(cause) => annotations.insert(CHANGE_CAUSE.into(), cause),
        None => annotations.remove(CHANGE_CAUSE),
    };
    // The fetched resourceVersion makes this fail rather than overwrite a newer change.
    api.replace(name, &PostParams::default(), &deployment).await?;
    Ok(format!("deployment/{name} rolled back to revision {number}"))
}

async fn owned_replica_sets(client: &Client, namespace: &str, deployment: &Deployment) -> Result<Vec<ReplicaSet>> {
    // The selector narrows the list on the server when it is plain labels.
    let selector = deployment.spec.as_ref().and_then(|s| s.selector.match_labels.as_ref()).map(|labels| labels.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","));
    let params = match selector {
        Some(selector) if !selector.is_empty() => ListParams::default().labels(&selector),
        _ => ListParams::default(),
    };
    let uid = deployment.metadata.uid.clone();
    let list = Api::<ReplicaSet>::namespaced(client.clone(), namespace).list(&params).await?;
    Ok(list.items.into_iter().filter(|rs| rs.metadata.owner_references.iter().flatten().any(|o| Some(&o.uid) == uid.as_ref())).collect())
}

fn revision_of(meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta) -> Option<i64> {
    meta.annotations.as_ref()?.get(REVISION)?.parse().ok()
}

/// The template as deployed: without the hash label the ReplicaSet added.
fn deployed_template(rs: &ReplicaSet) -> Option<PodTemplateSpec> {
    let mut template = rs.spec.as_ref()?.template.clone()?;
    if let Some(labels) = template.metadata.as_mut().and_then(|m| m.labels.as_mut()) {
        labels.remove(HASH_LABEL);
    }
    Some(template)
}

fn revisions(deployment: &Deployment, owned: &[ReplicaSet]) -> Vec<Revision> {
    let current = revision_of(&deployment.metadata);
    let mut out: Vec<Revision> = owned
        .iter()
        .filter_map(|rs| {
            let number = revision_of(&rs.metadata)?;
            let template = deployed_template(rs)?;
            let images = template.spec.as_ref().map(|s| s.containers.iter().filter_map(|c| c.image.clone()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            Some(Revision {
                number,
                age: rs.metadata.creation_timestamp.as_ref().map(|t| crate::humanize_age(t.0)).unwrap_or_else(|| "-".into()),
                images,
                cause: rs.metadata.annotations.as_ref().and_then(|a| a.get(CHANGE_CAUSE)).cloned().unwrap_or_default(),
                pods: Some(rs.status.as_ref().map(|s| s.replicas).unwrap_or(0)),
                current: Some(number) == current,
                template: serde_yaml::to_string(&template).unwrap_or_default(),
            })
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.number));
    out
}

/// What a StatefulSet or DaemonSet is: its uid, selector, and the revision it is
/// rolling to (a StatefulSet says; a DaemonSet's is its newest).
async fn controller_owner(client: &Client, kind: &str, namespace: &str, name: &str) -> Result<(Option<String>, Option<std::collections::BTreeMap<String, String>>, Option<String>)> {
    Ok(match kind {
        "StatefulSet" => {
            let sts = Api::<StatefulSet>::namespaced(client.clone(), namespace).get(name).await?;
            (sts.metadata.uid, sts.spec.and_then(|s| s.selector.match_labels), sts.status.and_then(|s| s.update_revision))
        }
        _ => {
            let ds = Api::<DaemonSet>::namespaced(client.clone(), namespace).get(name).await?;
            (ds.metadata.uid, ds.spec.and_then(|s| s.selector.match_labels), None)
        }
    })
}

async fn owned_revisions(client: &Client, kind: &str, namespace: &str, name: &str) -> Result<(Vec<ControllerRevision>, Option<String>)> {
    let (uid, selector, current) = controller_owner(client, kind, namespace, name).await?;
    let selector = selector.map(|labels| labels.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")).filter(|s| !s.is_empty());
    let params = selector.map(|s| ListParams::default().labels(&s)).unwrap_or_default();
    let list = Api::<ControllerRevision>::namespaced(client.clone(), namespace).list(&params).await?;
    let owned = list.items.into_iter().filter(|r| r.metadata.owner_references.iter().flatten().any(|o| Some(&o.uid) == uid.as_ref())).collect();
    Ok((owned, current))
}

/// The pod template a ControllerRevision stores, under `spec.template`.
fn revision_template(revision: &ControllerRevision) -> Option<PodTemplateSpec> {
    let mut template = revision.data.as_ref()?.0.get("spec")?.get("template")?.clone();
    if let Some(map) = template.as_object_mut() {
        map.remove("$patch");
    }
    serde_json::from_value(template).ok()
}

fn controller_revisions(owned: &[ControllerRevision], current_name: Option<&str>) -> Vec<Revision> {
    let newest = owned.iter().map(|r| r.revision).max();
    let mut out: Vec<Revision> = owned
        .iter()
        .filter_map(|r| {
            let template = revision_template(r)?;
            let images = template.spec.as_ref().map(|s| s.containers.iter().filter_map(|c| c.image.clone()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let current = match current_name {
                Some(name) => r.metadata.name.as_deref() == Some(name),
                None => Some(r.revision) == newest,
            };
            Some(Revision {
                number: r.revision,
                age: r.metadata.creation_timestamp.as_ref().map(|t| crate::humanize_age(t.0)).unwrap_or_else(|| "-".into()),
                images,
                cause: r.metadata.annotations.as_ref().and_then(|a| a.get(CHANGE_CAUSE)).cloned().unwrap_or_default(),
                pods: None,
                current,
                template: serde_yaml::to_string(&template).unwrap_or_default(),
            })
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.number));
    out
}

async fn controller_history(client: &Client, kind: &str, namespace: &str, name: &str) -> Result<Vec<Revision>> {
    let (owned, current) = owned_revisions(client, kind, namespace, name).await?;
    Ok(controller_revisions(&owned, current.as_deref()))
}

/// Applies revision `number`'s stored template, as `kubectl rollout undo` does.
async fn controller_rollback(client: &Client, kind: &str, namespace: &str, name: &str, number: i64) -> Result<String> {
    let (owned, current) = owned_revisions(client, kind, namespace, name).await?;
    if controller_revisions(&owned, current.as_deref()).iter().any(|r| r.current && r.number == number) {
        bail!("{} {name} already runs revision {number}", kind.to_lowercase());
    }
    let source = owned.iter().find(|r| r.revision == number).with_context(|| format!("revision {number} is gone"))?;
    let data = source.data.as_ref().context("that revision has nothing stored")?.0.clone();
    let patch = Patch::Strategic(data);
    match kind {
        "StatefulSet" => {
            Api::<StatefulSet>::namespaced(client.clone(), namespace).patch(name, &PatchParams::default(), &patch).await?;
        }
        _ => {
            Api::<DaemonSet>::namespaced(client.clone(), namespace).patch(name, &PatchParams::default(), &patch).await?;
        }
    }
    Ok(format!("{}/{name} rolled back to revision {number}", kind.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rs(revision: &str, image: &str) -> ReplicaSet {
        serde_json::from_value(serde_json::json!({
            "metadata": {"name": format!("web-{revision}"), "annotations": {REVISION: revision}, "ownerReferences": [{"apiVersion": "apps/v1", "kind": "Deployment", "name": "web", "uid": "u1"}]},
            "spec": {"selector": {}, "template": {"metadata": {"labels": {"app": "web", HASH_LABEL: "abc"}}, "spec": {"containers": [{"name": "web", "image": image}]}}},
            "status": {"replicas": 0}
        }))
        .unwrap()
    }

    #[test]
    fn revisions_are_newest_first_with_the_running_one_marked() {
        let deployment: Deployment = serde_json::from_value(serde_json::json!({"metadata": {"name": "web", "uid": "u1", "annotations": {REVISION: "3"}}})).unwrap();
        let list = revisions(&deployment, &[rs("1", "web:1"), rs("3", "web:3"), rs("2", "web:2")]);
        assert_eq!(list.iter().map(|r| r.number).collect::<Vec<_>>(), [3, 2, 1]);
        assert!(list[0].current && !list[1].current);
        assert_eq!(list[1].images, "web:2");
    }

    #[test]
    fn controller_revisions_read_their_stored_template() {
        let revision = |n: i64, name: &str, image: &str| -> ControllerRevision {
            serde_json::from_value(serde_json::json!({
                "metadata": {"name": name},
                "revision": n,
                "data": {"spec": {"template": {"$patch": "replace", "metadata": {"labels": {"app": "db"}}, "spec": {"containers": [{"name": "db", "image": image}]}}}}
            }))
            .unwrap()
        };
        let owned = [revision(1, "db-aaa", "postgres:15"), revision(2, "db-bbb", "postgres:16")];
        let list = controller_revisions(&owned, Some("db-aaa"));
        assert_eq!(list.iter().map(|r| (r.number, r.current, r.images.as_str())).collect::<Vec<_>>(), [(2, false, "postgres:16"), (1, true, "postgres:15")]);
        assert!(!list[0].template.contains("$patch"));
        // A DaemonSet doesn't say which it runs: its newest.
        assert!(controller_revisions(&owned, None)[0].current);
    }

    #[test]
    fn the_hash_label_is_not_part_of_the_template() {
        let template = deployed_template(&rs("1", "web:1")).unwrap();
        let labels = template.metadata.unwrap().labels.unwrap();
        assert!(labels.contains_key("app") && !labels.contains_key(HASH_LABEL));
    }
}
