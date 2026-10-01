use std::collections::HashMap;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use kube::{
    Client,
    api::{Api, ApiResource, DynamicObject, ListParams},
    core::GroupVersionKind,
};
use tokio::{sync::watch, task::JoinHandle};

/// A CPU quantity in millicores: plain cores ("2"), millicores ("250m") or the
/// nanocores metrics-server reports ("123456789n").
pub fn parse_cpu_millicores(s: &str) -> i64 {
    if let Some(n) = s.strip_suffix('n') {
        (n.parse::<f64>().unwrap_or(0.0) / 1_000_000.0) as i64
    } else if let Some(u) = s.strip_suffix('u') {
        (u.parse::<f64>().unwrap_or(0.0) / 1_000.0) as i64
    } else if let Some(m) = s.strip_suffix('m') {
        m.parse::<f64>().unwrap_or(0.0) as i64
    } else {
        (s.parse::<f64>().unwrap_or(0.0) * 1000.0) as i64
    }
}

/// A memory quantity in bytes: binary (Ki, Mi, ...) or decimal (k, M, ...) suffixes,
/// or a plain count.
pub fn parse_memory_bytes(s: &str) -> i64 {
    const UNITS: &[(&str, f64)] = &[
        ("Ki", 1024.0),
        ("Mi", 1024.0 * 1024.0),
        ("Gi", 1024.0 * 1024.0 * 1024.0),
        ("Ti", 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("Pi", 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("Ei", 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("k", 1_000.0),
        ("M", 1_000_000.0),
        ("G", 1_000_000_000.0),
        ("T", 1_000_000_000_000.0),
        ("P", 1_000_000_000_000_000.0),
        ("E", 1_000_000_000_000_000_000.0),
    ];
    for (suffix, multiplier) in UNITS {
        if let Some(n) = s.strip_suffix(suffix) {
            return (n.parse::<f64>().unwrap_or(0.0) * multiplier) as i64;
        }
    }
    s.parse::<f64>().unwrap_or(0.0) as i64
}

/// Any quantity as a plain number, counting `m` as a thousandth.
pub fn parse_quantity(s: &str) -> Option<f64> {
    match s.strip_suffix('m') {
        Some(n) => n.parse::<f64>().ok().map(|v| v / 1000.0),
        None if s.chars().last().is_some_and(|c| c.is_ascii_digit() || c == '.') => s.parse().ok(),
        None => Some(parse_memory_bytes(s) as f64),
    }
}

#[derive(Clone)]
pub struct NodeUsage {
    pub name: String,
    pub cpu_millicores: i64,
    pub memory_bytes: i64,
}

#[derive(Clone)]
pub struct ClusterUsage {
    pub cpu_millicores: i64,
    pub memory_bytes: i64,
    /// Per-node usage, for the node detail view.
    pub nodes: Vec<NodeUsage>,
}

impl ClusterUsage {
    pub fn for_node(&self, name: &str) -> Option<&NodeUsage> {
        self.nodes.iter().find(|n| n.name == name)
    }
}

/// Polls node metrics (metrics-server has no watch) and publishes cluster and
/// per-node usage. `None` means metrics-server is unavailable, not zero usage.
pub fn watch_node_metrics(client: Client) -> (watch::Receiver<Option<ClusterUsage>>, JoinHandle<()>) {
    let (tx, rx) = watch::channel(None);

    let handle = tokio::spawn(async move {
        let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "NodeMetrics");
        let resource = ApiResource::from_gvk(&gvk);
        let api: Api<DynamicObject> = Api::all_with(client, &resource);
        let mut interval = tokio::time::interval(Duration::from_secs(15));

        loop {
            interval.tick().await;
            match api.list(&ListParams::default()).await {
                Ok(list) => {
                    let mut cpu_millicores = 0;
                    let mut memory_bytes = 0;
                    let mut nodes = Vec::with_capacity(list.items.len());
                    for item in &list.items {
                        let Some(usage) = item.data.get("usage") else { continue };
                        let node_cpu = usage.get("cpu").and_then(|v| v.as_str()).map(parse_cpu_millicores).unwrap_or(0);
                        let node_mem = usage.get("memory").and_then(|v| v.as_str()).map(parse_memory_bytes).unwrap_or(0);
                        cpu_millicores += node_cpu;
                        memory_bytes += node_mem;
                        if let Some(name) = item.metadata.name.clone() {
                            nodes.push(NodeUsage { name, cpu_millicores: node_cpu, memory_bytes: node_mem });
                        }
                    }
                    let _ = tx.send(Some(ClusterUsage { cpu_millicores, memory_bytes, nodes }));
                }
                Err(_) => {
                    let _ = tx.send(None);
                }
            }
        }
    });

    (rx, handle)
}

/// One pod's usage, summed over its containers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PodUsage {
    pub cpu_millicores: i64,
    pub memory_bytes: i64,
}

/// Pod usage by namespace, then name, so a lookup needs no allocation.
#[derive(Default)]
pub struct PodUsageMap(HashMap<String, HashMap<String, PodUsage>>);

impl PodUsageMap {
    pub fn get(&self, namespace: &str, name: &str) -> Option<PodUsage> {
        self.0.get(namespace)?.get(name).copied()
    }
}

/// Pod usage, polled only while something asks for it with `want`: listing every
/// pod's metrics is heavy on a big cluster. `None` means metrics-server is unavailable.
pub struct PodMetricsFeed {
    pub rx: watch::Receiver<Option<Arc<PodUsageMap>>>,
    wanted: Arc<AtomicBool>,
}

impl PodMetricsFeed {
    /// Asks for a fresh poll; call it while pods are on screen.
    pub fn want(&self) {
        self.wanted.store(true, Ordering::Relaxed);
    }
}

pub fn watch_pod_metrics(client: Client) -> (PodMetricsFeed, JoinHandle<()>) {
    let (tx, rx) = watch::channel(None);
    let wanted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&wanted);
    let handle = tokio::spawn(async move {
        let resource = ApiResource::from_gvk(&GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics"));
        let api: Api<DynamicObject> = Api::all_with(client, &resource);
        loop {
            // Wait to be asked, then poll at most every 15s, like `kubectl top`.
            while !flag.swap(false, Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            let usage = api.list(&ListParams::default()).await.ok().map(|list| Arc::new(pod_usage(&list.items)));
            let _ = tx.send(usage);
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
    (PodMetricsFeed { rx, wanted }, handle)
}

fn pod_usage(items: &[DynamicObject]) -> PodUsageMap {
    let mut map = PodUsageMap::default();
    for item in items {
        let (Some(namespace), Some(name)) = (item.metadata.namespace.clone(), item.metadata.name.clone()) else { continue };
        let mut usage = PodUsage::default();
        for container in item.data.get("containers").and_then(|c| c.as_array()).into_iter().flatten() {
            let quantity = |key: &str| container.get("usage").and_then(|u| u.get(key)).and_then(|v| v.as_str());
            usage.cpu_millicores += quantity("cpu").map(parse_cpu_millicores).unwrap_or(0);
            usage.memory_bytes += quantity("memory").map(parse_memory_bytes).unwrap_or(0);
        }
        map.0.entry(namespace).or_default().insert(name, usage);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_nanocores() {
        assert_eq!(parse_cpu_millicores("123456789n"), 123);
    }

    #[test]
    fn cpu_millicores_suffix() {
        assert_eq!(parse_cpu_millicores("250m"), 250);
    }

    #[test]
    fn cpu_whole_cores() {
        assert_eq!(parse_cpu_millicores("2"), 2000);
    }

    #[test]
    fn memory_binary_suffix() {
        assert_eq!(parse_memory_bytes("1Gi"), 1024 * 1024 * 1024);
        assert_eq!(parse_memory_bytes("512Ki"), 512 * 1024);
    }

    #[test]
    fn memory_decimal_suffix() {
        assert_eq!(parse_memory_bytes("1M"), 1_000_000);
    }

    #[test]
    fn memory_plain_bytes() {
        assert_eq!(parse_memory_bytes("128974848"), 128974848);
    }

    #[test]
    fn pod_usage_sums_the_containers() {
        let item: DynamicObject = serde_json::from_value(serde_json::json!({
            "apiVersion": "metrics.k8s.io/v1beta1", "kind": "PodMetrics",
            "metadata": {"name": "web", "namespace": "shop"},
            "containers": [{"usage": {"cpu": "100m", "memory": "64Mi"}}, {"usage": {"cpu": "50000000n", "memory": "32Mi"}}]
        })).unwrap();
        let map = pod_usage(&[item]);
        assert_eq!(map.get("shop", "web"), Some(PodUsage { cpu_millicores: 150, memory_bytes: 96 * 1024 * 1024 }));
        assert_eq!(map.get("shop", "other"), None);
    }
}
