//! Karpenter: NodePools joined against the Nodes they've provisioned, via
//! the `karpenter.sh/nodepool` label Karpenter itself sets on every Node it
//! creates (not through NodeClaim as an indirection — the label is already
//! there, direct and always current).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_yaml::Value;

use crate::theme::theme;
use crate::ui::{format_bytes, truncate};

use super::{Dashboard, DashboardContext};

pub struct Karpenter;

const NODEPOOL_LABEL: &str = "karpenter.sh/nodepool";
const INSTANCE_TYPE_LABEL: &str = "node.kubernetes.io/instance-type";

struct Pool {
    name: String,
    ready: bool,
    node_class: Option<String>,
    instance_types: Vec<String>,
    cpu_usage_millicores: i64,
    cpu_capacity_millicores: i64,
    memory_usage_bytes: i64,
    memory_capacity_bytes: i64,
    nodes: Vec<PoolNode>,
}

struct PoolNode {
    name: String,
    instance_type: String,
    ready: bool,
    cpu_millicores: Option<i64>,
    memory_bytes: Option<i64>,
    pods: usize,
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(value, |v, key| v.get(*key))?.as_str()
}

fn is_ready(manifest: &Value) -> bool {
    manifest
        .get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .any(|c| text(c, &["type"]) == Some("Ready") && text(c, &["status"]) == Some("True"))
}

fn instance_types(manifest: &Value) -> Vec<String> {
    manifest
        .get("spec")
        .and_then(|s| s.get("template"))
        .and_then(|s| s.get("spec"))
        .and_then(|s| s.get("requirements"))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .find(|r| text(r, &["key"]) == Some(INSTANCE_TYPE_LABEL))
        .and_then(|r| r.get("values"))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}

impl Dashboard for Karpenter {
    fn category(&self) -> &str {
        "Karpenter"
    }

    fn title(&self) -> String {
        "Karpenter".into()
    }

    fn lines(&self, ctx: &mut DashboardContext) -> Vec<Line<'static>> {
        let nodepools = ctx.fetch("karpenter.sh", "NodePool");
        let mut pools: Vec<Pool> = nodepools
            .iter()
            .map(|np| Pool {
                name: text(np, &["metadata", "name"]).unwrap_or("").to_string(),
                ready: is_ready(np),
                node_class: text(np, &["spec", "template", "spec", "nodeClassRef", "name"]).map(String::from),
                instance_types: instance_types(np),
                cpu_usage_millicores: 0,
                cpu_capacity_millicores: 0,
                memory_usage_bytes: 0,
                memory_capacity_bytes: 0,
                nodes: Vec::new(),
            })
            .collect();
        let mut unmanaged = 0usize;
        for (node, row) in ctx.nodes.iter().zip(ctx.node_rows) {
            let label = |key: &str| node.metadata.labels.as_ref()?.get(key).cloned();
            let Some(pool) = label(NODEPOOL_LABEL).and_then(|name| pools.iter_mut().find(|p| p.name == name)) else {
                unmanaged += 1;
                continue;
            };
            pool.cpu_usage_millicores += row.cpu_millicores.unwrap_or(0);
            pool.cpu_capacity_millicores += row.cpu_capacity;
            pool.memory_usage_bytes += row.memory_bytes.unwrap_or(0);
            pool.memory_capacity_bytes += row.memory_capacity;
            pool.nodes.push(PoolNode {
                name: row.name.clone(),
                instance_type: label(INSTANCE_TYPE_LABEL).unwrap_or_else(|| "-".into()),
                ready: row.ready,
                cpu_millicores: row.cpu_millicores,
                memory_bytes: row.memory_bytes,
                pods: row.pod_count,
            });
        }
        render(ctx.nodes.len(), unmanaged, &pools)
    }
}

fn dot(good: bool) -> Span<'static> {
    if good { Span::styled("●", Style::default().fg(theme().ok)) } else { Span::styled("●", Style::default().fg(theme().bad)) }
}

