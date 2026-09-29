//! What the kubeconfig user may do, from SelfSubjectAccessReviews. It is only shown in
//! the header; the API server still has the final say on every request.

use k8s_openapi::api::authorization::v1::{ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec};
use kube::{Api, Client, api::PostParams};

#[derive(Clone, Debug, PartialEq)]
pub enum Access {
    /// Anything, anywhere.
    Admin,
    /// Can change workloads, cluster-wide or only in `Some(namespace)`.
    ReadWrite(Option<String>),
    ReadOnly(Option<String>),
    /// Can't even list pods in its default namespace.
    Limited,
    /// The checks failed or timed out.
    Unknown,
}

impl Access {
    pub fn label(&self) -> String {
        let scoped = |name: &str, ns: &Option<String>| match ns {
            Some(ns) => format!("{name} ({ns})"),
            None => name.to_string(),
        };
        match self {
            Access::Admin => "admin".into(),
            Access::ReadWrite(ns) => scoped("read-write", ns),
            Access::ReadOnly(ns) => scoped("read-only", ns),
            Access::Limited => "limited".into(),
            Access::Unknown => "unknown".into(),
        }
    }
}

/// Runs every check at once, so it costs one round trip.
pub async fn check_access(client: &Client) -> Access {
    let namespace = client.default_namespace().to_string();
    let (admin, write_all, write_ns, read_all, read_ns) = futures::join!(
        can(client, "*", "*", "*", None),
        can(client, "delete", "", "pods", None),
        can(client, "delete", "", "pods", Some(&namespace)),
        can(client, "list", "", "pods", None),
        can(client, "list", "", "pods", Some(&namespace)),
    );
    let Ok(admin) = admin else { return Access::Unknown };
    classify(admin, write_all.unwrap_or(false), write_ns.unwrap_or(false), read_all.unwrap_or(false), read_ns.unwrap_or(false), namespace)
}

fn classify(admin: bool, write_all: bool, write_ns: bool, read_all: bool, read_ns: bool, namespace: String) -> Access {
    match () {
        _ if admin => Access::Admin,
        _ if write_all => Access::ReadWrite(None),
        _ if write_ns => Access::ReadWrite(Some(namespace)),
        _ if read_all => Access::ReadOnly(None),
        _ if read_ns => Access::ReadOnly(Some(namespace)),
        _ => Access::Limited,
    }
}

async fn can(client: &Client, verb: &str, group: &str, resource: &str, namespace: Option<&str>) -> kube::Result<bool> {
    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                verb: Some(verb.into()),
                group: Some(group.into()),
                resource: Some(resource.into()),
                namespace: namespace.map(str::to_string),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let created = Api::<SelfSubjectAccessReview>::all(client.clone()).create(&PostParams::default(), &review).await?;
    Ok(created.status.is_some_and(|s| s.allowed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_widest_access_wins() {
        let ns = || "team".to_string();
        assert_eq!(classify(true, false, false, false, false, ns()), Access::Admin);
        assert_eq!(classify(false, true, true, true, true, ns()), Access::ReadWrite(None));
        assert_eq!(classify(false, false, true, true, true, ns()), Access::ReadWrite(Some(ns())));
        assert_eq!(classify(false, false, false, false, true, ns()), Access::ReadOnly(Some(ns())));
        assert_eq!(classify(false, false, false, false, false, ns()), Access::Limited);
    }

    #[test]
    fn labels_name_the_namespace_when_scoped() {
        assert_eq!(Access::ReadWrite(Some("team".into())).label(), "read-write (team)");
        assert_eq!(Access::ReadOnly(None).label(), "read-only");
    }
}
