use std::time::Duration;

use kube::{
    Client,
    api::{Api, ApiResource, DynamicObject, ListParams},
    core::GroupVersionKind,
};
use tokio::{sync::watch, task::JoinHandle};

/// Parses a Kubernetes CPU quantity, plain cores ("2"), millicores
/// ("250m"), or the nanocore form metrics-server actually reports for
/// live usage ("123456789n"), into millicores.
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

/// Parses a Kubernetes memory quantity, binary suffixes (Ki/Mi/Gi/Ti),
/// decimal suffixes (k/M/G/T), or a plain byte count, into bytes.
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
    /// Per-node breakdown, same poll, the Node detail view needs just
    /// one node's numbers, not the cluster total.
    pub nodes: Vec<NodeUsage>,
}

impl ClusterUsage {
    pub fn for_node(&self, name: &str) -> Option<&NodeUsage> {
        self.nodes.iter().find(|n| n.name == name)
    }
}

/// Polls `metrics.k8s.io/v1beta1/nodes` and publishes cluster and per-node usage
/// through a `watch` channel (metrics-server has no watch). `None` means
/// metrics-server is unavailable, shown as such rather than as zero.
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
}
