use k8s_openapi::api::apps::v1::Deployment;

use super::*;

#[derive(Clone)]
pub struct DeploymentRow {
    pub namespace: String,
    pub name: String,
    /// Ready over desired replicas, like "2/3".
    pub ready: String,
    pub up_to_date: i32,
    pub available: i32,
    pub images: String,
    pub selector: String,
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
    let selector = dep
        .spec
        .as_ref()
        .and_then(|s| s.selector.match_labels.as_ref())
        .map(|labels| labels.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "-".into());
    DeploymentRow { namespace, name, ready, up_to_date, available, images, selector, age, age_secs }
}

impl AgeRow for DeploymentRow {
    fn age_secs(&self) -> i64 {
        self.age_secs
    }
    fn set_age(&mut self, age: String, secs: i64) {
        (self.age, self.age_secs) = (age, secs);
    }
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
