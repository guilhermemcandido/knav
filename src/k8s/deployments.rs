use std::sync::Arc;

use futures::StreamExt;
use k8s_openapi::api::apps::v1::Deployment;
use kube::{
    Client,
    api::Api,
    runtime::{WatchStreamExt, reflector, watcher},
};
use tokio::task::JoinHandle;

use super::*;

pub struct DeploymentRow {
    pub namespace: String,
    pub name: String,
    /// "ready/desired" replicas, e.g. "2/3" — kubectl/k9s convention.
    pub ready: String,
    pub up_to_date: i32,
    pub available: i32,
    /// The images it runs, for the wide view.
    pub images: String,
    pub age: String,
    pub age_secs: i64,
}

/// Whether a `have/want` ready count is short of what is wanted.
pub fn ready_is_short(ready: &str) -> bool {
    let mut parts = ready.split('/').filter_map(|p| p.parse::<i64>().ok());
    matches!((parts.next(), parts.next()), (Some(have), Some(want)) if have < want)
}

pub fn row_for_deployment(dep: &Deployment) -> DeploymentRow {
    let namespace = dep.metadata.namespace.clone().unwrap_or_default();
    let name = dep.metadata.name.clone().unwrap_or_default();
    let status = dep.status.clone().unwrap_or_default();
    let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
    let ready = format!("{}/{desired}", status.ready_replicas.unwrap_or(0));
    let up_to_date = status.updated_replicas.unwrap_or(0);
    let available = status.available_replicas.unwrap_or(0);
    let age = dep
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| humanize_age(t.0))
        .unwrap_or_else(|| "-".into());

    let age_secs = age_seconds(dep.metadata.creation_timestamp.as_ref());
    let images = dep
        .spec
        .as_ref()
        .and_then(|s| s.template.spec.as_ref())
        .map(|p| p.containers.iter().filter_map(|c| c.image.clone()).collect::<Vec<_>>().join(","))
        .filter(|i| !i.is_empty())
        .unwrap_or_else(|| "-".into());
    DeploymentRow { namespace, name, ready, up_to_date, available, images, age, age_secs }
}

/// Same live-watch pattern as `watch_pods`, for Deployments — see there
/// for why a reflector instead of polling.
pub fn watch_deployments(client: Client) -> (reflector::Store<Deployment>, JoinHandle<()>) {
    let api: Api<Deployment> = Api::all(client);
    let (reader, writer) = reflector::store();

    let stream = watcher(api, watcher::Config::default())
        .default_backoff()
        .reflect(writer)
        .applied_objects();

    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {}
    });

    (reader, handle)
}

/// Sorted snapshot, same reasoning as `snapshot` for Pods — kept as a
/// separate small function rather than a generic one across resource
/// kinds; with just two kinds so far, a shared-trait abstraction would be
/// more machinery than the ~10 lines it'd save.
pub fn snapshot_deployments(store: &reflector::Store<Deployment>) -> Vec<Arc<Deployment>> {
    let mut deployments = store.state();
    deployments.sort_by(|a, b| {
        let key = |d: &Arc<Deployment>| {
            (
                d.metadata.namespace.clone().unwrap_or_default(),
                d.metadata.name.clone().unwrap_or_default(),
            )
        };
        key(a).cmp(&key(b))
    });
    deployments
}

#[cfg(test)]
mod fault_tests {
    use super::*;

    #[test]
    fn short_of_replicas_is_a_fault() {
        assert!(ready_is_short("0/1"));
        assert!(ready_is_short("2/3"));
        assert!(!ready_is_short("3/3"));
        assert!(!ready_is_short("0/0"));
        assert!(!ready_is_short("junk"));
    }
}
