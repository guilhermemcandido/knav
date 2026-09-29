use serde_yaml::Value;

use super::*;

pub(super) fn node_sections(manifest: &Value) -> Vec<Section> {
    let ready = items(manifest, &["status", "conditions"]).iter().any(|c| text(c, &["type"]) == Some("Ready") && text(c, &["status"]) == Some("True"));
    let mut lines = vec![field_styled("Status", if ready { "Ready" } else { "Not ready" }, if ready { Style::Good } else { Style::Bad })];
    let roles: Vec<String> = pairs(at(manifest, &["metadata", "labels"])).into_iter().filter_map(|(k, _)| k.strip_prefix("node-role.kubernetes.io/").map(String::from)).collect();
    if !roles.is_empty() {
        lines.push(field("Roles", roles.join(", ")));
    }
    for (label, key) in [("Kubelet", "kubeletVersion"), ("OS", "osImage"), ("Kernel", "kernelVersion"), ("Runtime", "containerRuntimeVersion"), ("Architecture", "architecture")] {
        if let Some(value) = text(manifest, &["status", "nodeInfo", key]) {
            lines.push(field(label, value));
        }
    }
    for address in items(manifest, &["status", "addresses"]) {
        lines.push(field(text(address, &["type"]).unwrap_or("Address"), text(address, &["address"]).unwrap_or("")));
    }
    if is_unschedulable(manifest) {
        lines.push(field_styled("Scheduling", "disabled (cordoned)", Style::Warn));
    }
    let capacity = |key: &str| at(manifest, &["status", "allocatable", key]).and_then(scalar).unwrap_or_else(|| "?".into());
    for (label, key) in [("Pod CIDR", &["spec", "podCIDR"][..]), ("Provider", &["spec", "providerID"])] {
        if let Some(value) = text(manifest, key) {
            lines.push(field(label, value));
        }
    }
    let total = |key: &str| at(manifest, &["status", "capacity", key]).and_then(scalar).unwrap_or_else(|| "?".into());
    lines.push(field("Capacity", format!("cpu {}, memory {}, pods {}", total("cpu"), total("memory"), total("pods"))));
    lines.push(field("Allocatable", format!("cpu {}, memory {}, pods {}", capacity("cpu"), capacity("memory"), capacity("pods"))));
    let taints: Vec<String> = items(manifest, &["spec", "taints"]).iter().map(|t| format!("{}={}:{}", text(t, &["key"]).unwrap_or("?"), text(t, &["value"]).unwrap_or(""), text(t, &["effect"]).unwrap_or("?"))).collect();
    if !taints.is_empty() {
        lines.push(Line::Field("Taints".into(), taints.into_iter().map(|t| chunk(t, Style::PairChip)).collect()));
    }
    vec![Section { title: "Node".into(), lines }]
}

pub(super) fn is_unschedulable(manifest: &Value) -> bool {
    at(manifest, &["spec", "unschedulable"]).and_then(Value::as_bool).unwrap_or(false)
}

