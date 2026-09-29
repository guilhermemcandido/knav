//! GitOps: reconciliation status across Flux and Argo CD, whichever is installed,
//! plus the events that mention them.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_yaml::Value;

use knav_k8s::EventEntry;
use knav_common::theme::theme;
use knav_common::util::text::truncate;

use super::{Dashboard, DashboardContext};

pub struct GitOps;

/// The Flux kinds shown, in tile order. `HelmRelease` is Flux's own kind, not the
/// Helm releases knav reads from Secrets.
const FLUX_KINDS: &[(&str, &str, &str)] = &[
    ("kustomize.toolkit.fluxcd.io", "Kustomization", "Kustomization"),
    ("helm.toolkit.fluxcd.io", "HelmRelease", "HelmRelease"),
    ("source.toolkit.fluxcd.io", "GitRepository", "GitRepository"),
    ("source.toolkit.fluxcd.io", "OCIRepository", "OCIRepository"),
];

struct FluxTally {
    label: &'static str,
    total: usize,
    ready: usize,
    not_ready: usize,
    suspended: usize,
    unknown: usize,
}

#[derive(Default)]
struct ArgoTally {
    total: usize,
    sync_synced: usize,
    sync_out_of_sync: usize,
    sync_unknown: usize,
    health_healthy: usize,
    health_degraded: usize,
    health_progressing: usize,
    health_other: usize,
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(value, |v, key| v.get(*key))?.as_str()
}

fn is_suspended(manifest: &Value) -> bool {
    manifest.get("spec").and_then(|s| s.get("suspend")).and_then(Value::as_bool).unwrap_or(false)
}

fn ready_status(manifest: &Value) -> Option<&str> {
    manifest.get("status").and_then(|s| s.get("conditions")).and_then(Value::as_sequence).into_iter().flatten().find(|c| text(c, &["type"]) == Some("Ready")).and_then(|c| text(c, &["status"]))
}

fn tally(label: &'static str, objects: &[Value]) -> FluxTally {
    let mut t = FluxTally { label, total: objects.len(), ready: 0, not_ready: 0, suspended: 0, unknown: 0 };
    for object in objects {
        if is_suspended(object) {
            t.suspended += 1;
        } else {
            match ready_status(object) {
                Some("True") => t.ready += 1,
                Some("False") => t.not_ready += 1,
                _ => t.unknown += 1,
            }
        }
    }
    t
}

fn argo_tally(applications: &[Value]) -> ArgoTally {
    let mut t = ArgoTally { total: applications.len(), ..Default::default() };
    for app in applications {
        match text(app, &["status", "sync", "status"]) {
            Some("Synced") => t.sync_synced += 1,
            Some("OutOfSync") => t.sync_out_of_sync += 1,
            _ => t.sync_unknown += 1,
        }
        match text(app, &["status", "health", "status"]) {
            Some("Healthy") => t.health_healthy += 1,
            Some("Degraded") => t.health_degraded += 1,
            Some("Progressing") => t.health_progressing += 1,
            _ => t.health_other += 1,
        }
    }
    t
}

impl Dashboard for GitOps {
    fn category(&self) -> &str {
        "GitOps"
    }

    fn title(&self) -> String {
        "GitOps".into()
    }

    fn lines(&self, ctx: &mut DashboardContext) -> Vec<Line<'static>> {
        let flux: Vec<FluxTally> = FLUX_KINDS.iter().map(|(group, kind, label)| tally(label, &ctx.fetch(group, kind))).collect();
        let argocd = argo_tally(&ctx.fetch("argoproj.io", "Application"));
        let flux_kind_names: Vec<&str> = FLUX_KINDS.iter().map(|(_, _, label)| *label).collect();
        let events: Vec<EventEntry> = ctx.events.iter().filter(|e| flux_kind_names.contains(&e.kind.as_str()) || e.kind == "Application").cloned().collect();
        render(&flux, &argocd, &events)
    }
}

fn kind_tile(status: &FluxTally) -> Vec<Line<'static>> {
    let mut legend = vec![Span::raw(format!("{} total", status.total))];
    if status.ready > 0 {
        legend.push(Span::raw("  "));
        legend.push(Span::styled(format!("● {} ready", status.ready), Style::default().fg(theme().ok)));
    }
    if status.not_ready > 0 {
        legend.push(Span::raw("  "));
        legend.push(Span::styled(format!("● {} not ready", status.not_ready), Style::default().fg(theme().bad)));
    }
    if status.suspended > 0 {
        legend.push(Span::raw("  "));
        legend.push(Span::styled(format!("● {} suspended", status.suspended), Style::default().fg(theme().muted)));
    }
    if status.unknown > 0 {
        legend.push(Span::raw("  "));
        legend.push(Span::styled(format!("● {} unknown", status.unknown), Style::default().fg(theme().warn)));
    }
    vec![Line::styled(status.label, Style::default().add_modifier(Modifier::BOLD)), Line::from(legend)]
}

