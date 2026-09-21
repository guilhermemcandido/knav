//! Things to do to the selected object: delete, scale, restart, cordon, trigger or
//! suspend a CronJob, open a shell. Each works on a `Target` read from the manifest,
//! so it applies to any kind.

use anyhow::{Context as _, Result, bail};
use kube::{
    Client,
    api::{Api, ApiResource, DeleteParams, DynamicObject, Patch, PatchParams, PostParams},
    core::GroupVersionKind,
    discovery::{Scope, pinned_kind},
};
use serde_json::json;

pub use crate::ops::edit::Outcome;

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub manifest: serde_yaml::Value,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Delete,
    Scale(i32),
    Restart,
    /// `true` marks the node unschedulable (cordon), `false` undoes it.
    Cordon(bool),
    Trigger,
    /// `true` suspends the CronJob, `false` resumes it.
    Suspend(bool),
}

impl Target {
    pub fn from_manifest(manifest: &serde_yaml::Value) -> Option<Target> {
        let text = |path: &[&str]| {
            let mut value = manifest;
            for key in path {
                value = value.get(*key)?;
            }
            value.as_str().map(str::to_string)
        };
        Some(Target {
            api_version: text(&["apiVersion"])?,
            kind: text(&["kind"])?,
            name: text(&["metadata", "name"])?,
            namespace: text(&["metadata", "namespace"]),
            manifest: manifest.clone(),
        })
    }

    /// `pod default/web-1`, or `node worker-1` when it has no namespace.
    pub fn label(&self) -> String {
        let kind = self.kind.to_lowercase();
        match &self.namespace {
            Some(ns) => format!("{kind} {ns}/{}", self.name),
            None => format!("{kind} {}", self.name),
        }
    }

    pub fn scalable(&self) -> bool {
        matches!(self.kind.as_str(), "Deployment" | "StatefulSet" | "ReplicaSet")
    }

    pub fn restartable(&self) -> bool {
        matches!(self.kind.as_str(), "Deployment" | "StatefulSet" | "DaemonSet")
    }

    /// What `kubectl port-forward` calls this object (`pod/x`, `svc/x`, `deploy/x`).
    pub fn forward_resource(&self) -> Option<String> {
        let prefix = match self.kind.as_str() {
            "Pod" => "pod",
            "Service" => "svc",
            "Deployment" => "deploy",
            _ => return None,
        };
        Some(format!("{prefix}/{}", self.name))
    }

    /// Every port the object declares (container ports, or a Service's
    /// ports), in order and without repeats.
    pub fn ports(&self) -> Vec<u16> {
        let Some(spec) = self.manifest.get("spec") else { return Vec::new() };
        let containers = spec.get("containers").or_else(|| spec.get("template")?.get("spec")?.get("containers"));
        let mut ports: Vec<u64> = containers
            .and_then(|c| c.as_sequence())
            .into_iter()
            .flatten()
            .flat_map(|c| c.get("ports").and_then(|p| p.as_sequence()).into_iter().flatten())
            .filter_map(|p| p.get("containerPort")?.as_u64())
            .collect();
        ports.extend(spec.get("ports").and_then(|p| p.as_sequence()).into_iter().flatten().filter_map(|p| p.get("port")?.as_u64()));
        let mut seen = Vec::new();
        for port in ports.into_iter().filter_map(|p| u16::try_from(p).ok()) {
            if !seen.contains(&port) {
                seen.push(port);
            }
        }
        seen
    }

    /// The desired replica count now, to pre-fill the scale prompt.
    pub fn replicas(&self) -> i64 {
        self.manifest.get("spec").and_then(|s| s.get("replicas")).and_then(|r| r.as_i64()).unwrap_or(1)
    }

