//! A readable summary of one object, like the drawer of a desktop client: name,
//! namespace, labels, status, containers, conditions and events, instead of raw
//! YAML. It works on the manifest, so it covers every kind; the common kinds
//! get their own sections.

use serde_yaml::Value;

use crate::k8s::EventEntry;
use crate::k8s::describe::Tone;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Plain,
    Strong,
    Muted,
    Good,
    Warn,
    Bad,
    /// A label or key, drawn as a small pill.
    Chip,
}

impl From<Tone> for Style {
    fn from(tone: Tone) -> Style {
        match tone {
            Tone::Good => Style::Good,
            Tone::Warn => Style::Warn,
            Tone::Bad => Style::Bad,
            Tone::Muted => Style::Muted,
            Tone::Plain => Style::Plain,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub style: Style,
}

fn chunk(text: impl Into<String>, style: Style) -> Chunk {
    Chunk { text: text.into(), style }
}

/// One line of a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// `label   value ...`, the value wrapping under itself.
    Field(String, Vec<Chunk>),
    /// A line on its own (a container's name, an event).
    Item(Vec<Chunk>),
    /// A line indented under the one before.
    Sub(String, Vec<Chunk>),
    /// A line pushed in by this many cells, to line up under a column above.
    Pad(usize, Vec<Chunk>),
    Blank,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub lines: Vec<Line>,
}

fn field(label: &str, text: impl Into<String>) -> Line {
    Line::Field(label.to_string(), vec![chunk(text, Style::Plain)])
}

fn field_styled(label: &str, text: impl Into<String>, style: Style) -> Line {
    Line::Field(label.to_string(), vec![chunk(text, style)])
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(value, |v, key| v.get(*key))?.as_str()
}

fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |v, key| v.get(*key))
}

fn items<'a>(value: &'a Value, path: &[&str]) -> &'a [Value] {
    at(value, path).and_then(Value::as_sequence).map(Vec::as_slice).unwrap_or(&[])
}

/// A number or string scalar as text.
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn number(value: &Value, path: &[&str]) -> Option<i64> {
    at(value, path).and_then(Value::as_i64)
}

/// The pairs of a string map, sorted by key.
fn pairs(value: Option<&Value>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = value.and_then(Value::as_mapping).map(|m| m.iter().filter_map(|(k, v)| Some((k.as_str()?.to_string(), scalar(v)?))).collect()).unwrap_or_default();
    out.sort();
    out
}

fn chips(pairs: &[(String, String)]) -> Vec<Chunk> {
    pairs.iter().map(|(k, v)| chunk(if v.is_empty() { k.clone() } else { format!("{k}={v}") }, Style::Chip)).collect()
}

fn age_of(timestamp: &str) -> Option<String> {
    timestamp.parse::<k8s_openapi::jiff::Timestamp>().ok().map(crate::k8s::humanize_age)
}

/// `cpu 100m, memory 64Mi` from a resources map.
fn resource_list(value: Option<&Value>) -> String {
    let all = pairs(value);
    all.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")
}

/// Conditions where `True` is the bad answer.
fn true_is_bad(kind: &str) -> bool {
    kind.ends_with("Pressure") || kind.ends_with("Unavailable") || kind.ends_with("Failure") || kind.ends_with("Failed") || kind == "ReplicaFailure"
}

fn conditions(manifest: &Value) -> Option<Section> {
    let list = items(manifest, &["status", "conditions"]);
    if list.is_empty() {
        return None;
    }
    // Type, status and reason in columns, the message under the reason.
    let type_w = list.iter().filter_map(|c| text(c, &["type"])).map(|t| t.chars().count()).max().unwrap_or(4).min(30);
    let status_w = 7;
    let lines = list
        .iter()
        .flat_map(|c| {
            let kind = text(c, &["type"]).unwrap_or("?");
            let status = text(c, &["status"]).unwrap_or("?");
            let style = match (status, true_is_bad(kind)) {
                ("True", false) | ("False", true) => Style::Good,
                ("True", true) => Style::Bad,
                ("False", false) => Style::Warn,
                _ => Style::Muted,
            };
            let mut chunks = vec![chunk(format!("{kind:<type_w$}  "), Style::Plain), chunk(format!("{status:<status_w$} "), style)];
            if let Some(reason) = text(c, &["reason"]) {
                chunks.push(chunk(reason, Style::Muted));
            }
            let mut lines = vec![Line::Item(chunks)];
            // The message goes under the reason, in full.
            if let Some(message) = text(c, &["message"]).filter(|m| !m.is_empty()) {
                lines.push(Line::Pad(2 + type_w + 2 + status_w + 1, vec![chunk(message, Style::Muted)]));
            }
            lines
        })
        .collect();
    Some(Section { title: "Conditions".into(), lines })
}

