//! Cluster-wide numbers taken from the pods themselves: what they request, how
//! they are spread over nodes and namespaces, and their phases.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use k8s_openapi::api::core::v1::Pod;

use crate::metrics::{parse_cpu_millicores, parse_memory_bytes};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub cpu_requests_millicores: i64,
    pub memory_requests_bytes: i64,
    /// CPU (millicores) and memory (bytes) requested by the pods on each node.
    pub node_requests: HashMap<String, (i64, i64)>,
    /// Pods per phase, in a fixed order.
    pub phases: Vec<(&'static str, usize)>,
    /// The ten namespaces with the most pods, biggest first.
    pub namespaces: Vec<(String, usize)>,
    /// How many namespaces have pods at all.
    pub namespace_count: usize,
}

/// What one pod asks for: the sum over its containers.
fn requests(pod: &Pod) -> (i64, i64) {
    let Some(spec) = pod.spec.as_ref() else { return (0, 0) };
    spec.containers.iter().filter_map(|c| c.resources.as_ref()?.requests.as_ref()).fold((0, 0), |(cpu, memory), r| {
        (cpu + r.get("cpu").map_or(0, |q| parse_cpu_millicores(&q.0)), memory + r.get("memory").map_or(0, |q| parse_memory_bytes(&q.0)))
    })
}

pub fn report(pods: &[Arc<Pod>]) -> Report {
    let mut report = Report::default();
    let mut phases: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut namespaces: HashMap<String, usize> = HashMap::new();
    for pod in pods {
        let phase = match pod.status.as_ref().and_then(|s| s.phase.as_deref()) {
            Some("Running") => "Running",
            Some("Pending") => "Pending",
            Some("Succeeded") => "Succeeded",
            Some("Failed") => "Failed",
            _ => "Unknown",
        };
        *phases.entry(phase).or_default() += 1;
        *namespaces.entry(pod.metadata.namespace.clone().unwrap_or_default()).or_default() += 1;
        // Finished pods hold nothing, and neither do pods no node has taken yet.
        let Some(node) = pod.spec.as_ref().and_then(|s| s.node_name.clone()) else { continue };
        if matches!(phase, "Succeeded" | "Failed") {
            continue;
        }
        let (cpu, memory) = requests(pod);
        report.cpu_requests_millicores += cpu;
        report.memory_requests_bytes += memory;
        let slot = report.node_requests.entry(node).or_default();
        slot.0 += cpu;
        slot.1 += memory;
    }
    report.phases = ["Running", "Pending", "Succeeded", "Failed", "Unknown"].into_iter().filter_map(|p| phases.get(p).map(|n| (p, *n))).collect();
    report.namespace_count = namespaces.len();
    let mut namespaces: Vec<(String, usize)> = namespaces.into_iter().collect();
    namespaces.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    namespaces.truncate(10);
    report.namespaces = namespaces;
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(namespace: &str, node: &str, phase: &str, cpu: &str, memory: &str) -> Arc<Pod> {
        Arc::new(
            serde_json::from_value(serde_json::json!({
                "metadata": {"name": "p", "namespace": namespace},
                "spec": {"nodeName": node, "containers": [{"name": "c", "resources": {"requests": {"cpu": cpu, "memory": memory}}}, {"name": "d", "resources": {"requests": {"cpu": "50m"}}}]},
                "status": {"phase": phase}
            }))
            .unwrap(),
        )
    }

    #[test]
    fn requests_add_up_over_containers_and_skip_finished_pods() {
        let pods = vec![pod("a", "n1", "Running", "100m", "64Mi"), pod("a", "n1", "Running", "1", "1Gi"), pod("b", "n2", "Succeeded", "500m", "1Gi")];
        let r = report(&pods);
        assert_eq!(r.cpu_requests_millicores, 150 + 1050);
        assert_eq!(r.memory_requests_bytes, 64 * 1024 * 1024 + 1024 * 1024 * 1024);
        assert_eq!(r.node_requests["n1"].0, 1200);
        assert!(!r.node_requests.contains_key("n2"));
    }

    #[test]
    fn a_pod_no_node_has_taken_asks_for_nothing_yet() {
        let unscheduled: Arc<Pod> = Arc::new(serde_json::from_value(serde_json::json!({"metadata": {"name": "p", "namespace": "a"}, "spec": {"containers": [{"name": "c", "resources": {"requests": {"cpu": "4"}}}]}, "status": {"phase": "Pending"}})).unwrap());
        assert_eq!(report(&[unscheduled]).cpu_requests_millicores, 0);
    }

    #[test]
    fn phases_and_namespaces_are_counted_biggest_first() {
        let pods = vec![pod("a", "n1", "Running", "1m", "1Mi"), pod("b", "n1", "Running", "1m", "1Mi"), pod("b", "n1", "Pending", "1m", "1Mi")];
        let r = report(&pods);
        assert_eq!(r.phases, vec![("Running", 2), ("Pending", 1)]);
        assert_eq!(r.namespaces, vec![("b".to_string(), 2), ("a".to_string(), 1)]);
    }
}
