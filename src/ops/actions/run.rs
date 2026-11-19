//! Runs an action on its targets in the background.

use crate::ops::NoticeTone;
use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, Result, bail};
use kube::{
    Client,
    api::{Api, ApiResource, DeleteParams, DynamicObject, Patch, PatchParams, PostParams},
    core::GroupVersionKind,
    discovery::{Scope, pinned_kind},
};
use serde_json::json;

use super::{Action, Target};
use crate::ops::Outcome;

/// How far a batch has got, read by the UI while it runs in the background.
#[derive(Default)]
pub struct Progress {
    pub done: std::sync::atomic::AtomicUsize,
    pub total: std::sync::atomic::AtomicUsize,
}

impl Progress {
    pub fn get(&self) -> (usize, usize) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.done.load(Relaxed), self.total.load(Relaxed))
    }
}

const CONCURRENCY: usize = 8;

/// What the API server said about each kind, looked up once per batch.
type Resolved = HashMap<(String, String), std::result::Result<(ApiResource, Scope), String>>;

/// Runs one action on every target, a few at a time, reporting successes as a count
/// and failures by name.
pub async fn run_many(client: Client, targets: Vec<Target>, action: Action, progress: Arc<Progress>) -> Outcome {
    use futures::StreamExt;
    use std::sync::atomic::Ordering::Relaxed;
    progress.total.store(targets.len(), Relaxed);
    let mut resolved: Resolved = HashMap::new();
    for target in &targets {
        let key = (target.api_version.clone(), target.kind.clone());
        if let std::collections::hash_map::Entry::Vacant(e) = resolved.entry(key) {
            let found = resolve(&client, target).await.map_err(|e| format!("{e:#}"));
            e.insert(found);
        }
    }
    let resolved = Arc::new(resolved);
    let total = targets.len();
    let results: Vec<(String, Result<String>)> = futures::stream::iter(targets)
        .map(|target| {
            let (client, resolved, progress) = (client.clone(), Arc::clone(&resolved), Arc::clone(&progress));
            async move {
                let outcome = match &resolved[&(target.api_version.clone(), target.kind.clone())] {
                    Ok((resource, scope)) => perform(&client, &target, action, api_from(&client, &target, resource, scope)).await,
                    Err(e) => Err(anyhow::anyhow!("{e}")),
                };
                progress.done.fetch_add(1, Relaxed);
                (target.label(), outcome)
            }
        })
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
    if let [(_, one)] = results.as_slice() {
        return match one {
            Ok(text) => Outcome { text: text.clone(), tone: NoticeTone::Done },
            Err(e) => Outcome { text: format!("{e:#}"), tone: NoticeTone::Failed },
        };
    }
    let failures: Vec<String> = results.iter().filter_map(|(label, r)| r.as_ref().err().map(|e| format!("{label}: {e:#}"))).collect();
    let done = total - failures.len();
    if failures.is_empty() {
        Outcome { text: format!("{}: {done} of {total}", action_name(action)), tone: NoticeTone::Done }
    } else {
        Outcome { text: format!("{}: {done} of {total}\n{}", action_name(action), failures.join("\n")), tone: NoticeTone::Failed }
    }
}

/// `Deleting Pod shop/web-1`, or `Deleting 12 objects`.
pub fn working_title(action: Action, targets: &[Target]) -> String {
    let verb = match action {
        Action::Delete => "Deleting",
        Action::Scale(_) => "Scaling",
        Action::Restart => "Restarting",
        Action::Cordon(true) => "Cordoning",
        Action::Cordon(false) => "Uncordoning",
        Action::Drain => "Draining",
        Action::Trigger => "Starting a job from",
        Action::Suspend(true) => "Suspending",
        Action::Suspend(false) => "Resuming",
        Action::Rollback(_) => "Rolling back",
        Action::Debug(_) => "Debugging",
    };
    match targets {
        [one] => format!("{verb} {}", one.label()),
        many => format!("{verb} {} objects", many.len()),
    }
}

