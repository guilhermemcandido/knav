use serde_yaml::Value;

use super::*;

pub(super) fn service_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = vec![field("Type", text(manifest, &["spec", "type"]).unwrap_or("ClusterIP"))];
    if let Some(ip) = text(manifest, &["spec", "clusterIP"]) {
        lines.push(field("Cluster IP", ip));
    }
    let external = load_balancer_addresses(manifest);
    if let Some(name) = text(manifest, &["spec", "externalName"]) {
        lines.push(field("External name", name));
    }
    if !external.is_empty() {
        lines.push(field("External", external.join(", ")));
    }
    for port in items(manifest, &["spec", "ports"]) {
        let name = text(port, &["name"]).map(|n| format!("{n}  ")).unwrap_or_default();
        let target = at(port, &["targetPort"]).and_then(scalar).unwrap_or_default();
        let node_port = number(port, &["nodePort"]).map(|n| format!("  (node port {n})")).unwrap_or_default();
        lines.push(Line::Field("Port".into(), vec![chunk(format!("{name}{} → {target}/{}{node_port}", number(port, &["port"]).unwrap_or(0), text(port, &["protocol"]).unwrap_or("TCP")), Style::Plain)]));
    }
    let selector = pairs(at(manifest, &["spec", "selector"]));
    if !selector.is_empty() {
        lines.push(Line::Field("Selector".into(), chips(&selector)));
    }
    for (label, path) in [("Session affinity", &["spec", "sessionAffinity"][..]), ("External traffic", &["spec", "externalTrafficPolicy"]), ("Internal traffic", &["spec", "internalTrafficPolicy"])] {
        if let Some(value) = text(manifest, path).filter(|v| *v != "None") {
            lines.push(field(label, value));
        }
    }
    let ranges = strings(manifest, &["spec", "loadBalancerSourceRanges"]);
    if !ranges.is_empty() {
        lines.push(Line::Field("Allowed sources".into(), ranges.into_iter().map(|r| chunk(r, Style::Chip)).collect()));
    }
    vec![Section { title: "Service".into(), lines }]
}

pub(super) fn ingress_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = Vec::new();
    if let Some(class) = text(manifest, &["spec", "ingressClassName"]) {
        lines.push(field("Class", class));
    }
    let address = load_balancer_addresses(manifest);
    lines.push(field_styled("Address", if address.is_empty() { "none yet".to_string() } else { address.join(", ") }, if address.is_empty() { Style::Warn } else { Style::Plain }));
    if let Some(service) = text(manifest, &["spec", "defaultBackend", "service", "name"]) {
        lines.push(field("Default backend", service));
    }
    for rule in items(manifest, &["spec", "rules"]) {
        let host = text(rule, &["host"]).unwrap_or("*");
        for path in items(rule, &["http", "paths"]) {
            let service = text(path, &["backend", "service", "name"]).or_else(|| text(path, &["backend", "resource", "name"])).unwrap_or("?");
            let port = at(path, &["backend", "service", "port", "number"]).and_then(scalar).or_else(|| text(path, &["backend", "service", "port", "name"]).map(String::from)).unwrap_or_default();
            lines.push(Line::Field("Rule".into(), vec![chunk(format!("{host}{}", text(path, &["path"]).unwrap_or("/")), Style::Strong), chunk(format!("  →  {service}:{port}"), Style::Plain)]));
        }
    }
    for tls in items(manifest, &["spec", "tls"]) {
        let hosts: Vec<&str> = items(tls, &["hosts"]).iter().filter_map(Value::as_str).collect();
        lines.push(field("TLS", format!("{} (secret {})", hosts.join(", "), text(tls, &["secretName"]).unwrap_or("?"))));
    }
    vec![Section { title: "Ingress".into(), lines }]
}

fn peer_chunks(peers: &[Value]) -> Vec<Chunk> {
    peers
        .iter()
        .flat_map(|p| {
            let mut out = Vec::new();
            if let Some(block) = at(p, &["ipBlock"]) {
                let except = strings(block, &["except"]);
                let extra = if except.is_empty() { String::new() } else { format!(" except {}", except.join(",")) };
                out.push(chunk(format!("{}{extra}", text(block, &["cidr"]).unwrap_or("?")), Style::Chip));
            }
            if let Some(ns) = at(p, &["namespaceSelector"]) {
                let sel = selector_chips(Some(ns));
                out.push(chunk(if sel.is_empty() { "all namespaces".into() } else { format!("namespaces {}", sel.iter().map(|c| c.text.clone()).collect::<Vec<_>>().join(",")) }, Style::Chip));
            }
            if let Some(pod) = at(p, &["podSelector"]) {
                let sel = selector_chips(Some(pod));
                out.push(chunk(if sel.is_empty() { "all pods".into() } else { format!("pods {}", sel.iter().map(|c| c.text.clone()).collect::<Vec<_>>().join(",")) }, Style::Chip));
            }
            out
        })
        .collect()
}

fn port_chunks(ports: &[Value]) -> Vec<Chunk> {
    ports.iter().map(|p| chunk(format!("{}/{}", at(p, &["port"]).and_then(scalar).unwrap_or_else(|| "any".into()), text(p, &["protocol"]).unwrap_or("TCP")), Style::Chip)).collect()
}

