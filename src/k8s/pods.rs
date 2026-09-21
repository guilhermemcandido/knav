use std::sync::Arc;

use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    Client,
    api::{Api, LogParams},
    runtime::{WatchStreamExt, reflector, watcher},
};
use tokio::{sync::mpsc, task::JoinHandle};

use super::*;

pub struct PodRow {
    pub namespace: String,
    pub name: String,
    pub phase: String,
    pub restarts: i32,
    pub containers: Vec<ContainerInfo>,
    /// "ready/total" containers, e.g. "2/3", standard in both k9s and
    /// Freelens' pod lists.
    pub ready: String,
    pub node: String,
    /// What controls it, the kind of its owner (`ReplicaSet`, `Job`), or `-`.
    pub controlled_by: String,
    pub qos: String,
    /// Pod IP and the images it runs, for the wide view.
    pub ip: String,
    pub images: String,
    pub age: String,
    pub age_secs: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContainerStatusKind {
    Running,
    Waiting,
    Terminated,
    Unknown,
}

#[derive(Clone)]
pub struct ContainerInfo {
    pub name: String,
    pub status: ContainerStatusKind,
    pub reason: Option<String>,
    pub restarts: i32,
}

/// Per-container state from `status.containerStatuses`. The pod `phase` stays
/// `Running` for a crash loop; only the container's waiting reason shows it.
pub fn containers_for(pod: &Pod) -> Vec<ContainerInfo> {
    let statuses = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.clone())
        .unwrap_or_default();

    statuses
        .into_iter()
        .map(|cs| {
            let (status, reason) = match cs.state {
                Some(s) if s.running.is_some() => (ContainerStatusKind::Running, None),
                Some(s) if s.waiting.is_some() => {
                    (ContainerStatusKind::Waiting, s.waiting.and_then(|w| w.reason))
                }
                Some(s) if s.terminated.is_some() => {
                    (ContainerStatusKind::Terminated, s.terminated.and_then(|t| t.reason))
                }
                _ => (ContainerStatusKind::Unknown, None),
            };
            ContainerInfo { name: cs.name, status, reason, restarts: cs.restart_count }
        })
        .collect()
}

/// The STATUS kubectl and k9s show: the phase refined by container state
/// (`CrashLoopBackOff`, `Init:0/1`, `Completed`, `Terminating`, ...).
pub fn pod_status(pod: &Pod) -> String {
    let Some(status) = pod.status.as_ref() else { return "Unknown".into() };
    let mut reason = status.reason.clone().or_else(|| status.phase.clone()).unwrap_or_else(|| "Unknown".into());
    let mut initializing = false;
    let init_statuses = status.init_container_statuses.as_deref().unwrap_or_default();
    for (i, container) in init_statuses.iter().enumerate() {
        let state = container.state.as_ref();
        if let Some(done) = state.and_then(|s| s.terminated.as_ref()) {
            if done.exit_code == 0 {
                continue;
            }
            initializing = true;
            reason = format!("Init:{}", terminated_reason(done));
        } else if let Some(waiting) = state.and_then(|s| s.waiting.as_ref())
            && waiting.reason.as_deref().is_some_and(|r| !r.is_empty() && r != "PodInitializing")
        {
            initializing = true;
            reason = format!("Init:{}", waiting.reason.clone().unwrap_or_default());
        } else {
            initializing = true;
            reason = format!("Init:{i}/{}", init_statuses.len());
        }
        break;
    }
    if !initializing {
        let mut has_running = false;
        for container in status.container_statuses.as_deref().unwrap_or_default().iter().rev() {
            let state = container.state.as_ref();
            if let Some(waiting) = state.and_then(|s| s.waiting.as_ref()).filter(|w| w.reason.as_deref().is_some_and(|r| !r.is_empty())) {
                reason = waiting.reason.clone().unwrap_or_default();
            } else if let Some(done) = state.and_then(|s| s.terminated.as_ref()) {
                reason = terminated_reason(done);
            } else if container.ready && state.is_some_and(|s| s.running.is_some()) {
                has_running = true;
            }
        }
        if reason == "Completed" && has_running {
            reason = "Running".into();
        }
    }
    if pod.metadata.deletion_timestamp.is_some() {
        reason = if status.reason.as_deref() == Some("NodeLost") { "Unknown".into() } else { "Terminating".into() };
    }
    reason
}

