//! What a pod or node uses right now, from metrics-server, against what it asks for,
//! its limits, or what the node can give.

use serde_yaml::Value;

use super::{Chunk, Line, Section, Style, VALUE_COLUMN, chunk};
use crate::metrics::{ClusterUsage, PodUsageMap, parse_cpu_millicores, parse_memory_bytes};

/// The usage the info view can show, when metrics-server answered.
#[derive(Clone, Copy, Default)]
pub struct Usage<'a> {
    pub pods: Option<&'a PodUsageMap>,
    pub nodes: Option<&'a ClusterUsage>,
}

fn cpu(m: i64) -> String {
    format!("{m}m")
}

fn memory(bytes: i64) -> String {
    if bytes >= 1024 * 1024 { format!("{}Mi", bytes / (1024 * 1024)) } else { format!("{}Ki", bytes / 1024) }
}

const BAR: usize = 16;

/// Orange from 70% of the ceiling, red from 90%.
fn tone(percent: i64) -> Style {
    if percent >= 90 { Style::Bad } else if percent >= 70 { Style::Warn } else { Style::Good }
}

/// A bar of `used` against `scale` (what its right end stands for), with a tick at
/// `tick`, then two cells of space.
fn bar(used: i64, scale: i64, tick: Option<i64>, fill: Style) -> Vec<Chunk> {
    let cells = |v: i64| ((v as f64 / scale.max(1) as f64) * BAR as f64).round() as usize;
    // Anything in use shows at least a sliver.
    let filled = if used > 0 { cells(used).clamp(1, BAR) } else { 0 };
    let tick = tick.map(cells).filter(|t| *t > filled && *t < BAR);
    let rest: String = (filled..BAR).map(|i| if Some(i) == tick { '│' } else { '░' }).collect();
    vec![chunk("█".repeat(filled), fill), chunk(rest, Style::Muted), chunk("  ", Style::Plain)]
}

/// A resource as two lines: the bar and what is used, then what it is measured
/// against in grey under the value, so a narrow panel doesn't cut it off.
fn two_lines(label: &str, (first, second): (Vec<Chunk>, Vec<Chunk>)) -> [Line; 2] {
    [Line::Field(label.into(), first), Line::Pad(VALUE_COLUMN, second)]
}

/// A pod's use of one resource: a bar to its limit (or request) and the amount, then
/// `of 170Mi limit (15%), 70Mi requested`.
fn pod_line(used: i64, request: Option<i64>, limit: Option<i64>, format: fn(i64) -> String) -> (Vec<Chunk>, Vec<Chunk>) {
    let (request, limit) = (request.filter(|r| *r > 0), limit.filter(|l| *l > 0));
    let percent = |of: i64| used * 100 / of.max(1);
    let mut chunks = match (request, limit) {
        (_, Some(limit)) => bar(used, limit, request, tone(percent(limit))),
        // Past the request is fine without a limit, but worth seeing.
        (Some(request), None) => bar(used, request, None, if used > request { Style::Warn } else { Style::Good }),
        (None, None) => Vec::new(),
    };
    chunks.push(chunk(format(used), Style::Strong));
    let note = match (request, limit) {
        (Some(request), Some(limit)) => format!("of {} limit ({}%), {} requested", format(limit), percent(limit), format(request)),
        (None, Some(limit)) => format!("of {} limit ({}%)", format(limit), percent(limit)),
        (Some(request), None) => format!("of {} requested ({}%), no limit", format(request), percent(request)),
        (None, None) => "no request or limit".into(),
    };
    (chunks, vec![chunk(note, Style::Muted)])
}

/// A node's use of one resource against what it can give to pods.
fn node_line(used: i64, allocatable: Option<i64>, format: fn(i64) -> String) -> (Vec<Chunk>, Vec<Chunk>) {
    let Some(of) = allocatable.filter(|a| *a > 0) else { return (vec![chunk(format(used), Style::Strong)], Vec::new()) };
    let percent = used * 100 / of;
    let mut chunks = bar(used, of, None, tone(percent));
    chunks.push(chunk(format(used), Style::Strong));
    (chunks, vec![chunk(format!("of {} allocatable ({percent}%)", format(of)), Style::Muted)])
}

