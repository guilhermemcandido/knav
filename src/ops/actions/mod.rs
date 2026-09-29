//! Actions on the selected objects: delete, scale, restart, cordon, trigger or suspend
//! a CronJob, open a shell. Each works on a `Target` read from the manifest.


#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub manifest: serde_yaml::Value,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Delete,
    Scale(i32),
    Restart,
    /// `true` marks the node unschedulable (cordon), `false` undoes it.
    Cordon(bool),
    Trigger,
    /// `true` suspends the CronJob, `false` resumes it.
    Suspend(bool),
}

impl Target {
    pub fn from_manifest(manifest: &serde_yaml::Value) -> Option<Target> {
        let text = |path: &[&str]| {
            let mut value = manifest;
            for key in path {
                value = value.get(*key)?;
            }
            value.as_str().map(str::to_string)
        };
        Some(Target {
            api_version: text(&["apiVersion"])?,
            kind: text(&["kind"])?,
            name: text(&["metadata", "name"])?,
            namespace: text(&["metadata", "namespace"]),
            manifest: manifest.clone(),
        })
    }

    /// `pod default/web-1`, or `node worker-1` when it has no namespace.
    pub fn label(&self) -> String {
        let kind = self.kind.to_lowercase();
        match &self.namespace {
            Some(ns) => format!("{kind} {ns}/{}", self.name),
            None => format!("{kind} {}", self.name),
        }
    }

    pub fn scalable(&self) -> bool {
        matches!(self.kind.as_str(), "Deployment" | "StatefulSet" | "ReplicaSet")
    }

    pub fn restartable(&self) -> bool {
        matches!(self.kind.as_str(), "Deployment" | "StatefulSet" | "DaemonSet")
    }

    /// What `kubectl port-forward` calls this object (`pod/x`, `svc/x`, `deploy/x`).
    pub fn forward_resource(&self) -> Option<String> {
        let prefix = match self.kind.as_str() {
            "Pod" => "pod",
            "Service" => "svc",
            "Deployment" => "deploy",
            _ => return None,
        };
        Some(format!("{prefix}/{}", self.name))
    }

    /// Every port the object declares (container or Service ports), in order, deduplicated.
    pub fn ports(&self) -> Vec<u16> {
        let Some(spec) = self.manifest.get("spec") else { return Vec::new() };
        let containers = spec.get("containers").or_else(|| spec.get("template")?.get("spec")?.get("containers"));
        let mut ports: Vec<u64> = containers
            .and_then(|c| c.as_sequence())
            .into_iter()
            .flatten()
            .flat_map(|c| c.get("ports").and_then(|p| p.as_sequence()).into_iter().flatten())
            .filter_map(|p| p.get("containerPort")?.as_u64())
            .collect();
        ports.extend(spec.get("ports").and_then(|p| p.as_sequence()).into_iter().flatten().filter_map(|p| p.get("port")?.as_u64()));
        let mut seen = Vec::new();
        for port in ports.into_iter().filter_map(|p| u16::try_from(p).ok()) {
            if !seen.contains(&port) {
                seen.push(port);
            }
        }
        seen
    }

    /// The desired replica count now, to pre-fill the scale prompt.
    pub fn replicas(&self) -> i64 {
        self.manifest.get("spec").and_then(|s| s.get("replicas")).and_then(|r| r.as_i64()).unwrap_or(1)
    }

    fn flag(&self, key: &str) -> bool {
        self.manifest.get("spec").and_then(|s| s.get(key)).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// The action for `o` on a node: cordon if it is schedulable, else undo.
    pub fn cordon_action(&self) -> Option<Action> {
        (self.kind == "Node").then(|| Action::Cordon(!self.flag("unschedulable")))
    }

    /// The action for `u` on a CronJob: suspend if it is running, else resume.
    pub fn suspend_action(&self) -> Option<Action> {
        (self.kind == "CronJob").then(|| Action::Suspend(!self.flag("suspend")))
    }
}


mod confirm;
mod run;
mod secret;

pub use confirm::{ConfirmSpec, confirm_spec};
pub use run::{Progress, run_many, working_title};
pub use secret::decode_secret;

#[cfg(test)]
mod tests {
    use super::run::job_from_cronjob;
    use super::*;

    fn target(yaml: &str) -> Target {
        Target::from_manifest(&serde_yaml::from_str(yaml).unwrap()).unwrap()
    }