    fn flag(&self, key: &str) -> bool {
        self.manifest.get("spec").and_then(|s| s.get(key)).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// The action for `o` on a node: cordon if it is schedulable, else undo.
    pub fn cordon_action(&self) -> Option<Action> {
        (self.kind == "Node").then(|| Action::Cordon(!self.flag("unschedulable")))
    }

    /// The action for `u` on a CronJob: suspend if it is running, else resume.
    pub fn suspend_action(&self) -> Option<Action> {
        (self.kind == "CronJob").then(|| Action::Suspend(!self.flag("suspend")))
    }
}

impl Action {
    /// What to ask before doing it, for the actions that ask.
    pub fn confirmation(self, target: &Target) -> Option<String> {
        match self {
            Action::Delete if target.kind == "Namespace" => {
                Some(format!("Delete {}? This removes everything in it.", target.label()))
            }
            Action::Delete => Some(format!("Delete {}?", target.label())),
            Action::Restart => Some(format!("Restart {}?", target.label())),
            Action::Trigger => Some(format!("Run {} now?", target.label())),
            Action::Suspend(true) => Some(format!("Suspend {}?", target.label())),
            Action::Suspend(false) => Some(format!("Resume {}?", target.label())),
            _ => None,
        }
    }
}

/// What to ask before running `action` on `targets`, for the actions that ask.
pub fn confirm_text(action: Action, targets: &[Target]) -> Option<String> {
    if let [one] = targets {
        return action.confirmation(one);
    }
    let verb = match action {
        Action::Delete => "Delete",
        Action::Restart => "Restart",
        _ => return None,
    };
    let kind = targets.first().map(|t| t.kind.to_lowercase()).unwrap_or_default();
    let plural = if kind.ends_with('s') { format!("{kind}es") } else if let Some(stem) = kind.strip_suffix('y') { format!("{stem}ies") } else { format!("{kind}s") };
    Some(format!("{verb} {} {plural}?", targets.len()))
}

/// Runs one action on each target, reporting the successes as a count and
/// the failures by name.
pub fn run_many(client: &Client, targets: &[Target], action: Action) -> Outcome {
    if let [one] = targets {
        return run(client, one, action);
    }
    let mut failures = Vec::new();
    for target in targets {
        let outcome = run(client, target, action);
        if outcome.error {
            failures.push(format!("{}: {}", target.label(), outcome.text));
        }
    }
    let done = targets.len() - failures.len();
    if failures.is_empty() {
        Outcome { text: format!("{}: {done} of {}", action_name(action), targets.len()), error: false }
    } else {
        Outcome { text: format!("{}: {done} of {}\n{}", action_name(action), targets.len(), failures.join("\n")), error: true }
    }
}

fn action_name(action: Action) -> &'static str {
    match action {
        Action::Delete => "Deleted",
        Action::Scale(_) => "Scaled",
        Action::Restart => "Restarted",
        Action::Cordon(_) => "Cordoned",
        Action::Trigger => "Triggered",
        Action::Suspend(_) => "Suspended",
    }
}

pub fn run(client: &Client, target: &Target, action: Action) -> Outcome {
    match block(perform(client, target, action)) {
        Ok(text) => Outcome { text, error: false },
        Err(e) => Outcome { text: format!("{e:#}"), error: true },
    }
}

fn block<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}

async fn api_for(client: &Client, target: &Target) -> Result<Api<DynamicObject>> {
    let type_meta = kube::api::TypeMeta { api_version: target.api_version.clone(), kind: target.kind.clone() };
    let gvk = GroupVersionKind::try_from(&type_meta)?;
    let (resource, caps) = pinned_kind(client, &gvk).await?;
    Ok(match (caps.scope, target.namespace.as_deref()) {
        (Scope::Namespaced, Some(ns)) => Api::namespaced_with(client.clone(), ns, &resource),
        _ => Api::all_with(client.clone(), &resource),
    })
}