fn properties(manifest: &Value) -> Section {
    let mut lines = vec![field_styled("Name", text(manifest, &["metadata", "name"]).unwrap_or(""), Style::Strong)];
    if let Some(ns) = text(manifest, &["metadata", "namespace"]) {
        lines.push(field("Namespace", ns));
    }
    lines.push(field("Kind", format!("{} ({})", text(manifest, &["kind"]).unwrap_or("?"), text(manifest, &["apiVersion"]).unwrap_or("?"))));
    if let Some(created) = text(manifest, &["metadata", "creationTimestamp"]) {
        lines.push(field("Created", format!("{} ago  ({created})", age_of(created).unwrap_or_else(|| "?".into()))));
    }
    if let Some(deleted) = text(manifest, &["metadata", "deletionTimestamp"]) {
        lines.push(field_styled("Deleting", format!("since {deleted}"), Style::Warn));
    }
    for owner in items(manifest, &["metadata", "ownerReferences"]) {
        lines.push(field("Controlled by", format!("{} {}", text(owner, &["kind"]).unwrap_or("?"), text(owner, &["name"]).unwrap_or("?"))));
    }
    let labels = pairs(at(manifest, &["metadata", "labels"]));
    if !labels.is_empty() {
        lines.push(Line::Field("Labels".into(), chips(&labels)));
    }
    let annotations = pairs(at(manifest, &["metadata", "annotations"]));
    if !annotations.is_empty() {
        lines.push(Line::Field("Annotations".into(), chips(&annotations)));
    }
    Section { title: "Properties".into(), lines }
}

