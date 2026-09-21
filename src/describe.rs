//! What each resource kind shows beyond namespace/name/age, after
//! Freelens: ReplicaSets get desired/current/ready, Services their type,
//! IPs and ports, PVCs their status and size, and so on. Each kind
//! describes itself as a few named columns with a colour tone, plus one
//! status note (a coloured dot and text) for the bottom bar.

use std::collections::BTreeMap;

use k8s_openapi::api::{
    apps::v1::{DaemonSet, ReplicaSet, StatefulSet},
    autoscaling::v2::HorizontalPodAutoscaler,
    batch::v1::{CronJob, Job},
    core::v1::{ConfigMap, Endpoints, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Secret, Service, ServiceAccount},
    networking::v1::{Ingress, NetworkPolicy},
    rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding},
    storage::v1::StorageClass,
};
use kube::api::DynamicObject;

/// How a cell reads at a glance: healthy, needs attention, broken, or
/// just background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Good,
    Warn,
    Bad,
    Muted,
}

/// One extra column of a row.
#[derive(Clone, Debug)]
pub struct Col {
    pub header: &'static str,
    pub text: String,
    pub tone: Tone,
    /// A number to sort by instead of the text (counts, sizes).
    pub sort: Option<i64>,
}

impl Col {
    fn plain(header: &'static str, text: impl Into<String>) -> Self {
        Col { header, text: text.into(), tone: Tone::Plain, sort: None }
    }

    fn toned(header: &'static str, text: impl Into<String>, tone: Tone) -> Self {
        Col { header, text: text.into(), tone, sort: None }
    }

    fn count(header: &'static str, n: usize) -> Self {
        Col { header, text: n.to_string(), tone: Tone::Plain, sort: Some(n as i64) }
    }
}

/// The status note for the bottom bar: a tone and a short text.
pub type Note = Option<(Tone, String)>;

/// What a kind adds to its table row. The default adds nothing.
pub trait Extras {
    fn extras(&self) -> (Vec<Col>, Note) {
        (Vec::new(), None)
    }

    /// The kind's column headers, known even when its list is empty (so an
    /// empty PVC list still shows STATUS, CAPACITY, ...).
    fn headers() -> Vec<&'static str>
    where
        Self: Default,
    {
        Self::default().extras().0.iter().map(|c| c.header).collect()
    }
}

impl Extras for Node {}
impl Extras for DynamicObject {}

/// `ready/desired` with the usual tone: green when all are ready, yellow
/// when some aren't, muted when nothing is wanted.
pub fn ready_tone(ready: i64, desired: i64) -> Tone {
    if desired == 0 {
        Tone::Muted
    } else if ready >= desired {
        Tone::Good
    } else {
        Tone::Warn
    }
}

fn ready_note(ready: i64, desired: i64) -> Note {
    Some((ready_tone(ready, desired), format!("{ready}/{desired}")))
}

impl Extras for ReplicaSet {
    fn extras(&self) -> (Vec<Col>, Note) {
        let desired = i64::from(self.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0));
        let status = self.status.as_ref();
        let current = i64::from(status.map(|s| s.replicas).unwrap_or(0));
        let ready = i64::from(status.and_then(|s| s.ready_replicas).unwrap_or(0));
        let cols = vec![
            Col { header: "DESIRED", text: desired.to_string(), tone: Tone::Plain, sort: Some(desired) },
            Col { header: "CURRENT", text: current.to_string(), tone: Tone::Plain, sort: Some(current) },
            Col { header: "READY", text: ready.to_string(), tone: ready_tone(ready, desired), sort: Some(ready) },
        ];
        (cols, ready_note(ready, desired))
    }
}

impl Extras for StatefulSet {
    fn extras(&self) -> (Vec<Col>, Note) {
        let desired = i64::from(self.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1));
        let ready = i64::from(self.status.as_ref().and_then(|s| s.ready_replicas).unwrap_or(0));
        (vec![Col { header: "READY", text: format!("{ready}/{desired}"), tone: ready_tone(ready, desired), sort: Some(ready) }], ready_note(ready, desired))
    }
}