pub(super) fn action_name(action: Action) -> &'static str {
    match action {
        Action::Delete => "Deleted",
        Action::Scale(_) => "Scaled",
        Action::Restart => "Restarted",
        Action::Cordon(_) => "Cordoned",
        Action::Drain => "Drained",
        Action::Trigger => "Triggered",
        Action::Suspend(_) => "Suspended",
        Action::Rollback(_) => "Rolled back",
        Action::Debug(_) => "Debugged",
    }
}

async fn resolve(client: &Client, target: &Target) -> Result<(ApiResource, Scope)> {
    let type_meta = kube::api::TypeMeta { api_version: target.api_version.clone(), kind: target.kind.clone() };
    let gvk = GroupVersionKind::try_from(&type_meta)?;
    let (resource, caps) = pinned_kind(client, &gvk).await?;
    Ok((resource, caps.scope))
}

pub(super) fn api_from(client: &Client, target: &Target, resource: &ApiResource, scope: &Scope) -> Api<DynamicObject> {
    match (scope, target.namespace.as_deref()) {
        (Scope::Namespaced, Some(ns)) => Api::namespaced_with(client.clone(), ns, resource),
        _ => Api::all_with(client.clone(), resource),
    }
}

/// Deleting asks for the very object that was shown: if it was replaced meanwhile (a pod the
/// controller recreated under the same name), the server refuses instead of deleting the new one.
fn delete_params(target: &Target) -> DeleteParams {
    let uid = target.manifest.get("metadata").and_then(|m| m.get("uid")).and_then(serde_yaml::Value::as_str).map(str::to_string);
    DeleteParams { preconditions: uid.map(|uid| kube::api::Preconditions { uid: Some(uid), resource_version: None }), ..DeleteParams::default() }
}

async fn perform(client: &Client, target: &Target, action: Action, api: Api<DynamicObject>) -> Result<String> {
    let name = target.name.as_str();
    let merge = |body: serde_json::Value| Patch::Merge(body);
    let params = PatchParams::default();
    let label = target.label();
    match action {
        Action::Delete => {
            api.delete(name, &delete_params(target)).await?;
            Ok(format!("Deleted {label}"))
        }
        Action::Scale(replicas) => {
            if !target.scalable() {
                bail!("{label} can't be scaled");
            }
            api.patch_scale(name, &params, &merge(json!({ "spec": { "replicas": replicas } }))).await?;
            Ok(format!("Scaled {label} to {replicas}"))
        }
        Action::Restart => {
            if !target.restartable() {
                bail!("{label} can't be restarted");
            }
            let now = k8s_openapi::jiff::Timestamp::now().to_string();
            let body = json!({ "spec": { "template": { "metadata": { "annotations": { "kubectl.kubernetes.io/restartedAt": now } } } } });
            api.patch(name, &params, &merge(body)).await?;
            Ok(format!("Restarting {label}"))
        }
        Action::Cordon(on) => {
            api.patch(name, &params, &merge(json!({ "spec": { "unschedulable": on } }))).await?;
            Ok(format!("{} {label}", if on { "Cordoned" } else { "Uncordoned" }))
        }
        Action::Drain => {
            if target.kind != "Node" {
                bail!("{label} can't be drained");
            }
            drain(client, &api, target).await
        }
        Action::Suspend(on) => {
            api.patch(name, &params, &merge(json!({ "spec": { "suspend": on } }))).await?;
            Ok(format!("{} {label}", if on { "Suspended" } else { "Resumed" }))
        }
        Action::Rollback(revision) => {
            if !crate::k8s::rollout::has_history(&target.kind) {
                bail!("{label} has no rollout history");
            }
            let namespace = target.namespace.as_deref().context("a workload has a namespace")?;
            crate::k8s::rollout::rollback(client, &target.kind, namespace, name, revision).await
        }
        Action::Debug(_) => bail!("a debug container opens a shell, it doesn't run in the background"),
        Action::Trigger => {
            let job = job_from_cronjob(target, k8s_openapi::jiff::Timestamp::now().as_second())?;
            let namespace = target.namespace.as_deref().context("a CronJob has a namespace")?;
            let jobs: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &ApiResource::from_gvk(&GroupVersionKind::gvk("batch", "v1", "Job")));
            let created = jobs.create(&PostParams::default(), &serde_json::from_value(job)?).await?;
            Ok(format!("Started job {}", created.metadata.name.unwrap_or_default()))
        }
    }
}

