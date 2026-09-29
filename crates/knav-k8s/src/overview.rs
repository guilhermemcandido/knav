use std::sync::Arc;

use k8s_openapi::api::core::v1::{Event, Node};

use super::*;

/// What the Overview shows: usage, the kind catalog and the newest-first events.
pub struct Overview {
    pub events: Vec<EventEntry>,
    pub cpu_usage_millicores: i64,
    pub cpu_capacity_millicores: i64,
    pub memory_usage_bytes: i64,
    pub memory_capacity_bytes: i64,
    pub pod_capacity: i64,
    pub metrics_available: bool,
    /// Each section's title with its kinds and counts, built by the caller.
    pub catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>,
    /// Health by kind label, for the kinds that have a notion of it.
    pub health: std::collections::HashMap<&'static str, Health>,
    /// Requests, phases and busiest namespaces, computed while the Resources view is open.
    pub report: Option<crate::report::Report>,
}

/// How many of a kind's objects are healthy, need attention or are broken.
/// Objects with a neutral state (finished, nothing wanted) count in none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Health {
    pub good: usize,
    pub warn: usize,
    pub bad: usize,
}

impl Health {
    pub fn add(&mut self, tone: crate::describe::Tone) {
        use crate::describe::Tone;
        match tone {
            Tone::Good => self.good += 1,
            Tone::Warn => self.warn += 1,
            Tone::Bad => self.bad += 1,
            Tone::Plain | Tone::Muted => {}
        }
    }
}

/// Pods: running and ready, or finished, are fine; pending or not all ready need
/// a look; crashing or erroring are broken.
pub fn pods_health(rows: &[std::sync::Arc<PodRow>]) -> Health {
    use crate::describe::Tone;
    let mut health = Health::default();
    for row in rows {
        health.add(match status_tone(&row.phase) {
            Tone::Plain if ready_is_short(&row.ready) => Tone::Warn,
            Tone::Plain | Tone::Muted => Tone::Good,
            other => other,
        });
    }
    health
}

/// Deployments: fine when every wanted replica is ready.
pub fn deployments_health(rows: &[std::sync::Arc<DeploymentRow>]) -> Health {
    use crate::describe::Tone;
    let mut health = Health::default();
    for row in rows {
        health.add(if ready_is_short(&row.ready) { Tone::Warn } else { Tone::Good });
    }
    health
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
    usage: Option<&crate::metrics::ClusterUsage>,
    catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>,
    health: std::collections::HashMap<&'static str, Health>,
    report: Option<crate::report::Report>,
) -> Overview {
    let mut feed: Vec<EventEntry> = nodes.iter().flat_map(|n| node_warnings(n)).chain(events.iter().map(|e| event_entry(e))).collect();
    feed.sort_by_key(|e| e.age_secs);

    Overview {
        events: feed,
        cpu_usage_millicores: usage.map(|u| u.cpu_millicores).unwrap_or(0),
        cpu_capacity_millicores: node_allocatable_sum(nodes, "cpu", crate::metrics::parse_cpu_millicores),
        memory_usage_bytes: usage.map(|u| u.memory_bytes).unwrap_or(0),
        memory_capacity_bytes: node_allocatable_sum(nodes, "memory", crate::metrics::parse_memory_bytes),
        pod_capacity: node_allocatable_sum(nodes, "pods", |s| s.parse().unwrap_or(0)),
        metrics_available: usage.is_some(),
        catalog,
        health,
        report,
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    fn event(i: usize) -> Arc<Event> {
        Arc::new(serde_json::from_value(serde_json::json!({
            "metadata": {"name": format!("e-{i}"), "namespace": "ns"},
            "involvedObject": {"kind": "Pod", "name": format!("web-{i}"), "namespace": "ns"},
            "reason": "Pulled", "message": "Container image already present on machine", "type": "Normal",
            "lastTimestamp": "2026-09-22T10:00:00Z"
        })).unwrap())
    }

    /// `cargo test --release bench_overview -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_overview_with_many_events() {
        let events: Vec<Arc<Event>> = (0..100_000).map(event).collect();
        let t = Instant::now();
        let o = overview(&[], &events, None, Vec::new(), Default::default(), None);
        println!("overview of {} events: {:?}", events.len(), t.elapsed());
        assert!(o.events.len() <= 100_000);
    }
}