impl Extras for DaemonSet {
    fn extras(&self) -> (Vec<Col>, Note) {
        let status = self.status.as_ref();
        let desired = i64::from(status.map(|s| s.desired_number_scheduled).unwrap_or(0));
        let current = i64::from(status.map(|s| s.current_number_scheduled).unwrap_or(0));
        let ready = i64::from(status.map(|s| s.number_ready).unwrap_or(0));
        let updated = i64::from(status.and_then(|s| s.updated_number_scheduled).unwrap_or(0));
        let available = i64::from(status.and_then(|s| s.number_available).unwrap_or(0));
        let cols = vec![
            Col { header: "DESIRED", text: desired.to_string(), tone: Tone::Plain, sort: Some(desired) },
            Col { header: "CURRENT", text: current.to_string(), tone: Tone::Plain, sort: Some(current) },
            Col { header: "READY", text: ready.to_string(), tone: ready_tone(ready, desired), sort: Some(ready) },
            Col { header: "UP-TO-DATE", text: updated.to_string(), tone: Tone::Plain, sort: Some(updated) },
            Col { header: "AVAILABLE", text: available.to_string(), tone: Tone::Plain, sort: Some(available) },
        ];
        (cols, ready_note(ready, desired))
    }
}

impl Extras for Job {
    fn extras(&self) -> (Vec<Col>, Note) {
        let status = self.status.as_ref();
        let wanted = i64::from(self.spec.as_ref().and_then(|s| s.completions).unwrap_or(1));
        let done = i64::from(status.and_then(|s| s.succeeded).unwrap_or(0));
        let active = status.and_then(|s| s.active).unwrap_or(0);
        let has = |kind: &str| status.and_then(|s| s.conditions.as_ref()).is_some_and(|c| c.iter().any(|c| c.type_ == kind && c.status == "True"));
        let (state, tone) = if has("Failed") {
            ("Failed", Tone::Bad)
        } else if has("Complete") {
            ("Complete", Tone::Good)
        } else if active > 0 {
            ("Running", Tone::Warn)
        } else {
            ("Pending", Tone::Warn)
        };
        let cols = vec![
            Col { header: "COMPLETIONS", text: format!("{done}/{wanted}"), tone: Tone::Plain, sort: Some(done) },
            Col::toned("STATUS", state, tone),
        ];
        (cols, Some((tone, format!("{done}/{wanted} {state}"))))
    }
}

impl Extras for CronJob {
    fn extras(&self) -> (Vec<Col>, Note) {
        let suspended = self.spec.suspend.unwrap_or(false);
        let active = self.status.as_ref().and_then(|s| s.active.as_ref()).map_or(0, Vec::len);
        let last = self.status.as_ref().and_then(|s| s.last_schedule_time.as_ref()).map(|t| crate::k8s::humanize_age(t.0)).unwrap_or_else(|| "-".into());
        let cols = vec![
            Col::plain("SCHEDULE", self.spec.schedule.clone()),
            Col::toned("SUSPEND", if suspended { "True" } else { "False" }, if suspended { Tone::Warn } else { Tone::Plain }),
            Col::count("ACTIVE", active),
            Col::plain("LAST SCHEDULE", last),
        ];
        let note = if suspended { Some((Tone::Warn, "Suspended".to_string())) } else { Some((Tone::Good, "Scheduled".to_string())) };
        (cols, note)
    }
}

impl Extras for ConfigMap {
    fn extras(&self) -> (Vec<Col>, Note) {
        let keys = self.data.as_ref().map_or(0, BTreeMap::len) + self.binary_data.as_ref().map_or(0, BTreeMap::len);
        (vec![Col::count("KEYS", keys)], None)
    }
}

impl Extras for Secret {
    fn extras(&self) -> (Vec<Col>, Note) {
        let keys = self.data.as_ref().map_or(0, BTreeMap::len);
        (vec![Col::plain("TYPE", self.type_.clone().unwrap_or_else(|| "Opaque".into())), Col::count("KEYS", keys)], None)
    }
}

