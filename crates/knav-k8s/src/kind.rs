

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResourceKind {
    Overview,
    Pods,
    Deployments,
    Nodes,
    Namespaces,
    ReplicaSets,
    StatefulSets,
    DaemonSets,
    Jobs,
    CronJobs,
    ConfigMaps,
    Secrets,
    Hpas,
    Services,
    Endpoints,
    Ingresses,
    NetworkPolicies,
    Pvcs,
    Pvs,
    StorageClasses,
    ServiceAccounts,
    Roles,
    RoleBindings,
    ClusterRoles,
    ClusterRoleBindings,
    /// Helm releases, decoded from the Secrets Helm writes.
    HelmReleases,
    /// An extension category's dashboard, by category name.
    ExtensionDashboard(&'static str),
    /// The running port-forwards, knav's own rather than a cluster resource.
    PortForwards,
    /// The Custom Resources picker: every discovered CRD kind.
    CustomResourceList,
    /// The same picker, filtered to one API group (a leaked `&'static str`).
    CustomResourceGroup(&'static str),
    /// One CRD kind's objects: an index into `Catalog::crds`, and its label.
    CustomResource(usize, &'static str),
    /// Every resource type the API server lists (`:api`).
    ApiResources,
    /// One discovered type shown through the server's Table view: an index into
    /// `Catalog::apis`, and its plural.
    Api(usize, &'static str),
}

impl ResourceKind {
    pub fn label(self) -> &'static str {
        match self {
            ResourceKind::Overview => "Home",
            ResourceKind::Pods => "Pods",
            ResourceKind::Deployments => "Deployments",
            ResourceKind::Nodes => "Nodes",
            ResourceKind::Namespaces => "Namespaces",
            ResourceKind::ReplicaSets => "ReplicaSets",
            ResourceKind::StatefulSets => "StatefulSets",
            ResourceKind::DaemonSets => "DaemonSets",
            ResourceKind::Jobs => "Jobs",
            ResourceKind::CronJobs => "CronJobs",
            ResourceKind::ConfigMaps => "ConfigMaps",
            ResourceKind::Secrets => "Secrets",
            ResourceKind::Hpas => "HPAs",
            ResourceKind::PortForwards => "Port-forwards",
            ResourceKind::Services => "Services",
            ResourceKind::Endpoints => "Endpoints",
            ResourceKind::Ingresses => "Ingresses",
            ResourceKind::NetworkPolicies => "NetworkPolicies",
            ResourceKind::Pvcs => "PVCs",
            ResourceKind::Pvs => "PVs",
            ResourceKind::StorageClasses => "StorageClasses",
            ResourceKind::ServiceAccounts => "ServiceAccounts",
            ResourceKind::Roles => "Roles",
            ResourceKind::RoleBindings => "RoleBindings",
            ResourceKind::ClusterRoles => "ClusterRoles",
            ResourceKind::ClusterRoleBindings => "ClusterRoleBindings",
            ResourceKind::HelmReleases => "HelmReleases",
            ResourceKind::ExtensionDashboard(category) => category,
            ResourceKind::CustomResourceList => "CustomResources",
            ResourceKind::CustomResourceGroup(group) => group,
            ResourceKind::CustomResource(_, label) | ResourceKind::Api(_, label) => label,
            ResourceKind::ApiResources => "API Resources",
        }
    }

    /// What Enter drills into: a Deployment's ReplicaSets, a workload's or Service's
    /// Pods, a CronJob's Jobs, a Namespace's Pods.
    pub fn drill_target(self) -> Option<ResourceKind> {
        match self {
            ResourceKind::Deployments => Some(ResourceKind::ReplicaSets),
            ResourceKind::ReplicaSets
            | ResourceKind::StatefulSets
            | ResourceKind::DaemonSets
            | ResourceKind::Jobs
            | ResourceKind::Services
            | ResourceKind::Namespaces => Some(ResourceKind::Pods),
            ResourceKind::CronJobs => Some(ResourceKind::Jobs),
            _ => None,
        }
    }

    /// Whether Enter opens the manifest, for kinds that don't drill anywhere.
    pub fn opens_spec_on_enter(self) -> bool {
        self.drill_target().is_none()
            && !matches!(
                self,
                ResourceKind::Overview
                    | ResourceKind::Pods
                    | ResourceKind::Nodes
                    | ResourceKind::CustomResourceList
                    | ResourceKind::CustomResourceGroup(_)
                    | ResourceKind::ApiResources
                    | ResourceKind::ExtensionDashboard(_)
            )
    }

    /// The reverse of `label()`, for fixed kinds only.
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "Pods" => Some(ResourceKind::Pods),
            "Deployments" => Some(ResourceKind::Deployments),
            "Nodes" => Some(ResourceKind::Nodes),
            "Namespaces" => Some(ResourceKind::Namespaces),
            "ReplicaSets" => Some(ResourceKind::ReplicaSets),
            "StatefulSets" => Some(ResourceKind::StatefulSets),
            "DaemonSets" => Some(ResourceKind::DaemonSets),
            "Jobs" => Some(ResourceKind::Jobs),
            "CronJobs" => Some(ResourceKind::CronJobs),
            "ConfigMaps" => Some(ResourceKind::ConfigMaps),
            "Secrets" => Some(ResourceKind::Secrets),
            "HPAs" => Some(ResourceKind::Hpas),
            "Services" => Some(ResourceKind::Services),
            "Endpoints" => Some(ResourceKind::Endpoints),
            "Ingresses" => Some(ResourceKind::Ingresses),
            "NetworkPolicies" => Some(ResourceKind::NetworkPolicies),
            "PVCs" => Some(ResourceKind::Pvcs),
            "PVs" => Some(ResourceKind::Pvs),
            "StorageClasses" => Some(ResourceKind::StorageClasses),
            "ServiceAccounts" => Some(ResourceKind::ServiceAccounts),
            "Roles" => Some(ResourceKind::Roles),
            "RoleBindings" => Some(ResourceKind::RoleBindings),
            "ClusterRoles" => Some(ResourceKind::ClusterRoles),
            "ClusterRoleBindings" => Some(ResourceKind::ClusterRoleBindings),
            "HelmReleases" => Some(ResourceKind::HelmReleases),
            "Port-forwards" => Some(ResourceKind::PortForwards),
            "API Resources" => Some(ResourceKind::ApiResources),
            "CustomResources" => Some(ResourceKind::CustomResourceList),
            // Dashboards depend on the loaded extensions: see `Catalog::kind_for_tile_label`.
            _ => None,
        }
    }

    /// The list an owner reference's kind belongs to, if knav has one.
    pub fn from_owner_kind(kind: &str) -> Option<Self> {
        Some(match kind {
            "Deployment" => ResourceKind::Deployments,
            "ReplicaSet" => ResourceKind::ReplicaSets,
            "StatefulSet" => ResourceKind::StatefulSets,
            "DaemonSet" => ResourceKind::DaemonSets,
            "Job" => ResourceKind::Jobs,
            "CronJob" => ResourceKind::CronJobs,
            "Node" => ResourceKind::Nodes,
            "Service" => ResourceKind::Services,
            "Pod" => ResourceKind::Pods,
            "ConfigMap" => ResourceKind::ConfigMaps,
            "Secret" => ResourceKind::Secrets,
            "HorizontalPodAutoscaler" => ResourceKind::Hpas,
            "Ingress" => ResourceKind::Ingresses,
            "PersistentVolumeClaim" => ResourceKind::Pvcs,
            "PersistentVolume" => ResourceKind::Pvs,
            "StorageClass" => ResourceKind::StorageClasses,
            "ServiceAccount" => ResourceKind::ServiceAccounts,
            _ => return None,
        })
    }

    pub fn from_command(cmd: &str) -> Option<Self> {
        COMMAND_ALIASES.iter().find(|(_, names)| names.contains(&cmd)).map(|(kind, _)| *kind)
    }

    /// Every name `:` accepts for this kind, plural first. Empty for CRD groups and kinds.
    pub fn aliases(self) -> &'static [&'static str] {
        COMMAND_ALIASES.iter().find(|(kind, _)| *kind == self).map(|(_, names)| *names).unwrap_or(&[])
    }
}

