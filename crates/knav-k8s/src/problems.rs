//! Everything in the cluster that needs a look, across kinds, each with the reason in
//! a few words and the cluster's own explanation.

use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Node, Pod},
};

use crate::describe::Tone;
use crate::{GenericRow, ResourceKind};

pub struct Problem {
    pub kind: ResourceKind,
    pub namespace: Option<String>,
    pub name: String,
    /// Bad when broken, Warn when degraded or stuck.
    pub tone: Tone,
    /// In a few words, like `CrashLoopBackOff` or `1/3 ready`.
    pub reason: String,
    /// The cluster's explanation, when it gives one.
    pub detail: String,
    pub age: String,
    pub age_secs: i64,
}

impl Problem {
    pub fn place(&self) -> String {
        match &self.namespace {
            Some(ns) => format!("{ns}/{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// Broken first, then degraded; within each, the newest first.
pub fn sort(problems: &mut [Problem]) {
    problems.sort_by_key(|p| (p.tone != Tone::Bad, p.age_secs));
}

fn age(meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta) -> (String, i64) {
    let text = meta.creation_timestamp.as_ref().map(|t| crate::humanize_age(t.0)).unwrap_or_else(|| "-".into());
    (text, crate::age_seconds(meta.creation_timestamp.as_ref()))
}

pub fn pod(pod: &Pod) -> Option<Problem> {
    let reason = crate::pod_status(pod);
    let tone = crate::status_tone(&reason);
    if !matches!(tone, Tone::Warn | Tone::Bad) {
        return None;
    }
    Some(Problem {
        kind: ResourceKind::Pods,
        namespace: pod.metadata.namespace.clone(),
        name: pod.metadata.name.clone().unwrap_or_default(),
        tone,
        reason,
        detail: pod_detail(pod),
        age: age(&pod.metadata).0,
        age_secs: age(&pod.metadata).1,
    })
}

/// Why a pod is unwell, from the most telling place first: a waiting container's
/// message, the last crash, then why it can't be scheduled, then the pod's own message.
fn pod_detail(pod: &Pod) -> String {
    let Some(status) = pod.status.as_ref() else { return String::new() };
    let containers = status.init_container_statuses.iter().flatten().chain(status.container_statuses.iter().flatten());
    for c in containers {
        let waiting = c.state.as_ref().and_then(|s| s.waiting.as_ref());
        let crashed = c.last_state.as_ref().and_then(|s| s.terminated.as_ref());
        match (waiting, crashed) {
            (Some(w), Some(t)) if w.reason.as_deref() == Some("CrashLoopBackOff") => {
                let why = t.reason.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
                return format!("{} exited with code {}{why}, restarted {} times", c.name, t.exit_code, c.restart_count);
            }
            (Some(w), _) if w.message.is_some() => return format!("{}: {}", c.name, w.message.clone().unwrap_or_default()),
            _ => {}
        }
        if let Some(t) = c.state.as_ref().and_then(|s| s.terminated.as_ref()).filter(|t| t.exit_code != 0) {
            let why = t.reason.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
            return format!("{} exited with code {}{why}", c.name, t.exit_code);
        }
    }
    let unscheduled = status.conditions.iter().flatten().find(|c| c.type_ == "PodScheduled" && c.status == "False");
    if let Some(message) = unscheduled.and_then(|c| c.message.clone()) {
        return message;
    }
    status.message.clone().unwrap_or_default()
}

pub fn deployment(dep: &Deployment) -> Option<Problem> {
    let want = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    let status = dep.status.clone().unwrap_or_default();
    let ready = status.ready_replicas.unwrap_or(0);
    let condition = |kind: &str| status.conditions.iter().flatten().find(|c| c.type_ == kind).cloned();
    // A stalled rollout is broken even when the old pods still serve.
    let stalled = condition("Progressing").filter(|c| c.reason.as_deref() == Some("ProgressDeadlineExceeded"));
    if ready >= want && stalled.is_none() {
        return None;
    }
    let tone = if ready == 0 || stalled.is_some() { Tone::Bad } else { Tone::Warn };
    let reason = if stalled.is_some() { "Rollout stuck".to_string() } else { format!("{ready}/{want} ready") };
    let detail = stalled.or_else(|| condition("Available").filter(|c| c.status == "False")).and_then(|c| c.message).unwrap_or_default();
    let (age, age_secs) = age(&dep.metadata);
    Some(Problem { kind: ResourceKind::Deployments, namespace: dep.metadata.namespace.clone(), name: dep.metadata.name.clone().unwrap_or_default(), tone, reason, detail, age, age_secs })
}

pub fn node(node: &Node) -> Option<Problem> {
    let conditions: Vec<_> = node.status.as_ref().and_then(|s| s.conditions.clone()).unwrap_or_default();
    let ready = conditions.iter().find(|c| c.type_ == "Ready");
    let (age, age_secs) = age(&node.metadata);
    let make = |tone, reason: String, detail: String| Problem { kind: ResourceKind::Nodes, namespace: None, name: node.metadata.name.clone().unwrap_or_default(), tone, reason, detail, age: age.clone(), age_secs };
    if ready.is_none_or(|c| c.status != "True") {
        return Some(make(Tone::Bad, "NotReady".into(), ready.and_then(|c| c.message.clone()).unwrap_or_default()));
    }
    // Memory, disk and PID pressure are true when something is wrong.
    if let Some(pressure) = conditions.iter().find(|c| c.type_ != "Ready" && c.status == "True") {
        return Some(make(Tone::Warn, pressure.type_.clone(), pressure.message.clone().unwrap_or_default()));
    }
    if node.spec.as_ref().and_then(|s| s.unschedulable).unwrap_or(false) {
        return Some(make(Tone::Warn, "Cordoned".into(), "No new pods are scheduled here".into()));
    }
    None
}

/// A row of any other kind whose status reads as degraded or broken.
pub fn row(kind: ResourceKind, row: &GenericRow) -> Option<Problem> {
    let (tone, text) = row.status.clone()?;
    if !matches!(tone, Tone::Warn | Tone::Bad) {
        return None;
    }
    Some(Problem { kind, namespace: (row.namespace != "-").then(|| row.namespace.clone()), name: row.name.clone(), tone, reason: text, detail: String::new(), age: row.age.clone(), age_secs: row.age_secs })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod_from(json: serde_json::Value) -> Pod {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn a_crash_loop_says_how_the_container_last_exited() {
        let p = pod_from(serde_json::json!({
            "metadata": {"name": "web", "namespace": "shop"},
            "status": {"phase": "Running", "containerStatuses": [{
                "name": "app", "ready": false, "restartCount": 7, "image": "x", "imageID": "x",
                "state": {"waiting": {"reason": "CrashLoopBackOff", "message": "back-off 5m0s"}},
                "lastState": {"terminated": {"exitCode": 1, "reason": "Error"}}
            }]}
        }));
        let problem = pod(&p).unwrap();
        assert_eq!(problem.reason, "CrashLoopBackOff");
        assert_eq!(problem.tone, Tone::Bad);
        assert_eq!(problem.detail, "app exited with code 1 (Error), restarted 7 times");
    }

    #[test]
    fn a_pending_pod_says_why_it_cannot_be_scheduled() {
        let p = pod_from(serde_json::json!({
            "metadata": {"name": "big", "namespace": "shop"},
            "status": {"phase": "Pending", "conditions": [{"type": "PodScheduled", "status": "False", "message": "0/3 nodes are available: 3 Insufficient memory."}]}
        }));
        assert_eq!(pod(&p).unwrap().detail, "0/3 nodes are available: 3 Insufficient memory.");
    }

    #[test]
    fn healthy_and_finished_pods_are_not_problems() {
        let running = pod_from(serde_json::json!({"metadata": {"name": "a"}, "status": {"phase": "Running"}}));
        let done = pod_from(serde_json::json!({"metadata": {"name": "b"}, "status": {"phase": "Succeeded"}}));
        assert!(pod(&running).is_none() && pod(&done).is_none());
    }

    #[test]
    fn a_deployment_short_of_ready_pods_is_degraded_and_one_with_none_is_broken() {
        let dep = |ready: i32| -> Deployment { serde_json::from_value(serde_json::json!({"metadata": {"name": "web"}, "spec": {"replicas": 3, "selector": {}, "template": {}}, "status": {"readyReplicas": ready}})).unwrap() };
        assert!(deployment(&dep(3)).is_none());
        assert_eq!(deployment(&dep(1)).unwrap().tone, Tone::Warn);
        assert_eq!(deployment(&dep(0)).unwrap().tone, Tone::Bad);
        assert_eq!(deployment(&dep(1)).unwrap().reason, "1/3 ready");
    }

    #[test]
    fn broken_comes_before_degraded() {
        let p = |tone, age_secs| Problem { kind: ResourceKind::Pods, namespace: None, name: String::new(), tone, reason: String::new(), detail: String::new(), age: String::new(), age_secs };
        let mut list = vec![p(Tone::Warn, 1), p(Tone::Bad, 50), p(Tone::Bad, 5)];
        sort(&mut list);
        assert_eq!(list.iter().map(|p| (p.tone, p.age_secs)).collect::<Vec<_>>(), [(Tone::Bad, 5), (Tone::Bad, 50), (Tone::Warn, 1)]);
    }
}
