//! GitOps: reconciliation status across Flux and Argo CD, whichever is installed,
//! plus the events that mention them.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_yaml::Value;

use knav_k8s::EventEntry;
use knav_common::theme::theme;
use knav_common::util::text::truncate;

use super::kit::{Slice, card, columns, grid, legend, sparkline, stacked};
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
        render(&flux, &argocd, &events, ctx.width)
    }
}

/// The five-minute buckets of the last hour, oldest first, counting events about `kind`.
fn activity(events: &[EventEntry], kind: &str) -> Vec<usize> {
    const BUCKETS: usize = 12;
    let mut counts = vec![0; BUCKETS];
    for e in events.iter().filter(|e| e.kind == kind && (0..3600).contains(&e.age_secs)) {
        counts[BUCKETS - 1 - (e.age_secs as usize / 300).min(BUCKETS - 1)] += 1;
    }
    counts
}

fn headline(count: usize, total: usize, word: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{count}"), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" / {total}"), Style::default().fg(theme().muted)),
        Span::styled(format!("  {word}"), Style::default().fg(theme().muted)),
    ])
}

fn flux_card(status: &FluxTally, events: &[EventEntry], width: usize) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(4);
    let color = if status.not_ready > 0 { theme().bad } else if status.ready == status.total { theme().ok } else { theme().warn };
    let slices = [Slice::new(status.ready, theme().ok), Slice::new(status.not_ready, theme().bad), Slice::new(status.suspended, theme().muted), Slice::new(status.unknown, theme().warn)];
    let mut body = vec![
        headline(status.ready, status.total, "ready", color),
        Line::from(stacked(inner, &slices)),
        legend(&[(status.ready, "ready", theme().ok), (status.not_ready, "failing", theme().bad), (status.suspended, "suspended", theme().muted), (status.unknown, "unknown", theme().warn)]),
    ];
    let recent = activity(events, status.label);
    if recent.iter().any(|n| *n > 0) {
        let mut spans = sparkline(&recent, theme().accent);
        spans.push(Span::styled("  last hour", Style::default().fg(theme().muted)));
        body.push(Line::from(spans));
    }
    card(status.label, width, body)
}

fn argo_sync(argocd: &ArgoTally, width: usize) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(4);
    card(
        "Argo CD sync",
        width,
        vec![
            headline(argocd.sync_synced, argocd.total, "synced", if argocd.sync_out_of_sync > 0 { theme().warn } else { theme().ok }),
            Line::from(stacked(inner, &[Slice::new(argocd.sync_synced, theme().ok), Slice::new(argocd.sync_out_of_sync, theme().warn), Slice::new(argocd.sync_unknown, theme().muted)])),
            legend(&[(argocd.sync_synced, "synced", theme().ok), (argocd.sync_out_of_sync, "out of sync", theme().warn), (argocd.sync_unknown, "unknown", theme().muted)]),
        ],
    )
}

fn argo_health(argocd: &ArgoTally, width: usize) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(4);
    card(
        "Argo CD health",
        width,
        vec![
            headline(argocd.health_healthy, argocd.total, "healthy", if argocd.health_degraded > 0 { theme().bad } else { theme().ok }),
            Line::from(stacked(
                inner,
                &[Slice::new(argocd.health_healthy, theme().ok), Slice::new(argocd.health_progressing, theme().warn), Slice::new(argocd.health_degraded, theme().bad), Slice::new(argocd.health_other, theme().muted)],
            )),
            legend(&[(argocd.health_healthy, "healthy", theme().ok), (argocd.health_progressing, "progressing", theme().warn), (argocd.health_degraded, "degraded", theme().bad), (argocd.health_other, "other", theme().muted)]),
        ],
    )
}

fn render(flux: &[FluxTally], argocd: &ArgoTally, events: &[EventEntry], width: usize) -> Vec<Line<'static>> {
    const MIN_CARD: usize = 34;
    let mut out = vec![Line::default()];
    let shown: Vec<&FluxTally> = flux.iter().filter(|f| f.total > 0).collect();
    if shown.is_empty() && argocd.total == 0 {
        out.push(Line::styled("  Neither Flux nor Argo CD has any objects in this cluster.", Style::default().fg(theme().muted)));
        out.push(Line::default());
    }
    out.extend(grid(width, MIN_CARD, &shown, |status, w| flux_card(status, events, w)));
    if argocd.total > 0 {
        out.push(Line::default());
        let (cols, each) = columns(width, MIN_CARD);
        // Two cards side by side when they fit, stacked otherwise.
        if cols >= 2 {
            out.extend(grid(width, MIN_CARD, &[0, 1], |i, w| if *i == 0 { argo_sync(argocd, w) } else { argo_health(argocd, w) }));
        } else {
            out.extend(argo_sync(argocd, each));
            out.extend(argo_health(argocd, each));
        }
    }
    out.push(Line::default());
    out.push(Line::styled(format!(" Recent changes ({})", events.len()), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)));
    if events.is_empty() {
        out.push(Line::styled("  No events.", Style::default().fg(theme().muted)));
    } else {
        // The object and message share what the age and kind leave, and a very narrow
        // screen drops the message.
        let left = width.saturating_sub(4 + 6 + 15);
        let object_room = (left / 3).clamp(8, 25);
        let message_room = left.saturating_sub(object_room + 1);
        for e in events.iter().take(50) {
            let color = if e.severity == knav_k8s::EventSeverity::Warning { theme().warn } else { theme().ok };
            out.push(Line::from(vec![
                Span::styled("  ● ", Style::default().fg(color)),
                Span::styled(format!("{:<6}", e.age), Style::default().fg(theme().muted)),
                Span::styled(format!("{:<15}", truncate(&e.kind, 14)), Style::default().fg(color)),
                Span::raw(format!("{:<w$}", truncate(&e.object, object_room - 1), w = object_room)),
                Span::styled(if message_room >= 8 { truncate(&e.message, message_room) } else { String::new() }, Style::default().fg(theme().muted)),
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

    #[test]
    fn the_dashboard_lays_out_cards_that_fit_the_width() {
        let tallies = vec![
            FluxTally { label: "Kustomization", total: 14, ready: 12, not_ready: 2, suspended: 0, unknown: 0 },
            FluxTally { label: "HelmRelease", total: 8, ready: 8, not_ready: 0, suspended: 0, unknown: 0 },
            FluxTally { label: "GitRepository", total: 0, ready: 0, not_ready: 0, suspended: 0, unknown: 0 },
        ];
        let argo = ArgoTally { total: 12, sync_synced: 9, sync_out_of_sync: 3, health_healthy: 7, health_degraded: 2, health_progressing: 3, ..Default::default() };
        let events = vec![EventEntry { message: "ReconciliationSucceeded".into(), reason: "x".into(), object: "apps".into(), namespace: "flux".into(), kind: "Kustomization".into(), age: "2m".into(), age_secs: 120, severity: knav_k8s::EventSeverity::Normal }];
        for width in [40, 80, 120] {
            let lines = render(&tallies, &argo, &events, width);
            for line in &lines {
                assert!(line.width() <= width, "{width}: {}", line.spans.iter().map(|s| s.content.as_ref()).collect::<String>());
            }
            if width == 80 {
                for line in &lines {
                    println!("{}", line.spans.iter().map(|s| s.content.as_ref()).collect::<String>());
                }
            }
        }
    }
}
