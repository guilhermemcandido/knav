//! Every resource the API server offers, from discovery, and the API Resources list.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use kube::{
    Client,
    api::ApiResource,
    discovery::{Discovery, Scope, verbs},
};

use super::*;
use crate::describe::{Col, Tone};

/// One listable resource type, from discovery.
#[derive(Clone, Debug)]
pub struct ApiInfo {
    pub group: &'static str,
    pub version: String,
    pub kind: &'static str,
    pub plural: &'static str,
    pub namespaced: bool,
    pub verbs: Vec<String>,
}

impl ApiInfo {
    pub fn resource(&self) -> ApiResource {
        ApiResource {
            group: self.group.to_string(),
            version: self.version.clone(),
            api_version: if self.group.is_empty() { self.version.clone() } else { format!("{}/{}", self.group, self.version) },
            kind: self.kind.to_string(),
            plural: self.plural.to_string(),
        }
    }

    /// `pods`, or `deployments.apps` for a grouped one.
    pub fn qualified_name(&self) -> String {
        if self.group.is_empty() { self.plural.to_string() } else { format!("{}.{}", self.plural, self.group) }
    }
}

impl From<&CrdInfo> for ApiInfo {
    fn from(crd: &CrdInfo) -> Self {
        ApiInfo { group: crd.group, version: crd.version.clone(), kind: crd.kind, plural: leak(&crd.plural), namespaced: crd.namespaced, verbs: Vec::new() }
    }
}

/// Leaks `text` once and returns it as `&'static str`, which headers and labels need.
/// The set is bounded: the cluster's resource names and column titles.
pub fn leak(text: &str) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut map = INTERNED.get_or_init(Default::default).lock().expect("interner lock");
    if let Some(found) = map.get(text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
    map.insert(text.to_string(), leaked);
    leaked
}

/// The names (`plural.group`) of every installed CRD. Lists metadata only, in pages,
/// since full CRDs carry their schemas and can total hundreds of megabytes.
async fn crd_names(client: &Client) -> std::collections::HashSet<String> {
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
    use kube::{ResourceExt, api::{Api, ListParams}, core::PartialObjectMeta};
    let api: Api<PartialObjectMeta<CustomResourceDefinition>> = Api::all(client.clone());
    let mut names = std::collections::HashSet::new();
    let mut token: Option<String> = None;
    loop {
        let mut params = ListParams::default().limit(500);
        params.continue_token = token.take();
        let Ok(page) = api.list_metadata(&params).await else { break };
        names.extend(page.items.iter().map(|crd| crd.name_any()));
        match page.metadata.continue_.filter(|t| !t.is_empty()) {
            Some(next) => token = Some(next),
            None => break,
        }
    }
    names
}

/// Discovery through the aggregated API (two requests) when the server has it,
/// else one round trip per group.
async fn run_discovery(client: &Client) -> Option<Discovery> {
    match Discovery::new(client.clone()).run_aggregated().await {
        Ok(found) => Some(found),
        Err(_) => Discovery::new(client.clone()).run().await.ok(),
    }
}

/// Every listable resource at its recommended version, and the custom resources among
/// them at their preferred one. Both sorted by group then kind; empty if discovery fails.
pub async fn discover(client: &Client) -> (Vec<ApiInfo>, Vec<CrdInfo>) {
    let (found, names) = tokio::join!(run_discovery(client), crd_names(client));
    let Some(discovery) = found else { return (Vec::new(), Vec::new()) };
    let mut apis: Vec<ApiInfo> = discovery
        .groups()
        .flat_map(|group| group.recommended_resources())
        .filter(|(resource, caps)| !resource.plural.contains('/') && caps.supports_operation(verbs::LIST))
        .map(|(resource, caps)| ApiInfo {
            group: leak(&resource.group),
            version: resource.version.clone(),
            kind: leak(&resource.kind),
            plural: leak(&resource.plural),
            namespaced: caps.scope == Scope::Namespaced,
            verbs: caps.operations.clone(),
        })
        .collect();
    apis.sort_by(|a, b| (a.group, a.kind).cmp(&(b.group, b.kind)));
    apis.dedup_by(|a, b| a.group == b.group && a.plural == b.plural);

    let mut crds = Vec::new();
    for group in discovery.groups().filter(|g| !g.name().is_empty()) {
        // A CRD served in several versions is listed once, at the preferred one when it has it.
        let preferred = group.preferred_version_or_latest();
        let mut versions: Vec<&str> = group.versions().collect();
        versions.sort_by_key(|v| *v != preferred);
        let mut seen = std::collections::HashSet::new();
        for version in versions {
            for (resource, caps) in group.versioned_resources(version) {
                if resource.plural.contains('/') || !names.contains(&format!("{}.{}", resource.plural, resource.group)) || !seen.insert(resource.plural.clone()) {
                    continue;
                }
                crds.push(CrdInfo { group: leak(&resource.group), kind: leak(&resource.kind), plural: resource.plural.clone(), version: resource.version.clone(), namespaced: caps.scope == Scope::Namespaced });
            }
        }
    }
    crds.sort_by(|a, b| (a.group, a.kind).cmp(&(b.group, b.kind)));
    (apis, crds)
}

