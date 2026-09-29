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
        if !resolved.contains_key(&key) {
            let found = resolve(&client, target).await.map_err(|e| format!("{e:#}"));
            resolved.insert(key, found);
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
        Action::Trigger => "Starting a job from",
        Action::Suspend(true) => "Suspending",
        Action::Suspend(false) => "Resuming",
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
        Action::Trigger => "Triggered",
        Action::Suspend(_) => "Suspended",
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

async fn perform(client: &Client, target: &Target, action: Action, api: Api<DynamicObject>) -> Result<String> {
    let name = target.name.as_str();
    let merge = |body: serde_json::Value| Patch::Merge(body);
    let params = PatchParams::default();
    let label = target.label();
    match action {
        Action::Delete => {
            api.delete(name, &DeleteParams::default()).await?;
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
        Action::Suspend(on) => {
            api.patch(name, &params, &merge(json!({ "spec": { "suspend": on } }))).await?;
            Ok(format!("{} {label}", if on { "Suspended" } else { "Resumed" }))
        }
        Action::Trigger => {
            let job = job_from_cronjob(target, k8s_openapi::jiff::Timestamp::now().as_second())?;
            let namespace = target.namespace.as_deref().context("a CronJob has a namespace")?;
            let jobs: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &ApiResource::from_gvk(&GroupVersionKind::gvk("batch", "v1", "Job")));
            let created = jobs.create(&PostParams::default(), &serde_json::from_value(job)?).await?;
            Ok(format!("Started job {}", created.metadata.name.unwrap_or_default()))
        }
    }
}

/// The Job a CronJob would create, owned by it and named `<cronjob>-manual-<n>`,
/// like `kubectl create job --from=cronjob/...`.
pub(super) fn job_from_cronjob(target: &Target, stamp: i64) -> Result<serde_json::Value> {
    let cronjob = serde_json::to_value(&target.manifest)?;
    let template = cronjob.pointer("/spec/jobTemplate").context("the CronJob has no jobTemplate")?;
    let uid = cronjob.pointer("/metadata/uid").cloned().unwrap_or_default();
    let mut name = format!("{}-manual-{stamp:x}", target.name);
    name.truncate(63);
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

