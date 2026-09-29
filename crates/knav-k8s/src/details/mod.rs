//! A readable summary of one object, like the drawer of a desktop client: name,
//! namespace, labels, status, containers, conditions and events, instead of raw
//! YAML. It works on the manifest, so it covers every kind; the common kinds
//! get their own sections.

mod cluster;
mod network;
mod pod;
mod rbac;
mod storage;

use serde::Deserialize;
use serde_yaml::Value;

use self::{cluster::*, network::*, pod::*, rbac::*, storage::*};

use crate::EventEntry;
use crate::describe::Tone;

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
    /// A pill worth a second look.
    WarnChip,
    /// A `key=value` pill: the key, the `=` and the value each get their own colour.
    PairChip,
    /// A name that labels a value (a data key, a variable).
    Key,
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

impl Style {
    pub fn is_chip(self) -> bool {
        matches!(self, Style::Chip | Style::WarnChip | Style::PairChip)
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

/// As `at`, but for an extension manifest's dotted path (`.status.notAfter`)
/// instead of a `&[&str]` — the shape `ExtKind.view`'s fields carry.
fn at_dotted<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.trim_start_matches('.').split('.').try_fold(value, |v, key| v.get(key))
}

/// A field's value as one line of text: a scalar as itself, a sequence of
/// scalars comma-joined, anything else left out (an extension's curated
/// fields are meant to be short; a nested object isn't a "field" to show
/// this way).
fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::Sequence(items) => {
            let parts: Vec<String> = items.iter().filter_map(scalar).collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
        other => scalar(other),
    }
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
    pairs.iter().map(|(k, v)| if v.is_empty() { chunk(k.clone(), Style::Chip) } else { chunk(format!("{k}={v}"), Style::PairChip) }).collect()
}

/// The string items of a list.
fn strings<'a>(value: &'a Value, path: &[&str]) -> Vec<&'a str> {
    items(value, path).iter().filter_map(Value::as_str).collect()
}

fn flag(value: &Value, path: &[&str]) -> Option<bool> {
    at(value, path).and_then(Value::as_bool)
}

/// The load balancer addresses of a Service or Ingress status.
fn load_balancer_addresses(manifest: &Value) -> Vec<String> {
    items(manifest, &["status", "loadBalancer", "ingress"]).iter().filter_map(|i| text(i, &["ip"]).or_else(|| text(i, &["hostname"])).map(String::from)).collect()
}

/// A label selector as `key=value` and `key in (a,b)` chips.
fn selector_chips(selector: Option<&Value>) -> Vec<Chunk> {
    let Some(selector) = selector else { return Vec::new() };
    let mut out = chips(&pairs(at(selector, &["matchLabels"])));
    for e in items(selector, &["matchExpressions"]) {
        let values = strings(e, &["values"]).join(",");
        out.push(chunk(format!("{} {} {values}", text(e, &["key"]).unwrap_or("?"), text(e, &["operator"]).unwrap_or("?")).trim().to_string(), Style::Chip));
    }
    out
}

fn age_of(timestamp: &str) -> Option<String> {
    timestamp.parse::<k8s_openapi::jiff::Timestamp>().ok().map(crate::humanize_age)
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
        let label = if at(owner, &["controller"]).and_then(Value::as_bool).unwrap_or(true) { "Controlled by" } else { "Owned by" };
        lines.push(field(label, format!("{} {}", text(owner, &["kind"]).unwrap_or("?"), text(owner, &["name"]).unwrap_or("?"))));
    }
    let finalizers: Vec<&str> = items(manifest, &["metadata", "finalizers"]).iter().filter_map(Value::as_str).collect();
    if !finalizers.is_empty() {
        lines.push(Line::Field("Finalizers".into(), finalizers.into_iter().map(|f| chunk(f, Style::Chip)).collect()));
    }
    let labels = pairs(at(manifest, &["metadata", "labels"]));
    if !labels.is_empty() {
        lines.push(Line::Field("Labels".into(), chips(&labels)));
    }
    let annotations: Vec<(String, String)> = pairs(at(manifest, &["metadata", "annotations"]))
        .into_iter()
        .filter(|(k, _)| k != "kubectl.kubernetes.io/last-applied-configuration")
        .map(|(k, v)| (k, if v.chars().count() > 120 { format!("{}…", v.chars().take(120).collect::<String>()) } else { v }))
        .collect();
    if !annotations.is_empty() {
        lines.push(Line::Field("Annotations".into(), chips(&annotations)));
    }
    Section { title: "Properties".into(), lines }
}

fn events_section(manifest: &Value, events: &[EventEntry]) -> Option<Section> {
    let (kind, name) = (text(manifest, &["kind"])?, text(manifest, &["metadata", "name"])?);
    let namespace = text(manifest, &["metadata", "namespace"]).unwrap_or("");
    let mine: Vec<&EventEntry> = events.iter().filter(|e| e.kind == kind && e.object == name && e.namespace == namespace).take(8).collect();
    if mine.is_empty() {
        return None;
    }
    let lines = mine
        .into_iter()
        .map(|e| {
            let style = if e.severity == crate::EventSeverity::Warning { Style::Warn } else { Style::Muted };
            Line::Item(vec![chunk(format!("{:<6}", e.age), Style::Muted), chunk(format!("{:<18}", e.reason), style), chunk(e.message.clone(), Style::Plain)])
        })
        .collect();
    Some(Section { title: "Events".into(), lines })
}

fn namespace_sections(manifest: &Value) -> Vec<Section> {
    let phase = text(manifest, &["status", "phase"]).unwrap_or("Active");
    vec![Section { title: "Status".into(), lines: vec![field_styled("Phase", phase, crate::describe::phase_tone(phase).into())] }]
}