/// One container: its image, state and settings.
fn container_lines(spec: &Value, status: Option<&Value>) -> Vec<Line> {
    let name = text(spec, &["name"]).unwrap_or("?");
    let mut head = vec![chunk(name, Style::Strong)];
    if let Some(status) = status {
        let (state, style) = if let Some(started) = text(status, &["state", "running", "startedAt"]) {
            (format!("Running for {}", age_of(started).unwrap_or_else(|| "?".into())), Style::Good)
        } else if let Some(waiting) = at(status, &["state", "waiting"]) {
            let reason = text(waiting, &["reason"]).unwrap_or("Waiting");
            (reason.to_string(), if matches!(reason, "ContainerCreating" | "PodInitializing") { Style::Warn } else { Style::Bad })
        } else if let Some(done) = at(status, &["state", "terminated"]) {
            let reason = text(done, &["reason"]).unwrap_or("Terminated");
            (format!("{reason} (exit {})", number(done, &["exitCode"]).unwrap_or(0)), if reason == "Completed" { Style::Muted } else { Style::Bad })
        } else {
            ("Unknown".to_string(), Style::Muted)
        };
        head.push(chunk(format!("   {state}"), style));
        let ready = at(status, &["ready"]).and_then(Value::as_bool).unwrap_or(false);
        head.push(chunk(if ready { "   ready" } else { "   not ready" }, if ready { Style::Good } else { Style::Warn }));
        let restarts = number(status, &["restartCount"]).unwrap_or(0);
        if restarts > 0 {
            head.push(chunk(format!("   {restarts} restart{}", if restarts == 1 { "" } else { "s" }), Style::Warn));
        }
    }
    let mut lines = vec![Line::Item(head), Line::Sub("image".into(), vec![chunk(text(spec, &["image"]).unwrap_or("?"), Style::Plain)])];
    let ports: Vec<String> = items(spec, &["ports"]).iter().filter_map(|p| Some(format!("{}/{}", number(p, &["containerPort"])?, text(p, &["protocol"]).unwrap_or("TCP")))).collect();
    if !ports.is_empty() {
        lines.push(Line::Sub("ports".into(), vec![chunk(ports.join(", "), Style::Plain)]));
    }
    let (requests, limits) = (resource_list(at(spec, &["resources", "requests"])), resource_list(at(spec, &["resources", "limits"])));
    if !requests.is_empty() {
        lines.push(Line::Sub("requests".into(), vec![chunk(requests, Style::Plain)]));
    }
    if !limits.is_empty() {
        lines.push(Line::Sub("limits".into(), vec![chunk(limits, Style::Plain)]));
    }
    // Environment: plain values as they are, references by where they come from.
    let mut env: Vec<Vec<Chunk>> = Vec::new();
    for var in items(spec, &["env"]) {
        let name = text(var, &["name"]).unwrap_or("?");
        let value = if let Some(v) = text(var, &["value"]) {
            vec![chunk(v, Style::Plain)]
        } else if let Some(r) = at(var, &["valueFrom", "configMapKeyRef"]) {
            vec![chunk(format!("configMap {} / {}", text(r, &["name"]).unwrap_or("?"), text(r, &["key"]).unwrap_or("?")), Style::Muted)]
        } else if let Some(r) = at(var, &["valueFrom", "secretKeyRef"]) {
            vec![chunk(format!("secret {} / {}", text(r, &["name"]).unwrap_or("?"), text(r, &["key"]).unwrap_or("?")), Style::Muted)]
        } else if let Some(r) = at(var, &["valueFrom", "fieldRef"]) {
            vec![chunk(format!("field {}", text(r, &["fieldPath"]).unwrap_or("?")), Style::Muted)]
        } else {
            vec![chunk("(from elsewhere)", Style::Muted)]
        };
        let mut chunks = vec![chunk(format!("{name} = "), Style::Strong)];
        chunks.extend(value);
        env.push(chunks);
    }
    for source in items(spec, &["envFrom"]) {
        let (kind, name) = if let Some(n) = text(source, &["configMapRef", "name"]) { ("configMap", n) } else if let Some(n) = text(source, &["secretRef", "name"]) { ("secret", n) } else { continue };
        env.push(vec![chunk(format!("every key of {kind} {name}"), Style::Muted)]);
    }
    for (i, chunks) in env.into_iter().enumerate() {
        lines.push(Line::Sub(if i == 0 { "env".into() } else { String::new() }, chunks));
    }
    for (i, mount) in items(spec, &["volumeMounts"]).iter().enumerate() {
        let read_only = at(mount, &["readOnly"]).and_then(Value::as_bool).unwrap_or(false);
        lines.push(Line::Sub(if i == 0 { "mounts".into() } else { String::new() }, vec![chunk(text(mount, &["mountPath"]).unwrap_or("?"), Style::Plain), chunk(format!("  from {}{}", text(mount, &["name"]).unwrap_or("?"), if read_only { " (read only)" } else { "" }), Style::Muted)]));
    }
    lines
}

fn containers(title: &str, specs: &[Value], statuses: &[Value]) -> Option<Section> {
    if specs.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    for spec in specs {
        if !lines.is_empty() {
            lines.push(Line::Blank);
        }
        let name = text(spec, &["name"]);
        let status = statuses.iter().find(|s| text(s, &["name"]) == name);
        lines.extend(container_lines(spec, status));
    }
    Some(Section { title: title.into(), lines })
}

fn pod_sections(manifest: &Value) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut status = Vec::new();
    if let Ok(pod) = serde_yaml::from_value::<k8s_openapi::api::core::v1::Pod>(manifest.clone()) {
        let phase = crate::k8s::pod_status(&pod);
        status.push(field_styled("Status", phase.clone(), crate::k8s::status_tone(&phase).into()));
    }
    for (label, path) in [("Node", &["spec", "nodeName"][..]), ("Pod IP", &["status", "podIP"]), ("Host IP", &["status", "hostIP"]), ("QoS", &["status", "qosClass"]), ("Service account", &["spec", "serviceAccountName"]), ("Restart policy", &["spec", "restartPolicy"]), ("Priority class", &["spec", "priorityClassName"])] {
        if let Some(value) = text(manifest, path) {
            status.push(field(label, value));
        }
    }
    sections.push(Section { title: "Status".into(), lines: status });
    let statuses = items(manifest, &["status", "containerStatuses"]);
    sections.extend(containers("Containers", items(manifest, &["spec", "containers"]), statuses));
    sections.extend(containers("Init containers", items(manifest, &["spec", "initContainers"]), items(manifest, &["status", "initContainerStatuses"])));
    let volumes: Vec<Line> = items(manifest, &["spec", "volumes"])
        .iter()
        .map(|v| {
            let kind = v.as_mapping().and_then(|m| m.keys().filter_map(Value::as_str).find(|k| *k != "name")).unwrap_or("?");
            Line::Item(vec![chunk(format!("{:<28}", text(v, &["name"]).unwrap_or("?")), Style::Plain), chunk(kind, Style::Muted)])
        })
        .collect();
    if !volumes.is_empty() {
        sections.push(Section { title: "Volumes".into(), lines: volumes });
    }
    sections
}