pub(super) fn hpa_sections(manifest: &Value) -> Vec<Section> {
    let target = format!("{} {}", text(manifest, &["spec", "scaleTargetRef", "kind"]).unwrap_or("?"), text(manifest, &["spec", "scaleTargetRef", "name"]).unwrap_or("?"));
    let mut lines = vec![field("Scales", target)];
    lines.push(field("Replicas", format!("{} now, {} wanted (min {}, max {})", number(manifest, &["status", "currentReplicas"]).unwrap_or(0), number(manifest, &["status", "desiredReplicas"]).unwrap_or(0), number(manifest, &["spec", "minReplicas"]).unwrap_or(1), number(manifest, &["spec", "maxReplicas"]).unwrap_or(0))));
    let current = items(manifest, &["status", "currentMetrics"]);
    for metric in items(manifest, &["spec", "metrics"]) {
        let kind = text(metric, &["type"]).unwrap_or("?");
        let (name, target) = match kind {
            "Resource" => (text(metric, &["resource", "name"]).unwrap_or("?").to_string(), at(metric, &["resource", "target"])),
            "Pods" => (text(metric, &["pods", "metric", "name"]).unwrap_or("?").to_string(), at(metric, &["pods", "target"])),
            "External" => (text(metric, &["external", "metric", "name"]).unwrap_or("?").to_string(), at(metric, &["external", "target"])),
            _ => (kind.to_string(), None),
        };
        let goal = target.and_then(|t| at(t, &["averageUtilization"]).and_then(scalar).map(|v| format!("{v}%")).or_else(|| at(t, &["averageValue"]).or_else(|| at(t, &["value"])).and_then(scalar))).unwrap_or_else(|| "?".into());
        let now = current.iter().find(|c| text(c, &["type"]) == Some(kind) && (text(c, &["resource", "name"]) == Some(&name) || text(c, &["pods", "metric", "name"]) == Some(&name) || text(c, &["external", "metric", "name"]) == Some(&name))).and_then(|c| {
            let m = at(c, &[&kind.to_lowercase()])?;
            at(m, &["current", "averageUtilization"]).and_then(scalar).map(|v| format!("{v}%")).or_else(|| at(m, &["current", "averageValue"]).or_else(|| at(m, &["current", "value"])).and_then(scalar))
        });
        lines.push(field("Metric", format!("{name}  {} / {goal}", now.unwrap_or_else(|| "unknown".into()))));
    }
    if let Some(last) = text(manifest, &["status", "lastScaleTime"]) {
        lines.push(field("Last scaled", format!("{} ago", age_of(last).unwrap_or_else(|| "?".into()))));
    }
    vec![Section { title: "Autoscaler".into(), lines }]
}

/// A kind the API server offers, from discovery.
pub(super) fn api_resource_sections(manifest: &Value) -> Vec<Section> {
    let spec = |key: &str| text(manifest, &["spec", key]).unwrap_or("").to_string();
    let group = spec("group");
    let mut lines = vec![
        field_styled("Resource", spec("plural"), Style::Strong),
        field("Kind", spec("kind")),
        field("Group", if group.is_empty() { "core".to_string() } else { group.clone() }),
        field("Version", spec("version")),
        field("Scope", if at(manifest, &["spec", "namespaced"]).and_then(Value::as_bool).unwrap_or(false) { "namespaced" } else { "cluster-wide" }),
    ];
    let api_version = if group.is_empty() { spec("version") } else { format!("{group}/{}", spec("version")) };
    lines.push(field("apiVersion", api_version));
    let verbs: Vec<Chunk> = items(manifest, &["spec", "verbs"]).iter().filter_map(Value::as_str).map(|v| chunk(v, Style::Chip)).collect();
    if !verbs.is_empty() {
        lines.push(Line::Field("Verbs".into(), verbs));
    }
    vec![Section { title: "API resource".into(), lines }]
}

pub(super) fn pdb_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = Vec::new();
    for (label, key) in [("Min available", "minAvailable"), ("Max unavailable", "maxUnavailable")] {
        if let Some(v) = at(manifest, &["spec", key]).and_then(scalar) {
            lines.push(field(label, v));
        }
    }
    let selector = selector_chips(at(manifest, &["spec", "selector"]));
    if !selector.is_empty() {
        lines.push(Line::Field("Selector".into(), selector));
    }
    let allowed = number(manifest, &["status", "disruptionsAllowed"]).unwrap_or(0);
    lines.push(field_styled("Disruptions allowed", allowed.to_string(), if allowed == 0 { Style::Warn } else { Style::Good }));
    lines.push(field("Healthy", format!("{} now, {} needed, {} expected", number(manifest, &["status", "currentHealthy"]).unwrap_or(0), number(manifest, &["status", "desiredHealthy"]).unwrap_or(0), number(manifest, &["status", "expectedPods"]).unwrap_or(0))));
    vec![Section { title: "Disruption budget".into(), lines }]
}

