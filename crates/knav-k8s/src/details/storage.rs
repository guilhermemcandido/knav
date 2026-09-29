use serde_yaml::Value;

use super::*;

/// Values longer than this many lines are cut, saying how many were left.
const VALUE_LINES: usize = 40;

pub(super) fn size_text(bytes: usize) -> String {
    match bytes {
        0..=1023 => format!("{bytes} bytes"),
        _ => format!("{:.1} KB", bytes as f64 / 1024.0),
    }
}

/// A ConfigMap's keys and values, or a Secret's keys and sizes. A Secret's text
/// shows only while `reveal` is on (`x`).
pub(super) fn keys_section(manifest: &Value, kind: &str, reveal: bool) -> Vec<Section> {
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
            // Base64: about three bytes for every four characters.
            let size = if binary { value.len() * 3 / 4 } else { value.len() };
            let shown = if kind == "ConfigMap" && !binary {
                Some(value.to_string())
            } else if kind == "Secret" && reveal && field == "data" {
                decode_text(value)
            } else {
                None
            };
            entries.push((key.to_string(), shown, size));
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
        let mut head = vec![chunk(key.clone(), Style::Key), chunk(format!("   {}", size_text(*size)), Style::Muted)];
        match value {
            Some(value) => {
                lines.push(Line::Item(head));
                let all: Vec<&str> = value.lines().collect();
                for line in all.iter().take(VALUE_LINES) {
                    lines.push(Line::Pad(4, vec![chunk("│ ", Style::Muted), chunk(*line, Style::Plain)]));
                }
                if all.len() > VALUE_LINES {
                    lines.push(Line::Pad(4, vec![chunk("│ ", Style::Muted), chunk(format!("… {} more lines", all.len() - VALUE_LINES), Style::Muted)]));
                }
            }
            None => {
                head.push(chunk(if kind == "Secret" { "   hidden" } else { "   binary" }, Style::Muted));
                lines.push(Line::Item(head));
            }
        }
    }
    let title = match (kind, reveal) {
        ("Secret", true) => "Secret (values shown, x hides them)",
        ("Secret", false) => "Secret (values hidden, x shows them)",
        _ => "Data",
    };
    vec![Section { title: title.into(), lines }]
}

pub(super) fn storage_sections(manifest: &Value, kind: &str) -> Vec<Section> {
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
    for (label, key) in [("Volume mode", "volumeMode")] {
        if let Some(value) = text(manifest, &["spec", key]) {
            lines.push(field(label, value));
        }
    }
    if kind == "PersistentVolume" {
        let source = ["csi", "nfs", "hostPath", "local", "awsElasticBlockStore", "gcePersistentDisk", "azureDisk", "iscsi", "cephfs"].iter().find(|s| at(manifest, &["spec", s]).is_some());
        if let Some(source) = source {
            let detail = text(manifest, &["spec", source, "driver"]).or_else(|| text(manifest, &["spec", source, "server"])).or_else(|| text(manifest, &["spec", source, "path"])).unwrap_or("");
            lines.push(field("Source", format!("{source} {detail}").trim().to_string()));
        }
    }
    vec![Section { title: "Storage".into(), lines }]
}

pub(super) fn storage_class_sections(manifest: &Value) -> Vec<Section> {
    let annotations = |key: &str| text(manifest, &["metadata", "annotations", key]) == Some("true");
    let default = annotations("storageclass.kubernetes.io/is-default-class") || annotations("storageclass.beta.kubernetes.io/is-default-class");
    let mut lines = vec![field("Provisioner", text(manifest, &["provisioner"]).unwrap_or("?")), field("Reclaim policy", text(manifest, &["reclaimPolicy"]).unwrap_or("Delete")), field("Binding mode", text(manifest, &["volumeBindingMode"]).unwrap_or("Immediate")), field("Expansion", if flag(manifest, &["allowVolumeExpansion"]).unwrap_or(false) { "allowed" } else { "not allowed" })];
    if default {
        lines.insert(0, field_styled("Default", "yes", Style::Good));
    }
    let options = strings(manifest, &["mountOptions"]);
    if !options.is_empty() {
        lines.push(Line::Field("Mount options".into(), options.into_iter().map(|o| chunk(o, Style::Chip)).collect()));
    }
    let parameters = pairs(at(manifest, &["parameters"]));
    if !parameters.is_empty() {
        lines.push(Line::Field("Parameters".into(), chips(&parameters)));
    }
    vec![Section { title: "Storage class".into(), lines }]
}

/// A Secret value's text, when it is valid UTF-8 base64.
fn decode_text(encoded: &str) -> Option<String> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    String::from_utf8(STANDARD.decode(encoded.trim()).ok()?).ok()
}