/// What drain does with a pod: leave it, or evict it.
#[derive(Debug, PartialEq)]
enum Eviction {
    Evict,
    Leave(&'static str),
}

/// A DaemonSet's pod would come straight back and a mirror pod can't be evicted. A pod nothing
/// controls would be lost for good, so it stays, like `kubectl drain` without `--force`.
fn eviction_of(pod: &k8s_openapi::api::core::v1::Pod) -> Eviction {
    let owners = pod.metadata.owner_references.as_deref().unwrap_or_default();
    if pod.metadata.annotations.as_ref().is_some_and(|a| a.contains_key("kubernetes.io/config.mirror")) {
        Eviction::Leave("a static pod")
    } else if pod.metadata.deletion_timestamp.is_some() {
        Eviction::Leave("already terminating")
    } else if owners.iter().any(|o| o.kind == "DaemonSet") {
        Eviction::Leave("a DaemonSet's pod")
    } else if !owners.iter().any(|o| o.controller == Some(true)) && !matches!(pod.status.as_ref().and_then(|s| s.phase.as_deref()), Some("Succeeded" | "Failed")) {
        Eviction::Leave("nothing controls it")
    } else {
        Eviction::Evict
    }
}

/// Cordons the node, then evicts what can be evicted, retrying the pods a disruption budget holds
/// back for a few seconds before saying which stayed.
async fn drain(client: &Client, nodes: &Api<DynamicObject>, target: &Target) -> Result<String> {
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::{EvictParams, ListParams};
    nodes.patch(&target.name, &PatchParams::default(), &Patch::Merge(json!({ "spec": { "unschedulable": true } }))).await?;
    let all: Api<Pod> = Api::all(client.clone());
    let listed = all.list(&ListParams::default().fields(&format!("spec.nodeName={}", target.name))).await?;
    let mut left: Vec<Pod> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    for pod in listed.items {
        match eviction_of(&pod) {
            Eviction::Evict => left.push(pod),
            Eviction::Leave(why) => kept.push(format!("{} ({why})", pod.metadata.name.as_deref().unwrap_or_default())),
        }
    }
    let total = left.len();
    for attempt in 0..4 {
        let mut blocked = Vec::new();
        for pod in left {
            let (Some(namespace), Some(name)) = (pod.metadata.namespace.as_deref(), pod.metadata.name.as_deref()) else { continue };
            let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
            match api.evict(name, &EvictParams::default()).await {
                Ok(_) => {}
                Err(kube::Error::Api(e)) if e.code == 404 => {}
                Err(kube::Error::Api(e)) if e.code == 429 => blocked.push(pod),
                Err(e) => return Err(e).with_context(|| format!("evicting {namespace}/{name}")),
            }
        }
        left = blocked;
        if left.is_empty() || attempt == 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
    let evicted = total - left.len();
    if !left.is_empty() {
        let names: Vec<String> = left.iter().map(|p| format!("{}/{}", p.metadata.namespace.as_deref().unwrap_or("-"), p.metadata.name.as_deref().unwrap_or_default())).collect();
        bail!("Drained {} partly: {evicted} of {total} pods evicted, a disruption budget holds back {}", target.label(), names.join(", "));
    }
    let note = if kept.is_empty() { String::new() } else { format!("; left {}", kept.join(", ")) };
    Ok(format!("Drained {}: {evicted} pod{} evicted{note}", target.label(), if evicted == 1 { "" } else { "s" }))
}

/// `<cronjob>-manual-<stamp>` in at most 63 characters. It is the CronJob's name that gives way,
/// never the stamp, so two triggers of a long-named CronJob don't collide.
fn manual_job_name(cronjob: &str, stamp: i64) -> String {
    let suffix = format!("-manual-{stamp:x}");
    let room = 63usize.saturating_sub(suffix.len());
    format!("{}{suffix}", cronjob.chars().take(room).collect::<String>())
}

/// The Job a CronJob would create, owned by it and named `<cronjob>-manual-<n>`,
/// like `kubectl create job --from=cronjob/...`.
pub(super) fn job_from_cronjob(target: &Target, stamp: i64) -> Result<serde_json::Value> {
    let cronjob = serde_json::to_value(&target.manifest)?;
    let template = cronjob.pointer("/spec/jobTemplate").context("the CronJob has no jobTemplate")?;
    let uid = cronjob.pointer("/metadata/uid").cloned().unwrap_or_default();
    let name = manual_job_name(&target.name, stamp);
    let mut annotations = template.pointer("/metadata/annotations").cloned().unwrap_or_else(|| json!({}));
    annotations["cronjob.kubernetes.io/instantiate"] = json!("manual");
    Ok(json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": name,
            "labels": template.pointer("/metadata/labels").cloned().unwrap_or_else(|| json!({})),
            "annotations": annotations,
            "ownerReferences": [{
                "apiVersion": "batch/v1", "kind": "CronJob", "name": target.name, "uid": uid,
                "controller": true, "blockOwnerDeletion": true,
            }],
        },
        "spec": template.get("spec").cloned().unwrap_or_else(|| json!({})),
    }))
}