async fn perform(client: &Client, target: &Target, action: Action) -> Result<String> {
    let api = api_for(client, target).await?;
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

/// The Job a CronJob would create, owned by it, named `<cronjob>-manual-<n>`
/// (what `kubectl create job --from=cronjob/...` makes).
fn job_from_cronjob(target: &Target, stamp: i64) -> Result<serde_json::Value> {
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

/// A Secret's manifest with each `data` value decoded from base64, for
/// reading it. A value that isn't UTF-8 text shows its size instead.
pub fn decode_secret(manifest: &serde_yaml::Value) -> serde_yaml::Value {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut decoded = manifest.clone();
    if let Some(data) = decoded.get_mut("data").and_then(|d| d.as_mapping_mut()) {
        for (_, value) in data.iter_mut() {
            let Some(encoded) = value.as_str() else { continue };
            *value = match STANDARD.decode(encoded.trim()) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => serde_yaml::Value::String(text),
                    Err(e) => serde_yaml::Value::String(format!("<binary, {} bytes>", e.as_bytes().len())),
                },
                Err(_) => serde_yaml::Value::String(format!("<not base64: {encoded}>")),
            };
        }
    }
    decoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(yaml: &str) -> Target {
        Target::from_manifest(&serde_yaml::from_str(yaml).unwrap()).unwrap()
    }

    const DEPLOYMENT: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata: {name: web, namespace: shop}\nspec: {replicas: 3}\n";

    #[test]
    fn a_target_is_read_from_the_manifest() {
        let t = target(DEPLOYMENT);
        assert_eq!((t.api_version.as_str(), t.kind.as_str(), t.name.as_str()), ("apps/v1", "Deployment", "web"));
        assert_eq!(t.namespace.as_deref(), Some("shop"));
        assert_eq!(t.label(), "deployment shop/web");
        assert_eq!(t.replicas(), 3);
    }

    #[test]
    fn a_manifest_without_an_identity_is_no_target() {
        assert!(Target::from_manifest(&serde_yaml::from_str("foo: bar").unwrap()).is_none());
    }

    #[test]
    fn cluster_scoped_objects_are_labelled_without_a_namespace() {
        assert_eq!(target("apiVersion: v1\nkind: Node\nmetadata: {name: n1}\n").label(), "node n1");
    }

    #[test]
    fn only_workloads_scale_and_restart() {
        assert!(target(DEPLOYMENT).scalable() && target(DEPLOYMENT).restartable());
        let daemon = target("apiVersion: apps/v1\nkind: DaemonSet\nmetadata: {name: d}\n");
        assert!(!daemon.scalable() && daemon.restartable());
        let pod = target("apiVersion: v1\nkind: Pod\nmetadata: {name: p}\n");
        assert!(!pod.scalable() && !pod.restartable());
    }

    #[test]
    fn forwardable_kinds_name_themselves_the_way_kubectl_does() {
        assert_eq!(target(DEPLOYMENT).forward_resource().as_deref(), Some("deploy/web"));
        assert_eq!(target("apiVersion: v1\nkind: Pod\nmetadata: {name: p}\n").forward_resource().as_deref(), Some("pod/p"));
        assert_eq!(target("apiVersion: v1\nkind: Node\nmetadata: {name: n}\n").forward_resource(), None);
    }

    #[test]
    fn the_first_exposed_port_comes_from_containers_or_service_ports() {
        let pod = target("apiVersion: v1\nkind: Pod\nmetadata: {name: p}\nspec: {containers: [{name: a}, {name: b, ports: [{containerPort: 9000}]}]}\n");
        assert_eq!(pod.ports(), [9000]);
        let dep = target("apiVersion: apps/v1\nkind: Deployment\nmetadata: {name: d}\nspec: {template: {spec: {containers: [{name: a, ports: [{containerPort: 80}]}]}}}\n");
        assert_eq!(dep.ports(), [80]);
        let svc = target("apiVersion: v1\nkind: Service\nmetadata: {name: s}\nspec: {ports: [{port: 443}]}\n");
        assert_eq!(svc.ports(), [443]);
        assert!(target(DEPLOYMENT).ports().is_empty());
    }

    #[test]
    fn every_declared_port_is_listed_once_and_the_hint_says_when_there_are_none() {
        let pod = target("apiVersion: v1\nkind: Pod\nmetadata: {name: p}\nspec: {containers: [{name: a, ports: [{containerPort: 80}, {containerPort: 443}]}, {name: b, ports: [{containerPort: 80}]}]}\n");
        assert_eq!(pod.ports(), [80, 443]);
    }

    #[test]
    fn cordon_and_suspend_flip_the_current_state() {
        let node = target("apiVersion: v1\nkind: Node\nmetadata: {name: n}\nspec: {unschedulable: true}\n");
        assert_eq!(node.cordon_action(), Some(Action::Cordon(false)));
        let fresh = target("apiVersion: v1\nkind: Node\nmetadata: {name: n}\n");
        assert_eq!(fresh.cordon_action(), Some(Action::Cordon(true)));
        let cron = target("apiVersion: batch/v1\nkind: CronJob\nmetadata: {name: c}\nspec: {suspend: false}\n");
        assert_eq!(cron.suspend_action(), Some(Action::Suspend(true)));
        assert_eq!(target(DEPLOYMENT).cordon_action(), None);
        assert_eq!(target(DEPLOYMENT).suspend_action(), None);
    }

    #[test]
    fn destructive_actions_ask_first_and_reversible_ones_do_not() {
        let t = target(DEPLOYMENT);
        assert_eq!(Action::Delete.confirmation(&t).as_deref(), Some("Delete deployment shop/web?"));
        assert!(Action::Restart.confirmation(&t).is_some());
        let cron = target("apiVersion: batch/v1\nkind: CronJob\nmetadata: {name: tick, namespace: d}\n");
        assert_eq!(Action::Trigger.confirmation(&cron).as_deref(), Some("Run cronjob d/tick now?"));
        assert_eq!(Action::Suspend(true).confirmation(&cron).as_deref(), Some("Suspend cronjob d/tick?"));
        assert_eq!(Action::Suspend(false).confirmation(&cron).as_deref(), Some("Resume cronjob d/tick?"));
        assert!(Action::Scale(2).confirmation(&t).is_none());
        assert!(Action::Cordon(true).confirmation(&t).is_none());
        let ns = target("apiVersion: v1\nkind: Namespace\nmetadata: {name: shop}\n");
        assert!(Action::Delete.confirmation(&ns).unwrap().contains("everything"));
    }

    #[test]
    fn secret_values_are_decoded_and_binary_ones_summarised() {
        let secret: serde_yaml::Value = serde_yaml::from_str("kind: Secret\ndata: {user: YWRtaW4=, blob: gA==, bad: '!!'}\n").unwrap();
        let decoded = decode_secret(&secret);
        assert_eq!(decoded["data"]["user"], "admin");
        assert_eq!(decoded["data"]["blob"], "<binary, 1 bytes>");
        assert_eq!(decoded["data"]["bad"], "<not base64: !!>");
        assert_eq!(decoded["kind"], "Secret");
    }

    #[test]
    fn asking_about_several_targets_counts_them() {
        let pods: Vec<Target> = (0..3).map(|i| target(&format!("apiVersion: v1\nkind: Pod\nmetadata: {{name: p{i}, namespace: d}}\n"))).collect();
        assert_eq!(confirm_text(Action::Delete, &pods).as_deref(), Some("Delete 3 pods?"));
        assert_eq!(confirm_text(Action::Delete, &pods[..1]).as_deref(), Some("Delete pod d/p0?"));
        let policies: Vec<Target> = (0..2).map(|i| target(&format!("apiVersion: v1\nkind: NetworkPolicy\nmetadata: {{name: p{i}}}\n"))).collect();
        assert_eq!(confirm_text(Action::Restart, &policies).as_deref(), Some("Restart 2 networkpolicies?"));
        assert_eq!(confirm_text(Action::Scale(2), &pods), None);
    }

    #[test]
    fn a_triggered_job_is_owned_by_its_cronjob() {
        let cron = target(
            "apiVersion: batch/v1\nkind: CronJob\nmetadata: {name: tick, namespace: d, uid: abc}\nspec:\n  schedule: '* * * * *'\n  jobTemplate:\n    spec: {template: {spec: {containers: [{name: c, image: busybox}]}}}\n",
        );
        let job = job_from_cronjob(&cron, 255).unwrap();
        assert_eq!(job["metadata"]["name"], "tick-manual-ff");
        assert_eq!(job["metadata"]["ownerReferences"][0]["uid"], "abc");
        assert_eq!(job["metadata"]["annotations"]["cronjob.kubernetes.io/instantiate"], "manual");
        assert_eq!(job["spec"]["template"]["spec"]["containers"][0]["image"], "busybox");
    }
}
