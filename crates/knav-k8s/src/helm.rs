//! Helm releases, read from the Secrets Helm writes (one per revision) and decoded
//! like `helm list` does: base64, gzip, JSON. Works on any cluster, not just k3s.

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use base64::Engine;
use k8s_openapi::api::core::v1::Secret;
use kube::{Client, ResourceExt, runtime::reflector};
use serde::Deserialize;

use super::describe::{Col, Tone};
use super::*;

const FIELD_SELECTOR: &str = "type=helm.sh/release.v1";

/// The parts of a release payload this view shows.
#[derive(Deserialize, Default)]
struct ReleasePayload {
    #[serde(default)]
    chart: Chart,
}

#[derive(Deserialize, Default)]
struct Chart {
    #[serde(default)]
    metadata: ChartMetadata,
}

#[derive(Deserialize, Default)]
struct ChartMetadata {
    name: Option<String>,
    version: Option<String>,
    #[serde(rename = "appVersion")]
    app_version: Option<String>,
}

fn decode(secret: &Secret) -> Option<ReleasePayload> {
    let raw = &secret.data.as_ref()?.get("release")?.0;
    let gz = base64::engine::general_purpose::STANDARD.decode(raw).ok()?;
    let mut json = Vec::new();
    flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut json).ok()?;
    serde_json::from_slice(&json).ok()
}

fn release_tone(status: &str) -> Tone {
    match status {
        "deployed" => Tone::Good,
        "failed" => Tone::Bad,
        "superseded" | "uninstalled" => Tone::Muted,
        _ => Tone::Warn, // pending-install, pending-upgrade, pending-rollback, uninstalling, unknown
    }
}

/// One release's row, from its current (highest-revision) Secret.
fn release_row(secret: &Secret) -> GenericRow {
    let labels = secret.metadata.labels.as_ref();
    let name = labels.and_then(|l| l.get("name")).cloned().unwrap_or_else(|| secret.name_any());
    let namespace = secret.metadata.namespace.clone().unwrap_or_else(|| "-".into());
    let revision = labels.and_then(|l| l.get("version")).cloned().unwrap_or_else(|| "-".into());
    let status = labels.and_then(|l| l.get("status")).cloned().unwrap_or_else(|| "unknown".into());
    let tone = release_tone(&status);
    let payload = decode(secret).unwrap_or_default();
    let chart = match (&payload.chart.metadata.name, &payload.chart.metadata.version) {
        (Some(n), Some(v)) => format!("{n}-{v}"),
        (Some(n), None) => n.clone(),
        _ => "-".into(),
    };
    let app_version = payload.chart.metadata.app_version.clone().unwrap_or_else(|| "-".into());
    let age = secret.metadata.creation_timestamp.as_ref().map(|t| humanize_age(t.0)).unwrap_or_else(|| "-".into());
    let age_secs = age_seconds(secret.metadata.creation_timestamp.as_ref());
    let uid = secret.metadata.uid.clone().unwrap_or_default();
    let cols = vec![
        Col { header: "STATUS", text: status.clone(), tone, sort: None },
        Col { header: "CHART", text: chart, tone: Tone::Plain, sort: None },
        Col { header: "APP VERSION", text: app_version, tone: Tone::Plain, sort: None },
        Col { header: "REVISION", text: revision.clone(), tone: Tone::Plain, sort: revision.parse().ok() },
    ];
    GenericRow { namespace, name, age, age_secs, extras: cols, status: Some((tone, status)), uid, owners: Vec::new(), labels: label_text(secret.metadata.labels.as_ref()) }
}