/// An extension's `KeyValues` view: curated `[label, path]` pairs instead of
/// every field `spec_summary` would otherwise dump. A path that resolves to
/// nothing (or to something that isn't a scalar or a list of them) is left
/// out rather than shown blank; an extension with every field missing falls
/// through to `spec_summary` instead of showing an empty section.
fn key_values_section(manifest: &Value, fields: &[[String; 2]]) -> Option<Section> {
    let lines: Vec<Line> = fields.iter().filter_map(|[label, path]| Some(field(label, scalar_text(at_dotted(manifest, path)?)?))).collect();
    (!lines.is_empty()).then_some(Section { title: "Summary".into(), lines })
}

/// An extension's `Health` view: one field, and whether it matches the value
/// that means "healthy".
fn health_section(manifest: &Value, from: &str, ok: &str) -> Option<Section> {
    let value = scalar_text(at_dotted(manifest, from)?)?;
    let style = if value == ok { Style::Good } else { Style::Bad };
    Some(Section { title: "Health".into(), lines: vec![field_styled("Status", value, style)] })
}

/// Objects with no dedicated view: the plain fields of `spec` and `status`.
fn spec_summary(manifest: &Value) -> Vec<Section> {
    let mut sections = Vec::new();
    for key in ["spec", "status"] {
        let Some(map) = at(manifest, &[key]).and_then(Value::as_mapping) else { continue };
        let mut lines = Vec::new();
        for (name, value) in map {
            let Some(name) = name.as_str() else { continue };
            if name == "conditions" {
                continue;
            }
            let shown = match value {
                Value::Sequence(list) => format!("{} item{}", list.len(), if list.len() == 1 { "" } else { "s" }),
                Value::Mapping(m) => format!("{} field{}", m.len(), if m.len() == 1 { "" } else { "s" }),
                other => scalar(other).unwrap_or_default(),
            };
            let style = if matches!(value, Value::Sequence(_) | Value::Mapping(_)) { Style::Muted } else { Style::Plain };
            lines.push(field_styled(name, shown, style));
        }
        if !lines.is_empty() {
            sections.push(Section { title: if key == "spec" { "Spec".into() } else { "Status".into() }, lines });
        }
    }
    sections
}

/// A view template is a fixed choice, not a rendering instruction: the
/// manifest supplies a field path (and, for `KeyValues`, a label per field),
/// this crate supplies how it's drawn.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "template", rename_all = "snake_case")]
pub enum ViewTemplate {
    /// Reuses the same conditions renderer built-in kinds already have,
    /// which only ever reads `.status.conditions` — `from` isn't read, it's
    /// kept so a manifest still states its assumption in writing.
    Timeline {
        #[allow(dead_code)]
        from: String,
    },
    /// A single field compared against the value that means "healthy".
    Health { from: String, ok: String },
    /// Curated `[label, path]` pairs, in order, shown instead of the generic
    /// spec/status dump — e.g. `["Not After", ".status.notAfter"]`. A path
    /// that resolves to nothing is left out, not shown blank.
    KeyValues { fields: Vec<[String; 2]> },
}

/// The sections that describe `manifest`, with the events that mention it.
/// `custom` is an enabled extension's view for this object's kind, if any
/// (see `extensions::Registry::view_for`) — read only for kinds that fall
/// through to the generic summary; a built-in kind's own dedicated sections
/// below always win, an extension can't be declared for one anyway (its
/// group+kind matches a CRD, never a built-in kind's).
pub fn details(manifest: &Value, events: &[EventEntry], reveal: bool, custom: Option<&ViewTemplate>) -> Vec<Section> {
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
        "ConfigMap" | "Secret" => keys_section(manifest, kind, reveal),
        "PersistentVolumeClaim" | "PersistentVolume" => storage_sections(manifest, kind),
        "Node" => node_sections(manifest),
        "HorizontalPodAutoscaler" => hpa_sections(manifest),
        "Namespace" => namespace_sections(manifest),
        "StorageClass" => storage_class_sections(manifest),
        "Role" | "ClusterRole" => role_sections(manifest, kind),
        "RoleBinding" | "ClusterRoleBinding" => binding_sections(manifest),
        "ServiceAccount" => service_account_sections(manifest),
        "NetworkPolicy" => network_policy_sections(manifest),
        "Endpoints" => endpoints_sections(manifest),
        "EndpointSlice" => endpoint_slice_sections(manifest),
        "PodDisruptionBudget" => pdb_sections(manifest),
        "ResourceQuota" => quota_sections(manifest),
        "LimitRange" => limit_range_sections(manifest),
        "Lease" => lease_sections(manifest),
        "IngressClass" => ingress_class_sections(manifest),
        "CustomResourceDefinition" => crd_sections(manifest),
        _ => match custom {
            Some(ViewTemplate::KeyValues { fields }) => key_values_section(manifest, fields).map(|s| vec![s]).unwrap_or_else(|| spec_summary(manifest)),
            Some(ViewTemplate::Health { from, ok }) => health_section(manifest, from, ok).map(|s| vec![s]).unwrap_or_else(|| spec_summary(manifest)),
            // Timeline just means "this kind's .status.conditions matters", and
            // `conditions()` below already renders that for any kind that has
            // one — nothing extra to add here.
            Some(ViewTemplate::Timeline { .. }) | None => spec_summary(manifest),
        },
    };
    sections.extend(specific);
    sections.extend(conditions(manifest));
    sections.extend(events_section(manifest, events));
    sections
}


#[cfg(test)]
mod tests;