fn terminated_reason(done: &k8s_openapi::api::core::v1::ContainerStateTerminated) -> String {
    match done.reason.as_deref().filter(|r| !r.is_empty()) {
        Some(reason) => reason.to_string(),
        None => match done.signal.filter(|s| *s != 0) {
            Some(signal) => format!("Signal:{signal}"),
            None => format!("ExitCode:{}", done.exit_code),
        },
    }
}

/// Whether a pod is worth a look: not running healthily, and not simply finished.
pub fn pod_is_fault(pod: &Pod) -> bool {
    use crate::k8s::describe::Tone;
    matches!(status_tone(&pod_status(pod)), Tone::Warn | Tone::Bad)
}

/// How a pod STATUS should be coloured: healthy plain, finished grey,
/// in-progress orange, broken red.
pub fn status_tone(status: &str) -> crate::k8s::describe::Tone {
    use crate::k8s::describe::Tone;
    match status {
        "Running" => Tone::Plain,
        "Completed" | "Succeeded" => Tone::Muted,
        "Pending" | "ContainerCreating" | "PodInitializing" | "Terminating" | "NotReady" | "Unknown" => Tone::Warn,
        s if s.starts_with("Init:") && s[5..].contains('/') => Tone::Warn,
        _ => Tone::Bad,
    }
}

pub fn row_for(pod: &Pod) -> PodRow {
    let namespace = pod.metadata.namespace.clone().unwrap_or_default();
    let name = pod.metadata.name.clone().unwrap_or_default();
    let status = pod.status.clone().unwrap_or_default();
    let phase = pod_status(pod);
    let container_statuses = status.container_statuses.unwrap_or_default();
    let restarts = container_statuses.iter().map(|c| c.restart_count).sum();
    let ready_count = container_statuses.iter().filter(|c| c.ready).count();
    // Like kubectl: the total is the containers the spec asks for, so a
    // pod that has not started yet reads 0/1, not 0/0.
    let total = pod.spec.as_ref().map(|s| s.containers.len()).unwrap_or(container_statuses.len());
    let ready = format!("{ready_count}/{total}");
    let containers = containers_for(pod);
    let node = pod.spec.as_ref().and_then(|s| s.node_name.clone()).unwrap_or_else(|| "-".into());
    let age = pod
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| humanize_age(t.0))
        .unwrap_or_else(|| "-".into());

    let age_secs = age_seconds(pod.metadata.creation_timestamp.as_ref());
    let controlled_by = pod.metadata.owner_references.as_ref().and_then(|o| o.first()).map(|o| o.kind.clone()).unwrap_or_else(|| "-".into());
    let qos = status.qos_class.unwrap_or_else(|| "-".into());
    let ip = status.pod_ip.clone().unwrap_or_else(|| "-".into());
    let images = pod.spec.as_ref().map(|s| s.containers.iter().filter_map(|c| c.image.clone()).collect::<Vec<_>>().join(",")).filter(|i| !i.is_empty()).unwrap_or_else(|| "-".into());
    PodRow { namespace, name, phase, restarts, containers, ready, node, controlled_by, qos, ip, images, age, age_secs }
}

/// Watches every Pod in the cluster into an in-memory store, reconnecting with
/// backoff. Call `.snapshot()` for the current list.
pub fn watch_pods(client: Client) -> (reflector::Store<Pod>, JoinHandle<()>) {
    let api: Api<Pod> = Api::all(client);
    let (reader, writer) = reflector::store();

    let stream = watcher(api, watcher::Config::default())
        .default_backoff()
        .reflect(writer)
        .applied_objects();

    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {
            // Nothing to do per-event, `reader.snapshot()` already
            // reflects it, since `reflect(writer)` updates the store.
        }
    });

    (reader, handle)
}

/// A stable, sorted snapshot of every pod currently in the store. Sorted
/// so a selected row index stays pointing at the same pod across ticks
/// (the store itself has no defined order).
pub fn snapshot(store: &reflector::Store<Pod>) -> Vec<Arc<Pod>> {
    let mut pods = store.state();
    pods.sort_by(|a, b| {
        let key = |p: &Arc<Pod>| {
            (
                p.metadata.namespace.clone().unwrap_or_default(),
                p.metadata.name.clone().unwrap_or_default(),
            )
        };
        key(a).cmp(&key(b))
    });
    pods
}

