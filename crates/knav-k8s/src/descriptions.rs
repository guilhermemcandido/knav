//! One-line descriptions of custom resource kinds, read from their CRDs in the
//! background. The `:` menu shows what is there and never waits for the rest.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Client, api::Api};

/// How many CRDs are fetched at once, so a menu full of kinds can't flood the server.
static FETCHES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(6);

/// The longest description kept, in characters; the menu cuts it to what fits.
const MAX_CHARS: usize = 160;

#[derive(Default)]
struct State {
    /// `plural.group` to its description; `None` when the CRD has none or the fetch failed.
    done: HashMap<String, Option<String>>,
    pending: HashSet<String>,
}

#[derive(Clone, Default)]
pub struct Descriptions {
    state: Arc<Mutex<State>>,
}

impl Descriptions {
    /// The kind's description, when it has been fetched and there is one.
    pub fn get(&self, group: &str, plural: &str) -> Option<String> {
        self.state.lock().ok()?.done.get(&format!("{plural}.{group}"))?.clone()
    }

    /// Starts fetching the kinds (`(group, plural)`) not fetched or being fetched yet.
    pub fn request(&self, client: &Client, wanted: &[(String, String)]) {
        let Ok(mut state) = self.state.lock() else { return };
        for (group, plural) in wanted {
            let name = format!("{plural}.{group}");
            if state.done.contains_key(&name) || !state.pending.insert(name.clone()) {
                continue;
            }
            let (client, shared) = (client.clone(), Arc::clone(&self.state));
            tokio::spawn(async move {
                let found = {
                    let _slot = FETCHES.acquire().await;
                    let api: Api<CustomResourceDefinition> = Api::all(client);
                    api.get(&name).await.ok().and_then(|crd| describe(&crd))
                };
                if let Ok(mut state) = shared.lock() {
                    state.pending.remove(&name);
                    state.done.insert(name, found);
                }
                crate::note_change();
            });
        }
    }
}

/// The first version's schema description that says anything.
fn describe(crd: &CustomResourceDefinition) -> Option<String> {
    crd.spec.versions.iter().filter_map(|v| v.schema.as_ref()?.open_api_v3_schema.as_ref()?.description.as_deref()).find_map(clean)
}

/// One line: whitespace collapsed, cut after the first sentence and at a sane length.
fn clean(text: &str) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let first = match line.find(". ") {
        Some(end) => &line[..=end],
        None => line.as_str(),
    };
    let first = first.trim();
    if first.is_empty() {
        return None;
    }
    let mut out: String = first.chars().take(MAX_CHARS).collect();
    if first.chars().count() > MAX_CHARS {
        out.push('…');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crd(descriptions: &[Option<&str>]) -> CustomResourceDefinition {
        let versions: Vec<_> = descriptions
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let schema = d.map(|d| serde_json::json!({"openAPIV3Schema": {"type": "object", "description": d}}));
                let mut version = serde_json::json!({"name": format!("v{i}"), "served": true, "storage": i == 0});
                if let Some(schema) = schema {
                    version["schema"] = schema;
                }
                version
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "metadata": {"name": "agents.kagent.dev"},
            "spec": {"group": "kagent.dev", "names": {"kind": "Agent", "plural": "agents"}, "scope": "Namespaced", "versions": versions}
        }))
        .unwrap()
    }

    #[test]
    fn the_description_is_one_line_and_one_sentence() {
        let text = "An Agent declares\n  an LLM-backed agent.  It also\nhas more detail.";
        assert_eq!(clean(text).as_deref(), Some("An Agent declares an LLM-backed agent."));
    }

    #[test]
    fn a_long_description_is_cut_and_a_blank_one_is_none() {
        let long = "word ".repeat(100);
        let cut = clean(&long).unwrap();
        assert!(cut.chars().count() <= MAX_CHARS + 1 && cut.ends_with('…'));
        assert_eq!(clean("  \n "), None);
    }

    #[test]
    fn a_crd_reads_the_first_version_that_has_one() {
        assert_eq!(describe(&crd(&[None, Some("Second version.")])).as_deref(), Some("Second version."));
        assert_eq!(describe(&crd(&[None])), None);
    }
}