    const DEPLOYMENT: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata: {name: web, namespace: shop}\nspec: {replicas: 3}\n";

    #[test]
    fn a_target_is_read_from_the_manifest() {
        let t = target(DEPLOYMENT);
        assert_eq!((t.api_version.as_str(), t.kind.as_str(), t.name.as_str()), ("apps/v1", "Deployment", "web"));
        assert_eq!(t.namespace.as_deref(), Some("shop"));
        assert_eq!(t.label(), "deployment shop/web");
        assert_eq!(t.replicas(), 3);
    }

    #[test]
    fn a_manifest_without_an_identity_is_no_target() {
        assert!(Target::from_manifest(&serde_yaml::from_str("foo: bar").unwrap()).is_none());
    }

    #[test]
    fn cluster_scoped_objects_are_labelled_without_a_namespace() {
        assert_eq!(target("apiVersion: v1\nkind: Node\nmetadata: {name: n1}\n").label(), "node n1");
    }

    #[test]
    fn only_workloads_scale_and_restart() {
        assert!(target(DEPLOYMENT).scalable() && target(DEPLOYMENT).restartable());
        let daemon = target("apiVersion: apps/v1\nkind: DaemonSet\nmetadata: {name: d}\n");
        assert!(!daemon.scalable() && daemon.restartable());
        let pod = target("apiVersion: v1\nkind: Pod\nmetadata: {name: p}\n");
        assert!(!pod.scalable() && !pod.restartable());
    }

    #[test]
    fn forwardable_kinds_name_themselves_the_way_kubectl_does() {
        assert_eq!(target(DEPLOYMENT).forward_resource().as_deref(), Some("deploy/web"));
        assert_eq!(target("apiVersion: v1\nkind: Pod\nmetadata: {name: p}\n").forward_resource().as_deref(), Some("pod/p"));
        assert_eq!(target("apiVersion: v1\nkind: Node\nmetadata: {name: n}\n").forward_resource(), None);
    }

    #[test]
    fn the_first_exposed_port_comes_from_containers_or_service_ports() {
        let pod = target("apiVersion: v1\nkind: Pod\nmetadata: {name: p}\nspec: {containers: [{name: a}, {name: b, ports: [{containerPort: 9000}]}]}\n");
        assert_eq!(pod.ports(), [9000]);
        let dep = target("apiVersion: apps/v1\nkind: Deployment\nmetadata: {name: d}\nspec: {template: {spec: {containers: [{name: a, ports: [{containerPort: 80}]}]}}}\n");
        assert_eq!(dep.ports(), [80]);
        let svc = target("apiVersion: v1\nkind: Service\nmetadata: {name: s}\nspec: {ports: [{port: 443}]}\n");
        assert_eq!(svc.ports(), [443]);
        assert!(target(DEPLOYMENT).ports().is_empty());
    }

    #[test]
    fn every_declared_port_is_listed_once_and_the_hint_says_when_there_are_none() {
        let pod = target("apiVersion: v1\nkind: Pod\nmetadata: {name: p}\nspec: {containers: [{name: a, ports: [{containerPort: 80}, {containerPort: 443}]}, {name: b, ports: [{containerPort: 80}]}]}\n");
        assert_eq!(pod.ports(), [80, 443]);
    }

    #[test]
    fn cordon_and_suspend_flip_the_current_state() {
        let node = target("apiVersion: v1\nkind: Node\nmetadata: {name: n}\nspec: {unschedulable: true}\n");
        assert_eq!(node.cordon_action(), Some(Action::Cordon(false)));
        let fresh = target("apiVersion: v1\nkind: Node\nmetadata: {name: n}\n");
        assert_eq!(fresh.cordon_action(), Some(Action::Cordon(true)));
        let cron = target("apiVersion: batch/v1\nkind: CronJob\nmetadata: {name: c}\nspec: {suspend: false}\n");
        assert_eq!(cron.suspend_action(), Some(Action::Suspend(true)));
        assert_eq!(target(DEPLOYMENT).cordon_action(), None);
        assert_eq!(target(DEPLOYMENT).suspend_action(), None);
    }