/// The list of every resource type (`:api`).
pub struct ApiList {
    pub apis: Vec<ApiInfo>,
    /// How many objects each type has, filled in by the background counter.
    pub counts: InstanceCounts,
}

impl CatalogKind for ApiList {
    fn count(&self) -> usize {
        self.apis.len()
    }

    fn rows(&self) -> Vec<Arc<GenericRow>> {
        self.apis
            .iter()
            .map(|api| {
                let col = |header, text: String| Col { header, text, tone: Tone::Plain, sort: None };
                let count = self.counts.get(api.group, api.plural);
                GenericRow {
                    namespace: "-".into(),
                    name: api.plural.to_string(),
                    age: "-".into(),
                    age_secs: 0,
                    extras: vec![
                        col("GROUP", if api.group.is_empty() { "core".into() } else { api.group.to_string() }),
                        col("VERSION", api.version.clone()),
                        col("KIND", api.kind.to_string()),
                        Col { header: "COUNT", text: count.text(), tone: if count == Count::Known(0) { Tone::Muted } else { Tone::Plain }, sort: Some(count.sort_key()) },
                        col("NAMESPACED", api.namespaced.to_string()),
                        col("VERBS", api.verbs.join(",")),
                    ],
                    status: None,
                    uid: api.qualified_name(),
                    owners: Vec::new(),
                    labels: String::new(),
                }
            })
            .map(Arc::new)
            .collect()
    }

    fn spec_at(&self, index: usize) -> Option<serde_yaml::Value> {
        let api = self.apis.get(index)?;
        serde_yaml::to_value(serde_json::json!({
            "apiVersion": "meta.k8s.io/v1",
            "kind": "APIResource",
            "metadata": {"name": api.plural},
            "spec": {"group": api.group, "version": api.version, "kind": api.kind, "plural": api.plural, "namespaced": api.namespaced, "verbs": api.verbs},
        })).ok()
    }

    fn headers(&self) -> Vec<&'static str> {
        vec!["GROUP", "VERSION", "KIND", "COUNT", "NAMESPACED", "VERBS"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_of_apis_names_each_by_group() {
        let list = ApiList { counts: InstanceCounts::default(), apis: vec![ApiInfo { group: "apps", version: "v1".into(), kind: "Deployment", plural: "deployments", namespaced: true, verbs: vec!["list".into()] }] };
        let rows = list.rows();
        assert_eq!(rows[0].uid, "deployments.apps");
        assert_eq!(rows[0].extras[0].text, "apps");
        assert_eq!(rows[0].extras.len(), list.headers().len());
    }
}

#[cfg(test)]
mod live {
    /// Against the current kubeconfig context: `cargo test live_discovery -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_discovery() {
        let client = kube::Client::try_default().await.unwrap();
        let started = std::time::Instant::now();
        let (apis, crds) = super::discover(&client).await;
        println!("{} types, {} custom resources in {:?}", apis.len(), crds.len(), started.elapsed());
        for crd in crds.iter().take(5) {
            println!("  {}/{} {}", crd.group, crd.version, crd.kind);
        }
    }
}