#[cfg(test)]
mod drain_tests {
    use super::*;

    fn pod(yaml: &str) -> k8s_openapi::api::core::v1::Pod {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn drain_leaves_what_would_come_back_or_never_return() {
        let owned = pod("metadata:\n  name: a\n  ownerReferences:\n  - {apiVersion: apps/v1, kind: ReplicaSet, name: r, uid: u, controller: true}\n");
        assert_eq!(eviction_of(&owned), Eviction::Evict);
        let daemon = pod("metadata:\n  name: a\n  ownerReferences:\n  - {apiVersion: apps/v1, kind: DaemonSet, name: d, uid: u, controller: true}\n");
        assert_eq!(eviction_of(&daemon), Eviction::Leave("a DaemonSet's pod"));
        assert_eq!(eviction_of(&pod("metadata:\n  name: a\n")), Eviction::Leave("nothing controls it"));
        let finished = pod("metadata:\n  name: a\nstatus:\n  phase: Succeeded\n");
        assert_eq!(eviction_of(&finished), Eviction::Evict);
        let mirror = pod("metadata:\n  name: a\n  annotations:\n    kubernetes.io/config.mirror: x\n");
        assert_eq!(eviction_of(&mirror), Eviction::Leave("a static pod"));
    }
}

#[cfg(test)]
mod job_name_tests {
    use super::manual_job_name;

    #[test]
    fn a_long_cronjob_name_gives_way_not_the_stamp() {
        let long = "a".repeat(60);
        let (first, second) = (manual_job_name(&long, 0x1111), manual_job_name(&long, 0x2222));
        assert!(first.len() <= 63 && second.len() <= 63);
        assert_ne!(first, second);
        assert!(first.ends_with("-manual-1111"));
        assert_eq!(manual_job_name("nightly", 0xab), "nightly-manual-ab");
    }
}

#[cfg(test)]
mod delete_tests {
    use super::*;

    fn target(manifest: serde_yaml::Value) -> Target {
        Target { api_version: "v1".into(), kind: "Pod".into(), name: "web-1".into(), namespace: Some("shop".into()), manifest }
    }

    #[test]
    fn a_delete_names_the_uid_it_was_shown_with() {
        let shown = target(serde_yaml::from_str("metadata:\n  name: web-1\n  uid: abc-123\n").unwrap());
        let params = delete_params(&shown);
        assert_eq!(params.preconditions.and_then(|p| p.uid).as_deref(), Some("abc-123"));
        let unknown = target(serde_yaml::from_str("metadata:\n  name: web-1\n").unwrap());
        assert!(delete_params(&unknown).preconditions.is_none());
    }
}
