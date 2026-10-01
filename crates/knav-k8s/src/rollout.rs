//! A Deployment's rollout history, from the ReplicaSets it owns, and rolling back to
//! one of them the way `kubectl rollout undo` does.

use anyhow::{Context as _, Result, bail};
use k8s_openapi::api::{
    apps::v1::{Deployment, ReplicaSet},
    core::v1::PodTemplateSpec,
};
use kube::{
    Api, Client,
    api::{ListParams, PostParams},
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
    pub pods: i32,
    /// What the Deployment runs now.
    pub current: bool,
    /// The pod template as YAML, for comparing revisions.
    pub template: String,
}

/// Every revision the Deployment still has a ReplicaSet for, newest first.
pub async fn history(client: &Client, namespace: &str, name: &str) -> Result<Vec<Revision>> {
    let deployment = Api::<Deployment>::namespaced(client.clone(), namespace).get(name).await?;
    let owned = owned_replica_sets(client, namespace, &deployment).await?;
    Ok(revisions(&deployment, &owned))
}

/// Puts revision `number`'s pod template back on the Deployment, which rolls it out.
pub async fn rollback(client: &Client, namespace: &str, name: &str, number: i64) -> Result<String> {
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
                pods: rs.status.as_ref().map(|s| s.replicas).unwrap_or(0),
                current: Some(number) == current,
                template: serde_yaml::to_string(&template).unwrap_or_default(),
            })
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.number));
    out
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
    fn the_hash_label_is_not_part_of_the_template() {
        let template = deployed_template(&rs("1", "web:1")).unwrap();
        let labels = template.metadata.unwrap().labels.unwrap();
        assert!(labels.contains_key("app") && !labels.contains_key(HASH_LABEL));
    }
}