/// Deployments, ReplicaSets, StatefulSets, DaemonSets.
fn workload_sections(manifest: &Value, kind: &str) -> Vec<Section> {
    let mut lines = Vec::new();
    let desired = number(manifest, &["spec", "replicas"]).unwrap_or(if kind == "DaemonSet" { number(manifest, &["status", "desiredNumberScheduled"]).unwrap_or(0) } else { 1 });
    let ready = number(manifest, &["status", if kind == "DaemonSet" { "numberReady" } else { "readyReplicas" }]).unwrap_or(0);
    lines.push(field_styled("Ready", format!("{ready}/{desired}"), if ready >= desired { Style::Good } else { Style::Warn }));
    for (label, key) in [("Updated", "updatedReplicas"), ("Available", "availableReplicas")] {
        if let Some(n) = number(manifest, &["status", key]) {
            lines.push(field(label, n.to_string()));
        }
    }
    if let Some(strategy) = text(manifest, &["spec", "strategy", "type"]).or_else(|| text(manifest, &["spec", "updateStrategy", "type"])) {
        lines.push(field("Strategy", strategy));
    }
    let selector = pairs(at(manifest, &["spec", "selector", "matchLabels"]));
    if !selector.is_empty() {
        lines.push(Line::Field("Selector".into(), chips(&selector)));
    }
    let mut sections = vec![Section { title: "Status".into(), lines }];
    sections.extend(containers("Containers", items(manifest, &["spec", "template", "spec", "containers"]), &[]));
    sections
}

fn job_sections(manifest: &Value, kind: &str) -> Vec<Section> {
    let mut lines = Vec::new();
    if kind == "CronJob" {
        lines.push(field("Schedule", text(manifest, &["spec", "schedule"]).unwrap_or("?")));
        let suspended = at(manifest, &["spec", "suspend"]).and_then(Value::as_bool).unwrap_or(false);
        lines.push(field_styled("Suspended", if suspended { "yes" } else { "no" }, if suspended { Style::Warn } else { Style::Plain }));
        if let Some(last) = text(manifest, &["status", "lastScheduleTime"]) {
            lines.push(field("Last run", format!("{} ago", age_of(last).unwrap_or_default())));
        }
        lines.push(field("Active", items(manifest, &["status", "active"]).len().to_string()));
        return vec![
            Section { title: "Schedule".into(), lines },
        ]
        .into_iter()
        .chain(containers("Containers", items(manifest, &["spec", "jobTemplate", "spec", "template", "spec", "containers"]), &[]))
        .collect();
    }
    let wanted = number(manifest, &["spec", "completions"]).unwrap_or(1);
    let done = number(manifest, &["status", "succeeded"]).unwrap_or(0);
    lines.push(field_styled("Completions", format!("{done}/{wanted}"), if done >= wanted { Style::Good } else { Style::Warn }));
    for (label, key) in [("Active", "active"), ("Failed", "failed")] {
        if let Some(n) = number(manifest, &["status", key]) {
            lines.push(field_styled(label, n.to_string(), if label == "Failed" && n > 0 { Style::Bad } else { Style::Plain }));
        }
    }
    vec![Section { title: "Status".into(), lines }].into_iter().chain(containers("Containers", items(manifest, &["spec", "template", "spec", "containers"]), &[])).collect()
}