pub(super) fn network_policy_sections(manifest: &Value) -> Vec<Section> {
    let selector = selector_chips(at(manifest, &["spec", "podSelector"]));
    let mut types: Vec<&str> = strings(manifest, &["spec", "policyTypes"]);
    if types.is_empty() {
        types.push("Ingress");
        if !items(manifest, &["spec", "egress"]).is_empty() {
            types.push("Egress");
        }
    }
    let mut lines = vec![if selector.is_empty() { field("Applies to", "every pod in the namespace") } else { Line::Field("Applies to".into(), selector) }, field("Policy types", types.join(", "))];
    let mut sections = Vec::new();
    for (title, key, peer_key) in [("Ingress", "ingress", "from"), ("Egress", "egress", "to")] {
        if !types.contains(&title) {
            continue;
        }
        let rules = items(manifest, &["spec", key]);
        let mut rule_lines = Vec::new();
        if rules.is_empty() {
            rule_lines.push(Line::Item(vec![chunk(format!("no rules, all {} traffic is denied", title.to_lowercase()), Style::Warn)]));
        }
        for (i, rule) in rules.iter().enumerate() {
            if i > 0 {
                rule_lines.push(Line::Blank);
            }
            let peers = items(rule, &[peer_key]);
            rule_lines.push(Line::Field(if peer_key == "from" { "From".into() } else { "To".into() }, if peers.is_empty() { vec![chunk("anywhere", Style::Warn)] } else { peer_chunks(peers) }));
            let ports = items(rule, &["ports"]);
            rule_lines.push(Line::Field("Ports".into(), if ports.is_empty() { vec![chunk("all ports", Style::Muted)] } else { port_chunks(ports) }));
        }
        sections.push(Section { title: title.into(), lines: rule_lines });
    }
    lines.shrink_to_fit();
    sections.insert(0, Section { title: "Network policy".into(), lines });
    sections
}

pub(super) fn endpoints_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = Vec::new();
    for subset in items(manifest, &["subsets"]) {
        let ports: Vec<String> = items(subset, &["ports"]).iter().map(|p| format!("{}/{}", number(p, &["port"]).unwrap_or(0), text(p, &["protocol"]).unwrap_or("TCP"))).collect();
        for (key, style) in [("addresses", Style::Good), ("notReadyAddresses", Style::Warn)] {
            for address in items(subset, &[key]) {
                let target = at(address, &["targetRef"]).map(|t| format!("   {} {}", text(t, &["kind"]).unwrap_or("?"), text(t, &["name"]).unwrap_or("?"))).unwrap_or_default();
                lines.push(Line::Item(vec![chunk(format!("{:<16}", text(address, &["ip"]).unwrap_or("?")), style), chunk(ports.join(", "), Style::Plain), chunk(target, Style::Muted), chunk(if key == "notReadyAddresses" { "   not ready" } else { "" }, Style::Warn)]));
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::Item(vec![chunk("no addresses", Style::Warn)]));
    }
    vec![Section { title: "Endpoints".into(), lines }]
}

pub(super) fn endpoint_slice_sections(manifest: &Value) -> Vec<Section> {
    let ports: Vec<String> = items(manifest, &["ports"]).iter().map(|p| format!("{}/{}", at(p, &["port"]).and_then(scalar).unwrap_or_default(), text(p, &["protocol"]).unwrap_or("TCP"))).collect();
    let mut lines = vec![field("Address type", text(manifest, &["addressType"]).unwrap_or("?")), field("Ports", ports.join(", "))];
    for endpoint in items(manifest, &["endpoints"]) {
        let ready = flag(endpoint, &["conditions", "ready"]).unwrap_or(true);
        let target = at(endpoint, &["targetRef"]).map(|t| format!("   {} {}", text(t, &["kind"]).unwrap_or("?"), text(t, &["name"]).unwrap_or("?"))).unwrap_or_default();
        let node = text(endpoint, &["nodeName"]).map(|n| format!("   on {n}")).unwrap_or_default();
        lines.push(Line::Item(vec![chunk(format!("{:<16}", strings(endpoint, &["addresses"]).join(",")), if ready { Style::Good } else { Style::Warn }), chunk(if ready { "ready" } else { "not ready" }, if ready { Style::Good } else { Style::Warn }), chunk(target, Style::Muted), chunk(node, Style::Muted)]));
    }
    vec![Section { title: "Endpoint slice".into(), lines }]
}

pub(super) fn ingress_class_sections(manifest: &Value) -> Vec<Section> {
    let default = text(manifest, &["metadata", "annotations", "ingressclass.kubernetes.io/is-default-class"]) == Some("true");
    let mut lines = vec![field("Controller", text(manifest, &["spec", "controller"]).unwrap_or("?"))];
    if default {
        lines.push(field_styled("Default", "yes", Style::Good));
    }
    if let Some(kind) = text(manifest, &["spec", "parameters", "kind"]) {
        lines.push(field("Parameters", format!("{kind} {}", text(manifest, &["spec", "parameters", "name"]).unwrap_or("?"))));
    }
    vec![Section { title: "Ingress class".into(), lines }]
}
