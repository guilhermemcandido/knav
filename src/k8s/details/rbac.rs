//! Roles, bindings and service accounts.

use serde_yaml::Value;

use super::*;

/// Verbs that change things or widen access.
fn risky_verb(verb: &str) -> bool {
    matches!(verb, "*" | "escalate" | "bind" | "impersonate")
}

fn api_group(group: &str) -> &str {
    if group.is_empty() { "core" } else { group }
}

pub(super) fn role_sections(manifest: &Value, kind: &str) -> Vec<Section> {
    let rules = items(manifest, &["rules"]);
    let wildcard = rules.iter().filter(|r| strings(r, &["verbs"]).iter().any(|v| risky_verb(v))).count();
    let mut summary = vec![field("Rules", format!("{}", rules.len()))];
    if wildcard > 0 {
        summary.push(field_styled("Broad access", format!("{wildcard} rule{} with * or escalation verbs", if wildcard == 1 { "" } else { "s" }), Style::Warn));
    }
    if kind == "ClusterRole" && at(manifest, &["aggregationRule"]).is_some() {
        summary.push(Line::Field("Aggregates".into(), items(manifest, &["aggregationRule", "clusterRoleSelectors"]).iter().flat_map(|s| selector_chips(Some(s))).collect()));
        summary.push(Line::Item(vec![chunk("rules are filled in by the controller from matching roles", Style::Muted)]));
    }
    let mut sections = vec![Section { title: "Role".into(), lines: summary }];
    let mut lines = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        if i > 0 {
            lines.push(Line::Blank);
        }
        let groups = strings(rule, &["apiGroups"]);
        let resources = strings(rule, &["resources"]);
        let urls = strings(rule, &["nonResourceURLs"]);
        let mut targets: Vec<Chunk> = Vec::new();
        for resource in &resources {
            let group = if groups.len() == 1 { api_group(groups[0]).to_string() } else { groups.iter().map(|g| api_group(g)).collect::<Vec<_>>().join("|") };
            let sensitive = resource.contains('/') && ["exec", "attach", "portforward", "proxy"].iter().any(|s| resource.ends_with(s)) || *resource == "secrets" || *resource == "*";
            targets.push(chunk(format!("{group}/{resource}"), if sensitive { Style::WarnChip } else { Style::Chip }));
        }
        targets.extend(urls.iter().map(|u| chunk(*u, Style::Chip)));
        lines.push(Line::Field("Resources".into(), targets));
        let names = strings(rule, &["resourceNames"]);
        if !names.is_empty() {
            lines.push(Line::Field("Names".into(), names.into_iter().map(|n| chunk(n, Style::Chip)).collect()));
        }
        lines.push(Line::Field("Verbs".into(), strings(rule, &["verbs"]).into_iter().map(|v| chunk(v, if risky_verb(v) { Style::WarnChip } else { Style::Chip })).collect()));
    }
    if !lines.is_empty() {
        sections.push(Section { title: "Rules".into(), lines });
    }
    sections
}

pub(super) fn binding_sections(manifest: &Value) -> Vec<Section> {
    let role = format!("{} {}", text(manifest, &["roleRef", "kind"]).unwrap_or("?"), text(manifest, &["roleRef", "name"]).unwrap_or("?"));
    let broad = matches!(text(manifest, &["roleRef", "name"]), Some("cluster-admin"));
    let lines = vec![field_styled("Grants", role, if broad { Style::Warn } else { Style::Strong }), field("API group", text(manifest, &["roleRef", "apiGroup"]).unwrap_or("rbac.authorization.k8s.io"))];
    let binding_ns = text(manifest, &["metadata", "namespace"]);
    let subjects: Vec<Line> = items(manifest, &["subjects"])
        .iter()
        .map(|s| {
            let kind = text(s, &["kind"]).unwrap_or("?");
            let name = text(s, &["name"]).unwrap_or("?");
            let namespace = text(s, &["namespace"]).or(if kind == "ServiceAccount" { binding_ns } else { None });
            let risky = matches!(name, "system:masters" | "system:unauthenticated" | "system:anonymous");
            let mut chunks = vec![chunk(format!("{kind:<15}"), Style::Muted), chunk(name, if risky { Style::Warn } else { Style::Strong })];
            if let Some(ns) = namespace {
                chunks.push(chunk(format!("   in {ns}"), Style::Muted));
            }
            Line::Item(chunks)
        })
        .collect();
    let mut sections = vec![Section { title: "Binding".into(), lines }];
    if !subjects.is_empty() {
        sections.push(Section { title: "Subjects".into(), lines: subjects });
    }
    sections
}

pub(super) fn service_account_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = Vec::new();
    match flag(manifest, &["automountServiceAccountToken"]) {
        Some(false) => lines.push(field("Token automount", "off")),
        _ => lines.push(field_styled("Token automount", "on (pods get a token by default)", Style::Muted)),
    }
    let pull = strings_of_names(items(manifest, &["imagePullSecrets"]));
    if !pull.is_empty() {
        lines.push(Line::Field("Pull secrets".into(), pull.into_iter().map(|n| chunk(n, Style::Chip)).collect()));
    }
    let secrets = strings_of_names(items(manifest, &["secrets"]));
    if !secrets.is_empty() {
        lines.push(Line::Field("Secrets".into(), secrets.into_iter().map(|n| chunk(n, Style::Chip)).collect()));
    }
    vec![Section { title: "Service account".into(), lines }]
}

fn strings_of_names(list: &[Value]) -> Vec<String> {
    list.iter().filter_map(|i| text(i, &["name"]).map(String::from)).collect()
}