fn render(flux: &[FluxTally], argocd: &ArgoTally, events: &[EventEntry]) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for status in flux {
        out.extend(kind_tile(status));
        out.push(Line::default());
    }
    if argocd.total > 0 {
        out.push(Line::styled(format!("Application ({} total)", argocd.total), Style::default().add_modifier(Modifier::BOLD)));
        out.push(Line::from(vec![
            Span::raw("sync:   "),
            Span::styled(format!("● {} synced", argocd.sync_synced), Style::default().fg(theme().ok)),
            Span::raw("  "),
            Span::styled(format!("● {} out of sync", argocd.sync_out_of_sync), Style::default().fg(theme().warn)),
            Span::raw("  "),
            Span::styled(format!("● {} unknown", argocd.sync_unknown), Style::default().fg(theme().muted)),
        ]));
        out.push(Line::from(vec![
            Span::raw("health: "),
            Span::styled(format!("● {} healthy", argocd.health_healthy), Style::default().fg(theme().ok)),
            Span::raw("  "),
            Span::styled(format!("● {} degraded", argocd.health_degraded), Style::default().fg(theme().bad)),
            Span::raw("  "),
            Span::styled(format!("● {} progressing", argocd.health_progressing), Style::default().fg(theme().warn)),
            Span::raw("  "),
            Span::styled(format!("● {} other", argocd.health_other), Style::default().fg(theme().muted)),
        ]));
        out.push(Line::default());
    }
    out.push(Line::styled(format!("Events ({})", events.len()), Style::default().add_modifier(Modifier::BOLD)));
    if events.is_empty() {
        out.push(Line::styled("  No events.", Style::default().fg(theme().muted)));
    } else {
        for e in events.iter().take(50) {
            let color = if e.severity == knav_k8s::EventSeverity::Warning { theme().warn } else { theme().ok };
            out.push(Line::from(vec![
                Span::styled(format!("  {:<6}", e.age), Style::default().fg(theme().muted)),
                Span::styled(format!("{:<14}", e.kind), Style::default().fg(color)),
                Span::raw(format!("{:<24}", truncate(&e.object, 23))),
                Span::styled(truncate(&e.message, 80), Style::default().fg(theme().muted)),
            ]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(ready: Option<&str>, suspended: bool) -> Value {
        let conditions = ready.map(|s| serde_json::json!([{"type": "Ready", "status": s}])).unwrap_or(serde_json::json!([]));
        serde_json::from_value(serde_json::json!({"metadata": {"name": "x"}, "spec": {"suspend": suspended}, "status": {"conditions": conditions}})).unwrap()
    }

    fn application(sync: &str, health: &str) -> Value {
        serde_json::from_value(serde_json::json!({"metadata": {"name": "x"}, "status": {"sync": {"status": sync}, "health": {"status": health}}})).unwrap()
    }

    #[test]
    fn tallies_ready_not_ready_and_unknown() {
        let objects = vec![obj(Some("True"), false), obj(Some("False"), false), obj(None, false)];
        let status = tally("Kustomization", &objects);
        assert_eq!(status.total, 3);
        assert_eq!(status.ready, 1);
        assert_eq!(status.not_ready, 1);
        assert_eq!(status.unknown, 1);
    }

    #[test]
    fn a_suspended_object_counts_as_suspended_even_with_a_ready_condition() {
        let status = tally("HelmRelease", &[obj(Some("True"), true)]);
        assert_eq!(status.suspended, 1);
        assert_eq!(status.ready, 0);
    }

    #[test]
    fn argo_applications_tally_sync_and_health_independently() {
        let apps = vec![application("Synced", "Healthy"), application("OutOfSync", "Degraded"), application("Synced", "Progressing")];
        let t = argo_tally(&apps);
        assert_eq!(t.total, 3);
        assert_eq!(t.sync_synced, 2);
        assert_eq!(t.sync_out_of_sync, 1);
        assert_eq!(t.health_healthy, 1);
        assert_eq!(t.health_degraded, 1);
        assert_eq!(t.health_progressing, 1);
    }
}