pub(super) fn quota_sections(manifest: &Value) -> Vec<Section> {
    let hard = pairs(at(manifest, &["status", "hard"]).or_else(|| at(manifest, &["spec", "hard"])));
    let used = pairs(at(manifest, &["status", "used"]));
    let width = hard.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
    let lines = hard
        .iter()
        .map(|(key, limit)| {
            let now = used.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_else(|| "0".into());
            let percent = match (crate::metrics::parse_quantity(&now), crate::metrics::parse_quantity(limit)) {
                (Some(a), Some(b)) if b > 0.0 => Some((a / b * 100.0).round() as i64),
                _ => None,
            };
            let style = match percent {
                Some(p) if p >= 100 => Style::Bad,
                Some(p) if p >= 80 => Style::Warn,
                _ => Style::Plain,
            };
            Line::Item(vec![chunk(format!("{key:<width$}  "), Style::Muted), chunk(format!("{now} / {limit}"), style), chunk(percent.map(|p| format!("   {p}%")).unwrap_or_default(), style)])
        })
        .collect();
    let mut sections = vec![Section { title: "Usage against the quota".into(), lines }];
    let scopes = strings(manifest, &["spec", "scopes"]);
    if !scopes.is_empty() {
        sections.push(Section { title: "Scopes".into(), lines: vec![Line::Field("Scopes".into(), scopes.into_iter().map(|s| chunk(s, Style::Chip)).collect())] });
    }
    sections
}

pub(super) fn limit_range_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = Vec::new();
    for (i, limit) in items(manifest, &["spec", "limits"]).iter().enumerate() {
        if i > 0 {
            lines.push(Line::Blank);
        }
        lines.push(Line::Item(vec![chunk(text(limit, &["type"]).unwrap_or("?"), Style::Strong)]));
        for (label, key) in [("min", "min"), ("max", "max"), ("default", "default"), ("request", "defaultRequest"), ("ratio", "maxLimitRequestRatio")] {
            let value = resource_list(at(limit, &[key]));
            if !value.is_empty() {
                lines.push(Line::Sub(label.into(), vec![chunk(value, Style::Plain)]));
            }
        }
    }
    vec![Section { title: "Limits".into(), lines }]
}

pub(super) fn lease_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = vec![field("Holder", text(manifest, &["spec", "holderIdentity"]).unwrap_or("nobody"))];
    let duration = number(manifest, &["spec", "leaseDurationSeconds"]);
    if let Some(d) = duration {
        lines.push(field("Duration", format!("{d}s")));
    }
    if let Some(renewed) = text(manifest, &["spec", "renewTime"]) {
        let stale = renewed.parse::<k8s_openapi::jiff::Timestamp>().ok().zip(duration).is_some_and(|(t, d)| k8s_openapi::jiff::Timestamp::now().as_second() - t.as_second() > d);
        lines.push(field_styled("Renewed", format!("{} ago{}", age_of(renewed).unwrap_or_else(|| "?".into()), if stale { " (expired)" } else { "" }), if stale { Style::Warn } else { Style::Plain }));
    }
    if let Some(n) = number(manifest, &["spec", "leaseTransitions"]) {
        lines.push(field("Transitions", n.to_string()));
    }
    vec![Section { title: "Lease".into(), lines }]
}

pub(super) fn crd_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = vec![field("Group", text(manifest, &["spec", "group"]).unwrap_or("?")), field("Kind", text(manifest, &["spec", "names", "kind"]).unwrap_or("?")), field("Plural", text(manifest, &["spec", "names", "plural"]).unwrap_or("?")), field("Scope", text(manifest, &["spec", "scope"]).unwrap_or("?"))];
    for (label, key) in [("Short names", "shortNames"), ("Categories", "categories")] {
        let names = strings(manifest, &["spec", "names", key]);
        if !names.is_empty() {
            lines.push(Line::Field(label.into(), names.into_iter().map(|n| chunk(n, Style::Chip)).collect()));
        }
    }
    let versions: Vec<Line> = items(manifest, &["spec", "versions"])
        .iter()
        .map(|v| {
            let mut chunks = vec![chunk(format!("{:<10}", text(v, &["name"]).unwrap_or("?")), Style::Strong)];
            chunks.push(chunk(if flag(v, &["served"]).unwrap_or(false) { "served  " } else { "not served  " }, Style::Plain));
            if flag(v, &["storage"]).unwrap_or(false) {
                chunks.push(chunk("storage  ", Style::Good));
            }
            if flag(v, &["deprecated"]).unwrap_or(false) {
                chunks.push(chunk("deprecated", Style::Warn));
            }
            Line::Item(chunks)
        })
        .collect();
    let mut sections = vec![Section { title: "Custom resource".into(), lines }];
    if !versions.is_empty() {
        sections.push(Section { title: "Versions".into(), lines: versions });
    }
    sections
}