fn meter_line(label: &str, used: f64, capacity: f64, format_value: impl Fn(f64) -> String) -> Line<'static> {
    let ratio = if capacity > 0.0 { (used / capacity).clamp(0.0, 1.0) } else { 0.0 };
    let color = if ratio >= 0.9 { theme().bad } else if ratio >= 0.75 { theme().warn } else { theme().ok };
    const WIDTH: usize = 24;
    let filled = ((ratio * WIDTH as f64).round() as usize).min(WIDTH);
    Line::from(vec![
        Span::styled(format!("  {label:<8}"), Style::default().fg(theme().muted)),
        Span::styled("▓".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(WIDTH - filled), Style::default().fg(theme().muted)),
        Span::raw(format!(" {} / {} ({:.0}%)", format_value(used), format_value(capacity), ratio * 100.0)),
    ])
}

fn cores(v: f64) -> String {
    format!("{:.2}", v / 1000.0)
}

fn render(total_nodes: usize, unmanaged_nodes: usize, pools: &[Pool]) -> Vec<Line<'static>> {
    let karpenter_nodes: usize = pools.iter().map(|p| p.nodes.len()).sum();
    let ready_nodes = pools.iter().flat_map(|p| &p.nodes).filter(|n| n.ready).count();
    let nodepools_ready = pools.iter().filter(|p| p.ready).count();
    let mut out = vec![
        Line::from(vec![
            Span::styled(format!("{total_nodes} "), Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("nodes total, "),
            Span::styled(format!("{karpenter_nodes} "), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)),
            Span::raw("managed by Karpenter, "),
            Span::styled(format!("{ready_nodes}"), Style::default().fg(theme().ok)),
            Span::raw(" ready"),
        ]),
        Line::from(vec![Span::styled(format!("{}/{} ", nodepools_ready, pools.len()), Style::default().add_modifier(Modifier::BOLD)), Span::raw("NodePools ready")]),
        Line::default(),
    ];
    if pools.is_empty() {
        out.push(Line::styled("No NodePools found.", Style::default().fg(theme().muted)));
        return out;
    }
    for pool in pools {
        out.push(Line::from(vec![
            dot(pool.ready),
            Span::styled(format!(" {}", pool.name), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("  node class: {}", pool.node_class.as_deref().unwrap_or("-")), Style::default().fg(theme().muted)),
        ]));
        if !pool.instance_types.is_empty() {
            out.push(Line::styled(format!("  instance types: {}", pool.instance_types.join(", ")), Style::default().fg(theme().muted)));
        }
        out.push(meter_line("CPU", pool.cpu_usage_millicores as f64, pool.cpu_capacity_millicores as f64, cores));
        out.push(meter_line("Memory", pool.memory_usage_bytes as f64, pool.memory_capacity_bytes as f64, format_bytes));
        if pool.nodes.is_empty() {
            out.push(Line::styled("  (no nodes)", Style::default().fg(theme().muted)));
        } else {
            out.push(Line::styled(format!("  {:<24}{:<16}{:<10}{:<10}{:<10}PODS", "NAME", "INSTANCE TYPE", "STATUS", "CPU", "MEMORY"), Style::default().fg(theme().muted)));
            for node in &pool.nodes {
                let (status_text, status_color) = if node.ready { ("Ready", theme().ok) } else { ("NotReady", theme().bad) };
                let cpu = node.cpu_millicores.map(|m| format!("{:.2}", m as f64 / 1000.0)).unwrap_or_else(|| "-".into());
                let mem = node.memory_bytes.map(|b| format_bytes(b as f64)).unwrap_or_else(|| "-".into());
                out.push(Line::from(vec![
                    Span::raw(format!("  {:<24}{:<16}", truncate(&node.name, 23), truncate(&node.instance_type, 15))),
                    Span::styled(format!("{status_text:<10}"), Style::default().fg(status_color)),
                    Span::raw(format!("{:<10}{:<10}{}", cpu, mem, node.pods)),
                ]));
            }
        }
        out.push(Line::default());
    }
    if unmanaged_nodes > 0 {
        out.push(Line::styled(format!("{unmanaged_nodes} other node{} not managed by Karpenter", if unmanaged_nodes == 1 { "" } else { "s" }), Style::default().fg(theme().muted)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::NodeRow;

    fn node(name: &str, pool: Option<&str>, instance_type: &str) -> (std::sync::Arc<k8s_openapi::api::core::v1::Node>, NodeRow) {
        let mut labels = std::collections::BTreeMap::new();
        if let Some(pool) = pool {
            labels.insert(NODEPOOL_LABEL.to_string(), pool.to_string());
        }
        labels.insert(INSTANCE_TYPE_LABEL.to_string(), instance_type.to_string());
        let node = k8s_openapi::api::core::v1::Node { metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta { name: Some(name.into()), labels: Some(labels), ..Default::default() }, ..Default::default() };
        let row = NodeRow {
            name: name.into(),
            ready: true,
            schedulable: true,
            roles: String::new(),
            version: String::new(),
            cpu_millicores: Some(500),
            cpu_capacity: 2000,
            memory_bytes: Some(1024),
            memory_capacity: 4096,
            pod_count: 3,
            pod_capacity: 100,
            taints: 0,
            internal_ip: String::new(),
            os_image: String::new(),
            kernel: String::new(),
            runtime: String::new(),
            age: "1d".into(),
            age_secs: 86400,
        };
        (std::sync::Arc::new(node), row)
    }

    fn nodepool(name: &str, ready: bool) -> Value {
        serde_json::from_value(serde_json::json!({
            "metadata": {"name": name},
            "spec": {"template": {"spec": {"nodeClassRef": {"name": "default"}, "requirements": [{"key": INSTANCE_TYPE_LABEL, "operator": "In", "values": ["m5.large", "m5.xlarge"]}]}}},
            "status": {"conditions": [{"type": "Ready", "status": if ready {"True"} else {"False"}}]}
        }))
        .unwrap()
    }

    #[test]
    fn nodes_group_under_their_nodepool_label_and_render_without_panicking() {
        let (n1, r1) = node("a", Some("general"), "m5.large");
        let (n2, r2) = node("b", None, "m5.large");
        let mut pools = vec![Pool {
            name: "general".into(),
            ready: is_ready(&nodepool("general", true)),
            node_class: text(&nodepool("general", true), &["spec", "template", "spec", "nodeClassRef", "name"]).map(String::from),
            instance_types: instance_types(&nodepool("general", true)),
            cpu_usage_millicores: 0,
            cpu_capacity_millicores: 0,
            memory_usage_bytes: 0,
            memory_capacity_bytes: 0,
            nodes: Vec::new(),
        }];
        let nodes = [(&n1, &r1), (&n2, &r2)];
        let mut unmanaged = 0;
        for (node, row) in nodes {
            let label = |key: &str| node.metadata.labels.as_ref()?.get(key).cloned();
            match label(NODEPOOL_LABEL).and_then(|name| pools.iter_mut().find(|p| p.name == name)) {
                Some(pool) => pool.nodes.push(PoolNode { name: row.name.clone(), instance_type: label(INSTANCE_TYPE_LABEL).unwrap_or_default(), ready: row.ready, cpu_millicores: row.cpu_millicores, memory_bytes: row.memory_bytes, pods: row.pod_count }),
                None => unmanaged += 1,
            }
        }
        assert_eq!(pools[0].nodes.len(), 1);
        assert_eq!(unmanaged, 1);
        assert_eq!(pools[0].instance_types, vec!["m5.large", "m5.xlarge"]);
        // Renders without panicking, including the "unmanaged" footer line.
        assert!(!render(2, unmanaged, &pools).is_empty());
    }
}