/// The names `:` accepts per kind: the plural first (what autocomplete shows),
/// then the singular and k9s's short aliases.
pub const COMMAND_ALIASES: &[(ResourceKind, &[&str])] = &[
    (ResourceKind::Overview, &["overview", "home"]),
    (ResourceKind::Pods, &["pods", "pod", "po"]),
    (ResourceKind::Deployments, &["deployments", "deployment", "deploy", "dp", "dep"]),
    (ResourceKind::Nodes, &["nodes", "node", "no"]),
    (ResourceKind::Namespaces, &["namespaces", "namespace", "ns"]),
    (ResourceKind::ReplicaSets, &["replicasets", "replicaset", "rs"]),
    (ResourceKind::StatefulSets, &["statefulsets", "statefulset", "sts"]),
    (ResourceKind::DaemonSets, &["daemonsets", "daemonset", "ds"]),
    (ResourceKind::Jobs, &["jobs", "job"]),
    (ResourceKind::CronJobs, &["cronjobs", "cronjob", "cj"]),
    (ResourceKind::ConfigMaps, &["configmaps", "configmap", "cm"]),
    (ResourceKind::Secrets, &["secrets", "secret", "sec"]),
    (ResourceKind::Hpas, &["hpas", "hpa"]),
    (ResourceKind::Services, &["services", "service", "svc"]),
    (ResourceKind::Endpoints, &["endpoints", "endpoint", "ep"]),
    (ResourceKind::Ingresses, &["ingresses", "ingress", "ing"]),
    (ResourceKind::NetworkPolicies, &["networkpolicies", "networkpolicy", "netpol"]),
    (ResourceKind::Pvcs, &["pvcs", "pvc", "persistentvolumeclaims"]),
    (ResourceKind::Pvs, &["pvs", "pv", "persistentvolumes"]),
    (ResourceKind::StorageClasses, &["storageclasses", "storageclass", "sc"]),
    (ResourceKind::ServiceAccounts, &["serviceaccounts", "serviceaccount", "sa"]),
    (ResourceKind::Roles, &["roles", "role"]),
    (ResourceKind::RoleBindings, &["rolebindings", "rolebinding", "rb"]),
    (ResourceKind::ClusterRoles, &["clusterroles", "clusterrole", "cr"]),
    (ResourceKind::ClusterRoleBindings, &["clusterrolebindings", "clusterrolebinding", "crb"]),
    (ResourceKind::PortForwards, &["portforwards", "portforward", "pf"]),
    (ResourceKind::ApiResources, &["apiresources", "api", "apis", "aliases"]),
    (ResourceKind::CustomResourceList, &["customresources", "customresource", "customresourcedefinitions", "crds", "crd"]),
];