impl Extras for Service {
    fn extras(&self) -> (Vec<Col>, Note) {
        let spec = self.spec.as_ref();
        let kind = spec.and_then(|s| s.type_.clone()).unwrap_or_else(|| "ClusterIP".into());
        let cluster_ip = spec.and_then(|s| s.cluster_ip.clone()).filter(|ip| !ip.is_empty()).unwrap_or_else(|| "<none>".into());
        let ports = spec
            .and_then(|s| s.ports.as_ref())
            .map(|ports| {
                ports
                    .iter()
                    .map(|p| {
                        let proto = p.protocol.as_deref().unwrap_or("TCP");
                        match p.node_port {
                            Some(node) => format!("{}:{node}/{proto}", p.port),
                            None => format!("{}/{proto}", p.port),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "<none>".into());
        let mut external: Vec<String> = spec.and_then(|s| s.external_ips.clone()).unwrap_or_default();
        if let Some(ingress) = self.status.as_ref().and_then(|s| s.load_balancer.as_ref()).and_then(|l| l.ingress.as_ref()) {
            external.extend(ingress.iter().filter_map(|i| i.ip.clone().or_else(|| i.hostname.clone())));
        }
        let external = if external.is_empty() { "<none>".to_string() } else { external.join(",") };
        let cols = vec![Col::plain("TYPE", kind), Col::plain("CLUSTER-IP", cluster_ip), Col::plain("PORTS", ports), Col::plain("EXTERNAL-IP", external)];
        (cols, None)
    }
}

impl Extras for Endpoints {
    fn extras(&self) -> (Vec<Col>, Note) {
        let mut addresses: Vec<String> = Vec::new();
        for subset in self.subsets.iter().flatten() {
            let port = subset.ports.as_ref().and_then(|p| p.first()).map(|p| p.port);
            for address in subset.addresses.iter().flatten() {
                addresses.push(match port {
                    Some(port) => format!("{}:{port}", address.ip),
                    None => address.ip.clone(),
                });
            }
        }
        let text = match addresses.len() {
            0 => "<none>".to_string(),
            1..=2 => addresses.join(","),
            n => format!("{},{}, +{}", addresses[0], addresses[1], n - 2),
        };
        (vec![Col::toned("ENDPOINTS", text, if addresses.is_empty() { Tone::Warn } else { Tone::Plain })], None)
    }
}

impl Extras for Ingress {
    fn extras(&self) -> (Vec<Col>, Note) {
        let spec = self.spec.as_ref();
        let hosts: Vec<String> = spec.and_then(|s| s.rules.as_ref()).map(|r| r.iter().filter_map(|r| r.host.clone()).collect()).unwrap_or_default();
        let address = self
            .status
            .as_ref()
            .and_then(|s| s.load_balancer.as_ref())
            .and_then(|l| l.ingress.as_ref())
            .map(|i| i.iter().filter_map(|i| i.ip.clone().or_else(|| i.hostname.clone())).collect::<Vec<_>>().join(","))
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "-".into());
        let ports = if spec.and_then(|s| s.tls.as_ref()).is_some_and(|t| !t.is_empty()) { "80,443" } else { "80" };
        let cols = vec![
            Col::plain("CLASS", spec.and_then(|s| s.ingress_class_name.clone()).unwrap_or_else(|| "<none>".into())),
            Col::plain("HOSTS", if hosts.is_empty() { "*".to_string() } else { hosts.join(",") }),
            Col::plain("ADDRESS", address),
            Col::plain("PORTS", ports),
        ];
        (cols, None)
    }
}

impl Extras for NetworkPolicy {
    fn extras(&self) -> (Vec<Col>, Note) {
        let types = self.spec.as_ref().and_then(|s| s.policy_types.as_ref()).map(|t| t.join(",")).filter(|t| !t.is_empty()).unwrap_or_else(|| "Ingress".into());
        (vec![Col::plain("POLICY TYPES", types)], None)
    }
}

/// `ReadWriteOnce` -> `RWO`, the way kubectl abbreviates access modes.
fn access_modes(modes: Option<&Vec<String>>) -> String {
    let short: Vec<&str> = modes
        .into_iter()
        .flatten()
        .map(|m| match m.as_str() {
            "ReadWriteOnce" => "RWO",
            "ReadOnlyMany" => "ROX",
            "ReadWriteMany" => "RWX",
            "ReadWriteOncePod" => "RWOP",
            other => other,
        })
        .collect();
    if short.is_empty() { "-".into() } else { short.join(",") }
}

fn storage(capacity: Option<&BTreeMap<String, k8s_openapi::apimachinery::pkg::api::resource::Quantity>>) -> String {
    capacity.and_then(|c| c.get("storage")).map(|q| q.0.clone()).unwrap_or_else(|| "-".into())
}

fn phase_tone(phase: &str) -> Tone {
    match phase {
        "Bound" | "Active" | "Available" => Tone::Good,
        "Pending" | "Terminating" | "Released" => Tone::Warn,
        "Lost" | "Failed" => Tone::Bad,
        _ => Tone::Plain,
    }
}

impl Extras for PersistentVolumeClaim {
    fn extras(&self) -> (Vec<Col>, Note) {
        let phase = self.status.as_ref().and_then(|s| s.phase.clone()).unwrap_or_else(|| "Unknown".into());
        let spec = self.spec.as_ref();
        let cols = vec![
            Col::toned("STATUS", phase.clone(), phase_tone(&phase)),
            Col::plain("VOLUME", spec.and_then(|s| s.volume_name.clone()).unwrap_or_else(|| "-".into())),
            Col::plain("CAPACITY", storage(self.status.as_ref().and_then(|s| s.capacity.as_ref()))),
            Col::plain("ACCESS", access_modes(spec.and_then(|s| s.access_modes.as_ref()))),
            Col::plain("STORAGE CLASS", spec.and_then(|s| s.storage_class_name.clone()).unwrap_or_else(|| "-".into())),
        ];
        (cols, Some((phase_tone(&phase), phase)))
    }
}

impl Extras for PersistentVolume {
    fn extras(&self) -> (Vec<Col>, Note) {
        let phase = self.status.as_ref().and_then(|s| s.phase.clone()).unwrap_or_else(|| "Unknown".into());
        let spec = self.spec.as_ref();
        let claim = spec
            .and_then(|s| s.claim_ref.as_ref())
            .map(|c| format!("{}/{}", c.namespace.clone().unwrap_or_default(), c.name.clone().unwrap_or_default()))
            .unwrap_or_else(|| "-".into());
        let cols = vec![
            Col::toned("STATUS", phase.clone(), phase_tone(&phase)),
            Col::plain("CAPACITY", storage(spec.and_then(|s| s.capacity.as_ref()))),
            Col::plain("ACCESS", access_modes(spec.and_then(|s| s.access_modes.as_ref()))),
            Col::plain("RECLAIM", spec.and_then(|s| s.persistent_volume_reclaim_policy.clone()).unwrap_or_else(|| "-".into())),
            Col::plain("CLAIM", claim),
            Col::plain("STORAGE CLASS", spec.and_then(|s| s.storage_class_name.clone()).unwrap_or_else(|| "-".into())),
        ];
        (cols, Some((phase_tone(&phase), phase)))
    }
}

impl Extras for StorageClass {
    fn extras(&self) -> (Vec<Col>, Note) {
        let default = self.metadata.annotations.as_ref().and_then(|a| a.get("storageclass.kubernetes.io/is-default-class")).is_some_and(|v| v == "true");
        let cols = vec![
            Col::plain("PROVISIONER", self.provisioner.clone()),
            Col::plain("RECLAIM", self.reclaim_policy.clone().unwrap_or_else(|| "Delete".into())),
            Col::plain("BINDING MODE", self.volume_binding_mode.clone().unwrap_or_else(|| "Immediate".into())),
            Col::toned("DEFAULT", if default { "Yes" } else { "" }, Tone::Good),
        ];
        (cols, None)
    }
}

impl Extras for Namespace {
    fn extras(&self) -> (Vec<Col>, Note) {
        let phase = self.status.as_ref().and_then(|s| s.phase.clone()).unwrap_or_else(|| "Active".into());
        (vec![Col::toned("STATUS", phase.clone(), phase_tone(&phase))], Some((phase_tone(&phase), phase)))
    }
}

impl Extras for HorizontalPodAutoscaler {
    fn extras(&self) -> (Vec<Col>, Note) {
        let spec = &self.spec;
        let status = self.status.as_ref();
        let current = i64::from(status.and_then(|s| s.current_replicas).unwrap_or(0));
        let desired = i64::from(status.map(|s| s.desired_replicas).unwrap_or(0));
        let cols = vec![
            Col::plain("REFERENCE", format!("{}/{}", spec.scale_target_ref.kind, spec.scale_target_ref.name)),
            Col::plain("MIN", spec.min_replicas.map_or("1".to_string(), |n| n.to_string())),
            Col::plain("MAX", spec.max_replicas.to_string()),
            Col { header: "REPLICAS", text: current.to_string(), tone: if current == desired { Tone::Good } else { Tone::Warn }, sort: Some(current) },
        ];
        (cols, Some((if current == desired { Tone::Good } else { Tone::Warn }, format!("{current} -> {desired}"))))
    }
}

impl Extras for ServiceAccount {
    fn extras(&self) -> (Vec<Col>, Note) {
        (vec![Col::count("SECRETS", self.secrets.as_ref().map_or(0, Vec::len))], None)
    }
}

impl Extras for Role {
    fn extras(&self) -> (Vec<Col>, Note) {
        (vec![Col::count("RULES", self.rules.as_ref().map_or(0, Vec::len))], None)
    }
}

impl Extras for ClusterRole {
    fn extras(&self) -> (Vec<Col>, Note) {
        (vec![Col::count("RULES", self.rules.as_ref().map_or(0, Vec::len))], None)
    }
}

fn subjects_text(subjects: Option<&Vec<k8s_openapi::api::rbac::v1::Subject>>) -> String {
    let names: Vec<String> = subjects.into_iter().flatten().map(|s| s.name.clone()).collect();
    match names.len() {
        0 => "-".into(),
        1..=2 => names.join(","),
        n => format!("{},{}, +{}", names[0], names[1], n - 2),
    }
}

impl Extras for RoleBinding {
    fn extras(&self) -> (Vec<Col>, Note) {
        (vec![Col::plain("ROLE", format!("{}/{}", self.role_ref.kind, self.role_ref.name)), Col::plain("SUBJECTS", subjects_text(self.subjects.as_ref()))], None)
    }
}

impl Extras for ClusterRoleBinding {
    fn extras(&self) -> (Vec<Col>, Note) {
        (vec![Col::plain("ROLE", format!("{}/{}", self.role_ref.kind, self.role_ref.name)), Col::plain("SUBJECTS", subjects_text(self.subjects.as_ref()))], None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::apps::v1::{ReplicaSetSpec, ReplicaSetStatus};

    fn replica_set(desired: i32, current: i32, ready: i32) -> ReplicaSet {
        ReplicaSet {
            spec: Some(ReplicaSetSpec { replicas: Some(desired), ..Default::default() }),
            status: Some(ReplicaSetStatus { replicas: current, ready_replicas: Some(ready), ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn replica_sets_show_desired_current_ready_and_a_toned_note() {
        let (cols, note) = replica_set(3, 3, 3).extras();
        assert_eq!(cols.iter().map(|c| c.header).collect::<Vec<_>>(), ["DESIRED", "CURRENT", "READY"]);
        assert_eq!(cols[2].tone, Tone::Good);
        assert_eq!(note, Some((Tone::Good, "3/3".to_string())));

        let (cols, note) = replica_set(3, 3, 1).extras();
        assert_eq!(cols[2].tone, Tone::Warn);
        assert_eq!(note, Some((Tone::Warn, "1/3".to_string())));

        let (_, note) = replica_set(0, 0, 0).extras();
        assert_eq!(note, Some((Tone::Muted, "0/0".to_string())));
    }

    #[test]
    fn access_modes_are_abbreviated_like_kubectl() {
        assert_eq!(access_modes(Some(&vec!["ReadWriteOnce".to_string(), "ReadOnlyMany".to_string()])), "RWO,ROX");
        assert_eq!(access_modes(None), "-");
    }

    #[test]
    fn phase_tones() {
        assert_eq!(phase_tone("Bound"), Tone::Good);
        assert_eq!(phase_tone("Pending"), Tone::Warn);
        assert_eq!(phase_tone("Lost"), Tone::Bad);
    }
}