fn service_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = vec![field("Type", text(manifest, &["spec", "type"]).unwrap_or("ClusterIP"))];
    if let Some(ip) = text(manifest, &["spec", "clusterIP"]) {
        lines.push(field("Cluster IP", ip));
    }
    let external: Vec<String> = items(manifest, &["status", "loadBalancer", "ingress"]).iter().filter_map(|i| text(i, &["ip"]).or_else(|| text(i, &["hostname"])).map(String::from)).collect();
    if !external.is_empty() {
        lines.push(field("External", external.join(", ")));
    }
    for port in items(manifest, &["spec", "ports"]) {
        let name = text(port, &["name"]).map(|n| format!("{n}  ")).unwrap_or_default();
        let target = at(port, &["targetPort"]).and_then(scalar).unwrap_or_default();
        lines.push(Line::Field("Port".into(), vec![chunk(format!("{name}{} → {target}/{}", number(port, &["port"]).unwrap_or(0), text(port, &["protocol"]).unwrap_or("TCP")), Style::Plain)]));
    }
    let selector = pairs(at(manifest, &["spec", "selector"]));
    if !selector.is_empty() {
        lines.push(Line::Field("Selector".into(), chips(&selector)));
    }
    vec![Section { title: "Service".into(), lines }]
}

fn ingress_sections(manifest: &Value) -> Vec<Section> {
    let mut lines = Vec::new();
    if let Some(class) = text(manifest, &["spec", "ingressClassName"]) {
        lines.push(field("Class", class));
    }
    let address: Vec<String> = items(manifest, &["status", "loadBalancer", "ingress"]).iter().filter_map(|i| text(i, &["ip"]).or_else(|| text(i, &["hostname"])).map(String::from)).collect();
    lines.push(field_styled("Address", if address.is_empty() { "none yet".to_string() } else { address.join(", ") }, if address.is_empty() { Style::Warn } else { Style::Plain }));
    for rule in items(manifest, &["spec", "rules"]) {
        let host = text(rule, &["host"]).unwrap_or("*");
        for path in items(rule, &["http", "paths"]) {
            let service = text(path, &["backend", "service", "name"]).unwrap_or("?");
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

/// Values longer than this many lines are cut, saying how many were left.
const VALUE_LINES: usize = 40;

fn size_text(bytes: usize) -> String {
    match bytes {
        0..=1023 => format!("{bytes} bytes"),
        _ => format!("{:.1} KB", bytes as f64 / 1024.0),
    }
}

/// A ConfigMap's keys with their values, and a Secret's keys with their sizes:
/// secret values stay hidden here (`x` decodes them on request).
fn keys_section(manifest: &Value, kind: &str) -> Vec<Section> {
    let mut lines = Vec::new();
    if kind == "Secret" {
        lines.push(field("Type", text(manifest, &["type"]).unwrap_or("Opaque")));
        lines.push(Line::Blank);
    }
    let mut entries: Vec<(String, Option<String>, usize)> = Vec::new();
    for (field, binary) in [("data", kind == "Secret"), ("stringData", false), ("binaryData", true)] {
        let Some(map) = at(manifest, &[field]).and_then(Value::as_mapping) else { continue };
        for (key, value) in map {
            let (Some(key), Some(value)) = (key.as_str(), value.as_str()) else { continue };
            // Base64 in a Secret or in binaryData: about three bytes for every four characters.
            let size = if binary { value.len() * 3 / 4 } else { value.len() };
            entries.push((key.to_string(), (kind == "ConfigMap" && !binary).then(|| value.to_string()), size));
        }
    }
    entries.sort();
    if entries.is_empty() {
        lines.push(Line::Item(vec![chunk("no data", Style::Muted)]));
    }
    for (i, (key, value, size)) in entries.iter().enumerate() {
        if i > 0 {
            lines.push(Line::Blank);
        }
        let mut head = vec![chunk(key.clone(), Style::Strong), chunk(format!("   {}", size_text(*size)), Style::Muted)];
        match value {
            Some(value) => {
                lines.push(Line::Item(head));
                let all: Vec<&str> = value.lines().collect();
                for line in all.iter().take(VALUE_LINES) {
                    lines.push(Line::Pad(4, vec![chunk(*line, Style::Plain)]));
                }
                if all.len() > VALUE_LINES {
                    lines.push(Line::Pad(4, vec![chunk(format!("… {} more lines", all.len() - VALUE_LINES), Style::Muted)]));
                }
            }
            None => {
                head.push(chunk(if kind == "Secret" { "   hidden" } else { "   binary" }, Style::Muted));
                lines.push(Line::Item(head));
            }
        }
    }
    let title = if kind == "Secret" { "Secret (values hidden, x decodes them)" } else { "Data" };
    vec![Section { title: title.into(), lines }]
}

fn storage_sections(manifest: &Value, kind: &str) -> Vec<Section> {
    let phase = text(manifest, &["status", "phase"]).unwrap_or("Unknown");
    let style = match phase {
        "Bound" | "Available" | "Active" => Style::Good,
        "Pending" | "Released" | "Terminating" => Style::Warn,
        "Lost" | "Failed" => Style::Bad,
        _ => Style::Plain,
    };
    let mut lines = vec![field_styled("Status", phase, style)];
    let capacity = at(manifest, &["status", "capacity", "storage"]).or_else(|| at(manifest, &["spec", "capacity", "storage"])).and_then(scalar);
    if let Some(capacity) = capacity {
        lines.push(field("Capacity", capacity));
    }
    let modes: Vec<&str> = items(manifest, &["spec", "accessModes"]).iter().filter_map(Value::as_str).collect();
    if !modes.is_empty() {
        lines.push(field("Access", modes.join(", ")));
    }
    for (label, path) in [("Storage class", &["spec", "storageClassName"][..]), ("Volume", &["spec", "volumeName"]), ("Reclaim policy", &["spec", "persistentVolumeReclaimPolicy"])] {
        if let Some(value) = text(manifest, path) {
            lines.push(field(label, value));
        }
    }
    if kind == "PersistentVolume"
        && let Some(claim) = text(manifest, &["spec", "claimRef", "name"])
    {
        lines.push(field("Claim", format!("{}/{claim}", text(manifest, &["spec", "claimRef", "namespace"]).unwrap_or("?"))));
    }
    vec![Section { title: "Storage".into(), lines }]
}

fn node_sections(manifest: &Value) -> Vec<Section> {
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
    if lets_unschedulable(manifest) {
        lines.push(field_styled("Scheduling", "disabled (cordoned)", Style::Warn));
    }
    let capacity = |key: &str| at(manifest, &["status", "allocatable", key]).and_then(scalar).unwrap_or_else(|| "?".into());
    lines.push(field("Allocatable", format!("cpu {}, memory {}, pods {}", capacity("cpu"), capacity("memory"), capacity("pods"))));
    let taints: Vec<String> = items(manifest, &["spec", "taints"]).iter().map(|t| format!("{}={}:{}", text(t, &["key"]).unwrap_or("?"), text(t, &["value"]).unwrap_or(""), text(t, &["effect"]).unwrap_or("?"))).collect();
    if !taints.is_empty() {
        lines.push(Line::Field("Taints".into(), taints.into_iter().map(|t| chunk(t, Style::Chip)).collect()));
    }
    vec![Section { title: "Node".into(), lines }]
}

fn lets_unschedulable(manifest: &Value) -> bool {
    at(manifest, &["spec", "unschedulable"]).and_then(Value::as_bool).unwrap_or(false)
}

fn hpa_sections(manifest: &Value) -> Vec<Section> {
    let target = format!("{} {}", text(manifest, &["spec", "scaleTargetRef", "kind"]).unwrap_or("?"), text(manifest, &["spec", "scaleTargetRef", "name"]).unwrap_or("?"));
    let mut lines = vec![field("Scales", target)];
    lines.push(field("Replicas", format!("{} now, {} wanted (min {}, max {})", number(manifest, &["status", "currentReplicas"]).unwrap_or(0), number(manifest, &["status", "desiredReplicas"]).unwrap_or(0), number(manifest, &["spec", "minReplicas"]).unwrap_or(1), number(manifest, &["spec", "maxReplicas"]).unwrap_or(0))));
    vec![Section { title: "Autoscaler".into(), lines }]
}

fn events_section(manifest: &Value, events: &[EventEntry]) -> Option<Section> {
    let (kind, name) = (text(manifest, &["kind"])?, text(manifest, &["metadata", "name"])?);
    let mine: Vec<&EventEntry> = events.iter().filter(|e| e.kind == kind && e.object == name).take(8).collect();
    if mine.is_empty() {
        return None;
    }
    let lines = mine
        .into_iter()
        .map(|e| {
            let style = if e.severity == crate::k8s::EventSeverity::Warning { Style::Warn } else { Style::Muted };
            Line::Item(vec![chunk(format!("{:<6}", e.age), Style::Muted), chunk(format!("{:<18}", e.reason), style), chunk(e.message.clone(), Style::Plain)])
        })
        .collect();
    Some(Section { title: "Events".into(), lines })
}

/// A kind the API server offers, from discovery.
fn api_resource_sections(manifest: &Value) -> Vec<Section> {
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

/// The sections that describe `manifest`, with the events that mention it.
pub fn details(manifest: &Value, events: &[EventEntry]) -> Vec<Section> {
    let kind = text(manifest, &["kind"]).unwrap_or("");
    if kind == "APIResource" {
        return api_resource_sections(manifest);
    }
    let mut sections = vec![properties(manifest)];
    let specific = match kind {
        "Pod" => pod_sections(manifest),
        "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet" => workload_sections(manifest, kind),
        "Job" | "CronJob" => job_sections(manifest, kind),
        "Service" => service_sections(manifest),
        "Ingress" => ingress_sections(manifest),
        "ConfigMap" | "Secret" => keys_section(manifest, kind),
        "PersistentVolumeClaim" | "PersistentVolume" => storage_sections(manifest, kind),
        "Node" => node_sections(manifest),
        "HorizontalPodAutoscaler" => hpa_sections(manifest),
        "Namespace" => vec![Section { title: "Status".into(), lines: vec![field_styled("Phase", text(manifest, &["status", "phase"]).unwrap_or("Active"), Style::Good)] }],
        _ => Vec::new(),
    };
    sections.extend(specific);
    sections.extend(conditions(manifest));
    sections.extend(events_section(manifest, events));
    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn v(value: serde_json::Value) -> Value {
        serde_yaml::to_value(value).unwrap()
    }

    fn pod() -> Value {
        v(json!({"apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "web-x", "namespace": "shop", "creationTimestamp": "2020-01-01T00:00:00Z", "labels": {"app": "web", "tier": "front"},
                "ownerReferences": [{"kind": "ReplicaSet", "name": "web-5d9d"}]},
            "spec": {"nodeName": "node-1", "serviceAccountName": "web-sa", "containers": [{"name": "nginx", "image": "nginx:1.25", "ports": [{"containerPort": 80}],
                "resources": {"requests": {"cpu": "100m", "memory": "64Mi"}, "limits": {"cpu": "200m"}}, "env": [{"name": "A", "value": "1"}]}]},
            "status": {"phase": "Running", "podIP": "10.0.0.5", "containerStatuses": [{"name": "nginx", "ready": true, "restartCount": 2, "state": {"running": {"startedAt": "2020-01-01T00:00:00Z"}}}],
                "conditions": [{"type": "Ready", "status": "True"}, {"type": "PodScheduled", "status": "True"}]}}))
    }

    fn section<'a>(sections: &'a [Section], title: &str) -> &'a Section {
        sections.iter().find(|s| s.title == title).unwrap_or_else(|| panic!("no {title}: {:?}", sections.iter().map(|s| &s.title).collect::<Vec<_>>()))
    }

    fn label_value(section: &Section, label: &str) -> String {
        section.lines.iter().find_map(|l| match l {
            Line::Field(name, chunks) if name == label => Some(chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" ")),
            _ => None,
        }).unwrap_or_else(|| panic!("no field {label}"))
    }

    #[test]
    fn every_object_gets_its_name_namespace_labels_and_owner() {
        let sections = details(&pod(), &[]);
        let props = section(&sections, "Properties");
        assert_eq!(label_value(props, "Name"), "web-x");
        assert_eq!(label_value(props, "Namespace"), "shop");
        assert_eq!(label_value(props, "Labels"), "app=web tier=front");
        assert_eq!(label_value(props, "Controlled by"), "ReplicaSet web-5d9d");
    }

    #[test]
    fn a_pod_shows_status_node_and_each_container() {
        let sections = details(&pod(), &[]);
        assert_eq!(label_value(section(&sections, "Status"), "Node"), "node-1");
        let containers = section(&sections, "Containers");
        let text: String = containers.lines.iter().map(|l| format!("{l:?}")).collect();
        assert!(text.contains("nginx:1.25") && text.contains("80/TCP") && text.contains("cpu 100m") && text.contains("2 restarts") && text.contains("ready"), "{text}");
    }

    #[test]
    fn conditions_are_toned_and_true_pressure_is_bad() {
        let node = v(json!({"kind": "Node", "metadata": {"name": "n"}, "status": {"conditions": [{"type": "Ready", "status": "True"}, {"type": "MemoryPressure", "status": "True"}]}}));
        let sections = details(&node, &[]);
        let conditions = section(&sections, "Conditions");
        let styles: Vec<Style> = conditions.lines.iter().filter_map(|l| if let Line::Item(c) = l { Some(c[1].style) } else { None }).collect();
        assert_eq!(styles, [Style::Good, Style::Bad]);
    }

    #[test]
    fn secrets_list_key_names_and_never_values() {
        let secret = v(json!({"kind": "Secret", "metadata": {"name": "s"}, "type": "Opaque", "data": {"password": "c2VjcmV0", "user": "YWRtaW4="}}));
        let sections = details(&secret, &[]);
        let all = format!("{sections:?}");
        assert!(all.contains("password") && all.contains("user"));
        assert!(!all.contains("c2VjcmV0") && !all.contains("YWRtaW4="));
    }

    #[test]
    fn config_maps_show_their_values_and_pods_show_their_environment() {
        let map = v(json!({"kind": "ConfigMap", "metadata": {"name": "c"}, "data": {"Corefile": ".:53 {\n  errors\n}", "mode": "fast"}}));
        let text = format!("{:?}", details(&map, &[]));
        assert!(text.contains("errors") && text.contains("fast") && text.contains("Corefile"), "{text}");
        let pod = v(json!({"kind": "Pod", "metadata": {"name": "p"}, "spec": {"containers": [{"name": "c", "image": "i",
            "env": [{"name": "A", "value": "1"}, {"name": "B", "valueFrom": {"secretKeyRef": {"name": "db", "key": "pw"}}}], "envFrom": [{"configMapRef": {"name": "cfg"}}],
            "volumeMounts": [{"name": "data", "mountPath": "/data", "readOnly": true}]}]}}));
        let text = format!("{:?}", details(&pod, &[]));
        assert!(text.contains("A = ") && text.contains("secret db / pw") && text.contains("every key of configMap cfg") && text.contains("/data") && text.contains("read only"), "{text}");
    }

    #[test]
    fn a_service_lists_ports_and_selector() {
        let service = v(json!({"kind": "Service", "metadata": {"name": "web", "namespace": "shop"}, "spec": {"type": "ClusterIP", "clusterIP": "10.0.0.1", "ports": [{"port": 80, "targetPort": 8080, "protocol": "TCP"}], "selector": {"app": "web"}}}));
        let sections = details(&service, &[]);
        let s = section(&sections, "Service");
        assert_eq!(label_value(s, "Port"), "80 → 8080/TCP");
        assert_eq!(label_value(s, "Selector"), "app=web");
    }

    #[test]
    fn events_for_the_object_are_included() {
        let events = vec![EventEntry { message: "Back-off restarting".into(), reason: "BackOff".into(), object: "web-x".into(), kind: "Pod".into(), age: "3m".into(), age_secs: 180, severity: crate::k8s::EventSeverity::Warning }];
        let sections = details(&pod(), &events);
        assert_eq!(section(&sections, "Events").lines.len(), 1);
        assert!(details(&pod(), &[]).iter().all(|s| s.title != "Events"));
    }

    #[test]
    fn an_unknown_kind_still_gets_properties_and_conditions() {
        let widget = v(json!({"kind": "Widget", "apiVersion": "x/v1", "metadata": {"name": "w"}, "status": {"conditions": [{"type": "Ready", "status": "False", "reason": "Broken"}]}}));
        let sections = details(&widget, &[]);
        assert_eq!(sections.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Properties", "Conditions"]);
    }
}