#[cfg(test)]
mod resource_kind_tests {
    use super::*;

    #[test]
    fn owners_map_to_their_lists() {
        assert_eq!(ResourceKind::from_owner_kind("ReplicaSet"), Some(ResourceKind::ReplicaSets));
        assert_eq!(ResourceKind::from_owner_kind("Deployment"), Some(ResourceKind::Deployments));
        assert_eq!(ResourceKind::from_owner_kind("Widget"), None);
    }

    #[test]
    fn enter_opens_the_spec_except_where_it_drills_elsewhere() {
        assert!(ResourceKind::ConfigMaps.opens_spec_on_enter());
        assert!(!ResourceKind::Services.opens_spec_on_enter());
        assert!(!ResourceKind::Deployments.opens_spec_on_enter());
        assert_eq!(ResourceKind::Deployments.drill_target(), Some(ResourceKind::ReplicaSets));
        assert_eq!(ResourceKind::CronJobs.drill_target(), Some(ResourceKind::Jobs));
        assert_eq!(ResourceKind::ConfigMaps.drill_target(), None);
        assert!(ResourceKind::CustomResource(0, "Widget").opens_spec_on_enter());
        assert!(!ResourceKind::CustomResourceList.opens_spec_on_enter());
        assert!(!ResourceKind::Pods.opens_spec_on_enter());
        assert!(!ResourceKind::Nodes.opens_spec_on_enter());
    }

    #[test]
    fn command_resolves_full_names_and_short_aliases() {
        assert_eq!(ResourceKind::from_command("pods"), Some(ResourceKind::Pods));
        assert_eq!(ResourceKind::from_command("po"), Some(ResourceKind::Pods));
        assert_eq!(ResourceKind::from_command("configmaps"), Some(ResourceKind::ConfigMaps));
        assert_eq!(ResourceKind::from_command("cm"), Some(ResourceKind::ConfigMaps));
        assert_eq!(ResourceKind::from_command("crd"), Some(ResourceKind::CustomResourceList));
    }

    #[test]
    fn command_rejects_unknown_input() {
        assert_eq!(ResourceKind::from_command("bogus"), None);
        assert_eq!(ResourceKind::from_command(""), None);
    }

    #[test]
    fn from_label_and_label_round_trip_for_fixed_kinds() {
        for kind in [ResourceKind::Pods, ResourceKind::ConfigMaps, ResourceKind::CustomResourceList, ResourceKind::ClusterRoleBindings] {
            assert_eq!(ResourceKind::from_label(kind.label()), Some(kind));
        }
    }
}
