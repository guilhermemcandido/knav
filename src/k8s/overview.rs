use std::sync::Arc;

use k8s_openapi::api::core::v1::{Event, Node};

use super::*;

/// Resource usage, the resource-kind catalog, and the merged
/// newest-first Events feed — everything the Overview dashboard needs.
/// Deliberately has no pod/deployment/node counts of its own — those
/// live as regular entries in `catalog` instead of being duplicated here.
pub struct Overview {
    pub events: Vec<EventEntry>,
    pub cpu_usage_millicores: i64,
    pub cpu_capacity_millicores: i64,
    pub memory_usage_bytes: i64,
    pub memory_capacity_bytes: i64,
    pub pod_capacity: i64,
    pub metrics_available: bool,
    /// (section title, [(kind label, live count)]) — assembled by the
    /// caller from whichever watches/pollers it's holding; this function
    /// just bundles it in alongside everything else.
    pub catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>,
}

pub(super) fn node_allocatable_sum(nodes: &[Arc<Node>], key: &str, parse: impl Fn(&str) -> i64) -> i64 {
    nodes
        .iter()
        .filter_map(|n| n.status.as_ref()?.allocatable.as_ref()?.get(key))
        .map(|q| parse(&q.0))
        .sum()
}

pub fn overview(
    nodes: &[Arc<Node>],
    events: &[Arc<Event>],
    usage: Option<&crate::k8s::metrics::ClusterUsage>,
    catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>,
) -> Overview {
    let mut feed: Vec<EventEntry> = nodes.iter().flat_map(|n| node_warnings(n)).chain(events.iter().map(|e| event_entry(e))).collect();
    feed.sort_by_key(|e| e.age_secs);

    Overview {
        events: feed,
        cpu_usage_millicores: usage.map(|u| u.cpu_millicores).unwrap_or(0),
        cpu_capacity_millicores: node_allocatable_sum(nodes, "cpu", crate::k8s::metrics::parse_cpu_millicores),
        memory_usage_bytes: usage.map(|u| u.memory_bytes).unwrap_or(0),
        memory_capacity_bytes: node_allocatable_sum(nodes, "memory", crate::k8s::metrics::parse_memory_bytes),
        pod_capacity: node_allocatable_sum(nodes, "pods", |s| s.parse().unwrap_or(0)),
        metrics_available: usage.is_some(),
        catalog,
    }
}