    #[test]
    fn destructive_actions_ask_first_and_reversible_ones_do_not() {
        let t = target(DEPLOYMENT);
        let delete = confirm_spec(Action::Delete, std::slice::from_ref(&t)).unwrap();
        assert_eq!(delete.title, "Delete deployment?");
        assert!(delete.danger);
        assert_eq!(delete.subjects, vec![("Deployment".to_string(), "shop/web".to_string())]);
        let restart = confirm_spec(Action::Restart, std::slice::from_ref(&t)).unwrap();
        assert!(!restart.danger && restart.notes[0].0.contains("rolling"));
        let cron = target("apiVersion: batch/v1\nkind: CronJob\nmetadata: {name: tick, namespace: d}\n");
        assert_eq!(confirm_spec(Action::Trigger, std::slice::from_ref(&cron)).unwrap().title, "Run CronJob now?");
        assert_eq!(confirm_spec(Action::Suspend(true), std::slice::from_ref(&cron)).unwrap().verb, "Suspend");
        assert_eq!(confirm_spec(Action::Suspend(false), std::slice::from_ref(&cron)).unwrap().verb, "Resume");
        assert!(confirm_spec(Action::Scale(2), std::slice::from_ref(&t)).is_none());
        assert!(confirm_spec(Action::Cordon(true), std::slice::from_ref(&t)).is_none());
        let ns = target("apiVersion: v1\nkind: Namespace\nmetadata: {name: shop}\n");
        assert!(confirm_spec(Action::Delete, &[ns]).unwrap().notes.iter().any(|(n, _)| n.contains("Everything")));
    }

    #[test]
    fn deleting_a_pod_says_whether_anything_brings_it_back() {
        let owned = target("apiVersion: v1\nkind: Pod\nmetadata: {name: p, namespace: d, ownerReferences: [{kind: ReplicaSet, name: web-1}]}\n");
        let lone = target("apiVersion: v1\nkind: Pod\nmetadata: {name: p, namespace: d}\n");
        let notes = |t: Target| confirm_spec(Action::Delete, &[t]).unwrap().notes;
        assert!(notes(owned)[0].0.contains("ReplicaSet web-1"));
        assert!(notes(lone)[0].1, "a warning");
    }

    #[test]
    fn secret_values_are_decoded_and_binary_ones_summarised() {
        let secret: serde_yaml::Value = serde_yaml::from_str("kind: Secret\ndata: {user: YWRtaW4=, blob: gA==, bad: '!!'}\n").unwrap();
        let decoded = decode_secret(&secret);
        assert_eq!(decoded["data"]["user"], "admin");
        assert_eq!(decoded["data"]["blob"], "<binary, 1 bytes>");
        assert_eq!(decoded["data"]["bad"], "<not base64: !!>");
        assert_eq!(decoded["kind"], "Secret");
    }

    #[test]
    fn asking_about_several_targets_counts_them() {
        let pods: Vec<Target> = (0..8).map(|i| target(&format!("apiVersion: v1\nkind: Pod\nmetadata: {{name: p{i}, namespace: d}}\n"))).collect();
        let spec = confirm_spec(Action::Delete, &pods).unwrap();
        assert_eq!(spec.title, "Delete 8 pods?");
        assert_eq!(spec.subjects.len(), 7, "six named and a +2 more");
        assert_eq!(spec.subjects[6].1, "+2 more");
        let policies: Vec<Target> = (0..2).map(|i| target(&format!("apiVersion: v1\nkind: NetworkPolicy\nmetadata: {{name: p{i}}}\n"))).collect();
        assert_eq!(confirm_spec(Action::Restart, &policies).unwrap().title, "Restart 2 networkpolicies?");
        assert_eq!(confirm_spec(Action::Scale(2), &pods), None);
        assert_eq!(confirm_spec(Action::Trigger, &pods), None, "trigger is one at a time");
    }

    #[test]
    fn a_triggered_job_is_owned_by_its_cronjob() {
        let cron = target(
            "apiVersion: batch/v1\nkind: CronJob\nmetadata: {name: tick, namespace: d, uid: abc}\nspec:\n  schedule: '* * * * *'\n  jobTemplate:\n    spec: {template: {spec: {containers: [{name: c, image: busybox}]}}}\n",
        );
        let job = job_from_cronjob(&cron, 255).unwrap();
        assert_eq!(job["metadata"]["name"], "tick-manual-ff");
        assert_eq!(job["metadata"]["ownerReferences"][0]["uid"], "abc");
        assert_eq!(job["metadata"]["annotations"]["cronjob.kubernetes.io/instantiate"], "manual");
        assert_eq!(job["spec"]["template"]["spec"]["containers"][0]["image"], "busybox");
    }
}