/// Each container's `resources.<which>.<key>`, summed. `None` when no container sets
/// it, or for limits when any container lacks one, since then there is no ceiling.
fn pod_total(manifest: &Value, which: &str, key: &str, parse: fn(&str) -> i64) -> Option<i64> {
    let containers = manifest.get("spec")?.get("containers")?.as_sequence()?;
    // Quantities are strings (`500m`) or plain YAML numbers (`1`, `0.5`).
    let quantity = |v: &Value| v.as_str().map(str::to_string).or_else(|| v.as_f64().map(|n| n.to_string()));
    let values: Vec<Option<i64>> = containers.iter().map(|c| c.get("resources")?.get(which)?.get(key).and_then(quantity).map(|q| parse(&q))).collect();
    if which == "limits" { values.into_iter().sum() } else { values.into_iter().flatten().reduce(|a, b| a + b) }
}

pub(super) fn section(kind: &str, manifest: &Value, usage: Usage) -> Option<Section> {
    let name = manifest.get("metadata")?.get("name")?.as_str()?;
    let lines = match kind {
        "Pod" => {
            let namespace = manifest.get("metadata")?.get("namespace")?.as_str()?;
            let used = usage.pods?.get(namespace, name)?;
            let cpu_line = pod_line(used.cpu_millicores, pod_total(manifest, "requests", "cpu", parse_cpu_millicores), pod_total(manifest, "limits", "cpu", parse_cpu_millicores), cpu);
            let memory_line = pod_line(used.memory_bytes, pod_total(manifest, "requests", "memory", parse_memory_bytes), pod_total(manifest, "limits", "memory", parse_memory_bytes), memory);
            two_lines("CPU", cpu_line).into_iter().chain(two_lines("Memory", memory_line)).collect()
        }
        "Node" => {
            let used = usage.nodes?.for_node(name)?;
            let allocatable = |key: &str, parse: fn(&str) -> i64| manifest.get("status")?.get("allocatable")?.get(key)?.as_str().map(parse);
            let cpu_line = node_line(used.cpu_millicores, allocatable("cpu", parse_cpu_millicores), cpu);
            let memory_line = node_line(used.memory_bytes, allocatable("memory", parse_memory_bytes), memory);
            two_lines("CPU", cpu_line).into_iter().chain(two_lines("Memory", memory_line)).collect()
        }
        _ => return None,
    };
    Some(Section { title: "Usage".into(), lines })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        match line {
            Line::Field(_, chunks) | Line::Pad(_, chunks) => chunks.iter().map(|c| c.text.as_str()).collect(),
            _ => String::new(),
        }
    }

    fn joined((first, second): (Vec<Chunk>, Vec<Chunk>)) -> String {
        first.iter().chain(&second).map(|c| c.text.as_str()).collect::<Vec<_>>().join("|")
    }

    #[test]
    fn a_pod_shows_its_usage_against_requests_and_limits() {
        let manifest: Value = serde_yaml::from_str(
            "kind: Pod\nmetadata: {name: web, namespace: shop}\nspec:\n  containers:\n  - resources: {requests: {cpu: 100m, memory: 64Mi}, limits: {cpu: 200m, memory: 128Mi}}\n",
        )
        .unwrap();
        let mut pods = PodUsageMap::default();
        pods.insert("shop", "web", crate::metrics::PodUsage { cpu_millicores: 50, memory_bytes: 120 * 1024 * 1024 });
        let section = section("Pod", &manifest, Usage { pods: Some(&pods), nodes: None }).unwrap();
        // A quarter of the limit, with the request tick at the halfway mark.
        assert_eq!(text(&section.lines[0]), "████░░░░│░░░░░░░  50m");
        assert_eq!(text(&section.lines[1]), "of 200m limit (25%), 100m requested");
        assert_eq!(text(&section.lines[2]), "███████████████░  120Mi");
        assert_eq!(text(&section.lines[3]), "of 128Mi limit (93%), 64Mi requested");
    }

    #[test]
    fn without_a_limit_the_bar_runs_to_the_request() {
        assert_eq!(joined(pod_line(25, Some(100), None, cpu)), "████|░░░░░░░░░░░░|  |25m|of 100m requested (25%), no limit");
        assert_eq!(joined(pod_line(25, None, None, cpu)), "25m|no request or limit");
    }

    #[test]
    fn no_metrics_means_no_section() {
        let manifest: Value = serde_yaml::from_str("kind: Pod\nmetadata: {name: web, namespace: shop}\n").unwrap();
        assert!(section("Pod", &manifest, Usage::default()).is_none());
    }
}