/// Streams one container's log over an unbounded channel. The caller must abort
/// the returned handle when done. `previous` reads the last terminated run.
pub fn stream_logs(
    client: Client,
    namespace: String,
    pod: String,
    container: String,
    previous: bool,
) -> (mpsc::UnboundedReceiver<String>, JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel();

    let handle = tokio::spawn(async move {
        let api: Api<Pod> = Api::namespaced(client, &namespace);
        let lp = LogParams {
            container: Some(container),
            follow: !previous,
            previous,
            // Timestamps come from the API server, not from when knav read the line.
            timestamps: true,
            ..Default::default()
        };

        let mut lines = match api.log_stream(&pod, &lp).await {
            Ok(stream) => stream.lines(),
            Err(e) => {
                let _ = tx.send(format!("[failed to start log stream: {e}]"));
                return;
            }
        };

        loop {
            match lines.next().await {
                Some(Ok(line)) => {
                    if tx.send(line).is_err() {
                        break; // receiver dropped, view was closed
                    }
                }
                Some(Err(e)) => {
                    let _ = tx.send(format!("[log stream error: {e}]"));
                    break;
                }
                None => break, // stream ended (container exited)
            }
        }
    });

    (rx, handle)
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use crate::k8s::describe::Tone;

    fn pod(json: serde_json::Value) -> Pod {
        serde_json::from_value(json).unwrap()
    }

    fn with_container(state: serde_json::Value, ready: bool) -> Pod {
        pod(serde_json::json!({
            "metadata": {"name": "p"},
            "status": {"phase": "Running", "containerStatuses": [{"name": "c", "image": "i", "imageID": "", "ready": ready, "restartCount": 0, "state": state}]}
        }))
    }

    #[test]
    fn a_healthy_pod_is_running() {
        assert_eq!(pod_status(&with_container(serde_json::json!({"running": {}}), true)), "Running");
    }

    #[test]
    fn a_crash_looping_pod_says_so_though_its_phase_is_running() {
        let status = pod_status(&with_container(serde_json::json!({"waiting": {"reason": "CrashLoopBackOff"}}), false));
        assert_eq!(status, "CrashLoopBackOff");
        assert_eq!(status_tone(&status), Tone::Bad);
    }

    #[test]
    fn a_finished_container_shows_its_reason_and_a_bare_exit_code_otherwise() {
        assert_eq!(pod_status(&with_container(serde_json::json!({"terminated": {"exitCode": 0, "reason": "Completed"}}), false)), "Completed");
        assert_eq!(pod_status(&with_container(serde_json::json!({"terminated": {"exitCode": 3}}), false)), "ExitCode:3");
        assert_eq!(pod_status(&with_container(serde_json::json!({"terminated": {"exitCode": 137, "signal": 9}}), false)), "Signal:9");
    }

    #[test]
    fn init_containers_show_their_progress() {
        let waiting = pod(serde_json::json!({
            "metadata": {"name": "p"},
            "status": {"phase": "Pending", "initContainerStatuses": [
                {"name": "a", "image": "i", "imageID": "", "ready": true, "restartCount": 0, "state": {"terminated": {"exitCode": 0}}},
                {"name": "b", "image": "i", "imageID": "", "ready": false, "restartCount": 0, "state": {"waiting": {"reason": "PodInitializing"}}}]}
        }));
        assert_eq!(pod_status(&waiting), "Init:1/2");
        assert_eq!(status_tone("Init:1/2"), Tone::Warn);
        let failing = pod(serde_json::json!({
            "metadata": {"name": "p"},
            "status": {"phase": "Pending", "initContainerStatuses": [
                {"name": "a", "image": "i", "imageID": "", "ready": false, "restartCount": 0, "state": {"terminated": {"exitCode": 1, "reason": "Error"}}}]}
        }));
        assert_eq!(pod_status(&failing), "Init:Error");
        assert_eq!(status_tone("Init:Error"), Tone::Bad);
    }

    #[test]
    fn a_deleted_pod_is_terminating() {
        let mut p = with_container(serde_json::json!({"running": {}}), true);
        p.metadata.deletion_timestamp = Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(k8s_openapi::jiff::Timestamp::now()));
        assert_eq!(pod_status(&p), "Terminating");
    }

    #[test]
    fn tones_follow_health() {
        assert_eq!(status_tone("Running"), Tone::Plain);
        assert_eq!(status_tone("Completed"), Tone::Muted);
        assert_eq!(status_tone("Pending"), Tone::Warn);
        assert_eq!(status_tone("ImagePullBackOff"), Tone::Bad);
        assert_eq!(status_tone("Error"), Tone::Bad);
    }
}