/// One Secret per release: the highest revision, like `helm list`.
/// Sorted so `rows()` and `spec_at()` agree between calls.
fn current_revisions(secrets: &[Arc<Secret>]) -> Vec<Arc<Secret>> {
    let mut latest: HashMap<(String, String), Arc<Secret>> = HashMap::new();
    for secret in secrets {
        let Some(name) = secret.metadata.labels.as_ref().and_then(|l| l.get("name")) else { continue };
        let namespace = secret.metadata.namespace.clone().unwrap_or_default();
        let revision: u32 = secret.metadata.labels.as_ref().and_then(|l| l.get("version")).and_then(|v| v.parse().ok()).unwrap_or(0);
        let key = (namespace, name.clone());
        let current_rev = |s: &Secret| -> u32 { s.metadata.labels.as_ref().and_then(|l| l.get("version")).and_then(|v| v.parse().ok()).unwrap_or(0) };
        match latest.get(&key) {
            Some(existing) if current_rev(existing) >= revision => {}
            _ => {
                latest.insert(key, Arc::clone(secret));
            }
        }
    }
    let mut releases: Vec<Arc<Secret>> = latest.into_values().collect();
    releases.sort_by(|a, b| (a.metadata.namespace.as_deref(), a.metadata.labels.as_ref().and_then(|l| l.get("name")).map(String::as_str)).cmp(&(b.metadata.namespace.as_deref(), b.metadata.labels.as_ref().and_then(|l| l.get("name")).map(String::as_str))));
    releases
}

pub struct HelmStore {
    store: reflector::Store<Secret>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for HelmStore {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl HelmStore {
    pub fn start(client: Client) -> Self {
        let (store, handle) = watch_store_selected::<Secret>(client, FIELD_SELECTOR);
        HelmStore { store, handle }
    }

    fn releases(&self) -> Vec<Arc<Secret>> {
        current_revisions(&self.store.state())
    }
}

impl CatalogKind for HelmStore {
    fn count(&self) -> usize {
        self.releases().len()
    }

    fn rows(&self) -> Vec<Arc<GenericRow>> {
        self.releases().iter().map(|s| Arc::new(release_row(s))).collect()
    }

    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value> {
        self.releases().get(index).map(|s| manifest_value(s.as_ref()))
    }

    fn headers(&self) -> Vec<&'static str> {
        vec!["STATUS", "CHART", "APP VERSION", "REVISION"]
    }

    fn manifests(&self, namespace: Option<&str>) -> Vec<serde_yaml::Value> {
        self.releases().iter().filter(|s| namespace.is_none() || s.metadata.namespace.as_deref() == namespace).map(|s| manifest_value(s.as_ref())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    fn secret(namespace: &str, name: &str, revision: &str, status: &str) -> Arc<Secret> {
        let mut labels = BTreeMap::new();
        labels.insert("name".to_string(), name.to_string());
        labels.insert("version".to_string(), revision.to_string());
        labels.insert("status".to_string(), status.to_string());
        labels.insert("owner".to_string(), "helm".to_string());
        Arc::new(Secret { metadata: ObjectMeta { namespace: Some(namespace.to_string()), name: Some(format!("sh.helm.release.v1.{name}.v{revision}")), labels: Some(labels), ..Default::default() }, ..Default::default() })
    }

    #[test]
    fn only_the_highest_revision_per_release_survives() {
        let secrets = vec![secret("default", "api", "1", "superseded"), secret("default", "api", "3", "deployed"), secret("default", "api", "2", "superseded")];
        let current = current_revisions(&secrets);
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].metadata.labels.as_ref().unwrap().get("version").unwrap(), "3");
    }

    #[test]
    fn different_releases_and_namespaces_dont_collide() {
        let secrets = vec![secret("default", "api", "1", "deployed"), secret("default", "worker", "1", "deployed"), secret("staging", "api", "1", "deployed")];
        assert_eq!(current_revisions(&secrets).len(), 3);
    }

    #[test]
    fn status_decides_the_tone() {
        assert_eq!(release_tone("deployed"), Tone::Good);
        assert_eq!(release_tone("failed"), Tone::Bad);
        assert_eq!(release_tone("superseded"), Tone::Muted);
        assert_eq!(release_tone("pending-upgrade"), Tone::Warn);
    }
}
