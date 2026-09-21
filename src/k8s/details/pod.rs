use serde_yaml::Value;

use super::*;

/// One container: its image, state and settings.
pub(super) fn container_lines(spec: &Value, status: Option<&Value>) -> Vec<Line> {
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
    if let Some(done) = at(status.unwrap_or(&Value::Null), &["lastState", "terminated"]) {
        let when = text(done, &["finishedAt"]).and_then(age_of).map(|a| format!(", {a} ago")).unwrap_or_default();
        lines.push(Line::Sub("last exit".into(), vec![chunk(format!("{} (exit {}){when}", text(done, &["reason"]).unwrap_or("Terminated"), number(done, &["exitCode"]).unwrap_or(0)), Style::Warn)]));
    }
    for (label, key) in [("command", "command"), ("args", "args")] {
        let words = strings(spec, &[key]);
        if !words.is_empty() {
            lines.push(Line::Sub(label.into(), vec![chunk(words.join(" "), Style::Plain)]));
        }
    }
    let probes: Vec<&str> = [("livenessProbe", "liveness"), ("readinessProbe", "readiness"), ("startupProbe", "startup")].iter().filter(|(k, _)| at(spec, &[k]).is_some()).map(|(_, l)| *l).collect();
    if !probes.is_empty() {
        lines.push(Line::Sub("probes".into(), vec![chunk(probes.join(", "), Style::Plain)]));
    }
    if at(spec, &["resources", "requests"]).is_none() && at(spec, &["resources", "limits"]).is_none() {
        lines.push(Line::Sub("resources".into(), vec![chunk("no requests or limits", Style::Warn)]));
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
        let mut chunks = vec![chunk(name.to_string(), Style::Key), chunk(" = ", Style::Muted)];
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

pub(super) fn containers(title: &str, specs: &[Value], statuses: &[Value]) -> Option<Section> {
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

pub(super) fn pod_sections(manifest: &Value) -> Vec<Section> {
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
    sections.extend(volumes_section(at(manifest, &["spec"])));
    sections.extend(scheduling_section(manifest));
    sections
}

/// What a volume is backed by, in a few words.
fn volume_source(volume: &Value) -> String {
    let named = |path: &[&str]| text(volume, path).unwrap_or("?").to_string();
    if at(volume, &["persistentVolumeClaim"]).is_some() {
        format!("claim {}", named(&["persistentVolumeClaim", "claimName"]))
    } else if at(volume, &["configMap"]).is_some() {
        format!("configMap {}", named(&["configMap", "name"]))
    } else if at(volume, &["secret"]).is_some() {
        format!("secret {}", named(&["secret", "secretName"]))
    } else if at(volume, &["emptyDir"]).is_some() {
        let medium = text(volume, &["emptyDir", "medium"]).map(|m| format!(" ({m})")).unwrap_or_default();
        format!("emptyDir{medium}")
    } else if at(volume, &["hostPath"]).is_some() {
        format!("hostPath {}", named(&["hostPath", "path"]))
    } else if at(volume, &["projected"]).is_some() {
        "projected".into()
    } else if at(volume, &["csi"]).is_some() {
        format!("csi {}", named(&["csi", "driver"]))
    } else {
        volume.as_mapping().and_then(|m| m.keys().filter_map(Value::as_str).find(|k| *k != "name")).unwrap_or("?").to_string()
    }
}

/// The volumes of a pod spec, or of a workload's pod template.
pub(super) fn volumes_section(pod_spec: Option<&Value>) -> Option<Section> {
    let lines: Vec<Line> = items(pod_spec?, &["volumes"]).iter().map(|v| Line::Item(vec![chunk(format!("{:<28}", text(v, &["name"]).unwrap_or("?")), Style::Plain), chunk(volume_source(v), Style::Muted)])).collect();
    (!lines.is_empty()).then(|| Section { title: "Volumes".into(), lines })
}

fn scheduling_section(manifest: &Value) -> Option<Section> {
    let mut lines = Vec::new();
    let selector = pairs(at(manifest, &["spec", "nodeSelector"]));
    if !selector.is_empty() {
        lines.push(Line::Field("Node selector".into(), chips(&selector)));
    }
    let tolerations: Vec<Chunk> = items(manifest, &["spec", "tolerations"])
        .iter()
        .map(|t| {
            let key = text(t, &["key"]).unwrap_or("(any)");
            let effect = text(t, &["effect"]).map(|e| format!(":{e}")).unwrap_or_default();
            chunk(format!("{key}{effect}"), Style::Chip)
        })
        .collect();
    if !tolerations.is_empty() {
        lines.push(Line::Field("Tolerations".into(), tolerations));
    }
    (!lines.is_empty()).then(|| Section { title: "Scheduling".into(), lines })
}

/// Deployments, ReplicaSets, StatefulSets, DaemonSets.
pub(super) fn workload_sections(manifest: &Value, kind: &str) -> Vec<Section> {
    let mut lines = Vec::new();
    let desired = number(manifest, &["spec", "replicas"]).unwrap_or(if kind == "DaemonSet" { number(manifest, &["status", "desiredNumberScheduled"]).unwrap_or(0) } else { 1 });
    let ready = number(manifest, &["status", if kind == "DaemonSet" { "numberReady" } else { "readyReplicas" }]).unwrap_or(0);
    lines.push(field_styled("Ready", format!("{ready}/{desired}"), crate::k8s::describe::ready_tone(ready, desired).into()));
    let daemon = kind == "DaemonSet";
    let counters = [("Updated", if daemon { "updatedNumberScheduled" } else { "updatedReplicas" }), ("Available", if daemon { "numberAvailable" } else { "availableReplicas" }), ("Unavailable", if daemon { "numberUnavailable" } else { "unavailableReplicas" }), ("Misscheduled", "numberMisscheduled")];
    for (label, key) in counters {
        if let Some(n) = number(manifest, &["status", key]) {
            lines.push(field_styled(label, n.to_string(), if n > 0 && matches!(label, "Unavailable" | "Misscheduled") { Style::Warn } else { Style::Plain }));
        }
    }
    let rolling = |key: &str| at(manifest, &["spec", "strategy", "rollingUpdate", key]).or_else(|| at(manifest, &["spec", "updateStrategy", "rollingUpdate", key])).and_then(scalar);
    if let (Some(surge), Some(unavailable)) = (rolling("maxSurge"), rolling("maxUnavailable")) {
        lines.push(field("Rollout", format!("max surge {surge}, max unavailable {unavailable}")));
    }
    if flag(manifest, &["spec", "paused"]).unwrap_or(false) {
        lines.push(field_styled("Paused", "yes", Style::Warn));
    }
    if let Some(service) = text(manifest, &["spec", "serviceName"]) {
        lines.push(field("Service", service));
    }
    if let Some(strategy) = text(manifest, &["spec", "strategy", "type"]).or_else(|| text(manifest, &["spec", "updateStrategy", "type"])) {
        lines.push(field("Strategy", strategy));
    }
    let selector = pairs(at(manifest, &["spec", "selector", "matchLabels"]));
    if !selector.is_empty() {
        lines.push(Line::Field("Selector".into(), chips(&selector)));
    }
    let mut sections = vec![Section { title: "Status".into(), lines }];
    let pod_spec = ["spec", "template", "spec"];
    sections.extend(containers("Containers", items(manifest, &["spec", "template", "spec", "containers"]), &[]));
    sections.extend(containers("Init containers", items(manifest, &["spec", "template", "spec", "initContainers"]), &[]));
    sections.extend(volumes_section(at(manifest, &pod_spec)));
    sections
}

pub(super) fn job_sections(manifest: &Value, kind: &str) -> Vec<Section> {
    let mut lines = Vec::new();
    if kind == "CronJob" {
        lines.push(field("Schedule", text(manifest, &["spec", "schedule"]).unwrap_or("?")));
        let suspended = at(manifest, &["spec", "suspend"]).and_then(Value::as_bool).unwrap_or(false);
        lines.push(field_styled("Suspended", if suspended { "yes" } else { "no" }, if suspended { Style::Warn } else { Style::Plain }));
        if let Some(last) = text(manifest, &["status", "lastScheduleTime"]) {
            lines.push(field("Last run", format!("{} ago", age_of(last).unwrap_or_else(|| "?".into()))));
        }
        if let Some(policy) = text(manifest, &["spec", "concurrencyPolicy"]) {
            lines.push(field("Concurrency", policy));
        }
        for (label, key) in [("Keeps successful", "successfulJobsHistoryLimit"), ("Keeps failed", "failedJobsHistoryLimit")] {
            if let Some(n) = number(manifest, &["spec", key]) {
                lines.push(field(label, n.to_string()));
            }
        }
        lines.push(field("Active", items(manifest, &["status", "active"]).len().to_string()));
        return vec![
            Section { title: "Schedule".into(), lines },
        ]
        .into_iter()
        .chain(containers("Containers", items(manifest, &["spec", "jobTemplate", "spec", "template", "spec", "containers"]), &[]))
        .collect();
    }
    if flag(manifest, &["spec", "suspend"]).unwrap_or(false) {
        lines.push(field_styled("Suspended", "yes", Style::Warn));
    }
    let wanted = number(manifest, &["spec", "completions"]).unwrap_or(1);
    let done = number(manifest, &["status", "succeeded"]).unwrap_or(0);
    lines.push(field_styled("Completions", format!("{done}/{wanted}"), if done >= wanted { Style::Good } else { Style::Warn }));
    for (label, key) in [("Parallelism", "parallelism"), ("Backoff limit", "backoffLimit")] {
        if let Some(n) = number(manifest, &["spec", key]) {
            lines.push(field(label, n.to_string()));
        }
    }
    for (label, key) in [("Active", "active"), ("Failed", "failed")] {
        if let Some(n) = number(manifest, &["status", key]) {
            lines.push(field_styled(label, n.to_string(), if label == "Failed" && n > 0 { Style::Bad } else { Style::Plain }));
        }
    }
    vec![Section { title: "Status".into(), lines }].into_iter().chain(containers("Containers", items(manifest, &["spec", "template", "spec", "containers"]), &[])).collect()
}
