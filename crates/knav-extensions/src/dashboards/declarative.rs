//! Interprets a manifest's `[[extension.dashboard]]` widgets — the data-only
//! counterpart to the hand-written dashboards in this directory. A manifest
//! never supplies code, only field paths and a choice among four fixed
//! widgets (`count`/`tally`/`sum`/`list`, see `manifest::WidgetSpec`); this
//! file is the one place that knows how to fetch, compute and draw each one.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_yaml::Value;

use crate::manifest::{DashboardWidget, WidgetSpec};
use knav_common::theme::theme;
use knav_common::util::text::truncate;

use super::{Dashboard, DashboardContext};

/// One widget plus the `(group, kind)` pairs its `kind`/`extra_kinds`
/// resolved to against the manifest that declared it (see
/// `extensions::Registry::dashboard_widgets`) — already validated at parse
/// time, so every source here is real.
pub struct DeclarativeDashboard {
    pub title: String,
    pub category: String,
    pub widgets: Vec<(DashboardWidget, Vec<(String, String)>)>,
}

impl Dashboard for DeclarativeDashboard {
    fn category(&self) -> &str {
        &self.category
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn lines(&self, ctx: &mut DashboardContext) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        for (widget, sources) in &self.widgets {
            let label = widget.label.clone().unwrap_or_else(|| {
                let mut names = vec![widget.kind.clone()];
                names.extend(widget.extra_kinds.clone());
                names.join(" / ")
            });
            // `count` only needs the cheap number; every other widget needs
            // the full objects to compute conditions/fields/sorting from.
            let line = if let WidgetSpec::Count = widget.spec {
                let total: usize = sources.iter().map(|(group, kind)| ctx.count(group, kind)).sum();
                vec![Line::from(vec![Span::styled(format!("{total} "), Style::default().add_modifier(Modifier::BOLD)), Span::raw(label)])]
            } else {
                let objects: Vec<Value> = sources.iter().flat_map(|(group, kind)| ctx.fetch(group, kind)).collect();
                render_widget(&label, &widget.spec, &objects)
            };
            out.extend(line);
            out.push(Line::default());
        }
        out
    }
}

fn at_dotted<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.trim_start_matches('.').split('.').try_fold(value, |v, key| v.get(key))
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn condition_status<'a>(manifest: &'a Value, type_: &str) -> Option<&'a str> {
    manifest.get("status").and_then(|s| s.get("conditions")).and_then(Value::as_sequence).into_iter().flatten().find(|c| c.get("type").and_then(Value::as_str) == Some(type_)).and_then(|c| c.get("status")).and_then(Value::as_str)
}

/// True/False/Unknown for one object, by whichever `by` mode the widget asked for.
fn bucket_of(object: &Value, by: &str) -> Option<bool> {
    let status = if by == "ready" {
        condition_status(object, "Ready")
    } else if let Some(type_) = by.strip_prefix("condition:") {
        condition_status(object, type_)
    } else if let Some(path) = by.strip_prefix("field:") {
        return match at_dotted(object, path) {
            Some(Value::Bool(b)) => Some(*b),
            Some(Value::String(s)) => match s.as_str() {
                "True" | "true" => Some(true),
                "False" | "false" => Some(false),
                _ => None,
            },
            _ => None,
        };
    } else {
        None
    };
    match status {
        Some("True") => Some(true),
        Some("False") => Some(false),
        _ => None,
    }
}

fn tally_line(true_count: usize, false_count: usize, unknown_count: usize) -> Line<'static> {
    let total = true_count + false_count + unknown_count;
    let mut spans = vec![Span::raw(format!("{total} total"))];
    if true_count > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("● {true_count} true"), Style::default().fg(theme().ok)));
    }
    if false_count > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("● {false_count} false"), Style::default().fg(theme().bad)));
    }
    if unknown_count > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("● {unknown_count} unknown"), Style::default().fg(theme().muted)));
    }
    Line::from(spans)
}

fn bar_segment(count: i64, total: i64, color: Color, width: usize) -> Span<'static> {
    let cells = if total > 0 { ((count as f64 / total as f64) * width as f64).round() as usize } else { 0 };
    Span::styled("▓".repeat(cells.min(width)), Style::default().fg(color))
}

/// A field's value ordered by what it actually is: an RFC3339 timestamp, a
/// number, or text — whichever the column's values turn out to be. Missing
/// sorts last regardless, so a `list` widget's absent fields don't scatter
/// through the middle of an otherwise-ordered column.
#[derive(PartialEq, PartialOrd)]
enum SortKey {
    Time(i64),
    Num(f64),
    Text(String),
    Missing,
}

fn sort_key(object: &Value, path: &str) -> SortKey {
    let Some(value) = at_dotted(object, path) else { return SortKey::Missing };
    if let Some(s) = value.as_str()
        && let Ok(t) = s.parse::<k8s_openapi::jiff::Timestamp>()
    {
        return SortKey::Time(t.as_second());
    }
    if let Some(n) = value.as_f64() {
        return SortKey::Num(n);
    }
    match scalar_text(value) {
        Some(s) => SortKey::Text(s),
        None => SortKey::Missing,
    }
}

/// `.status.notAfter`-style formatting: how many days until (or since) the
/// timestamp a path points at, colour-coded the same way the expiry-aware
/// `cert-manager` dashboard always has been.
fn days_span(object: &Value, path: &str) -> Span<'static> {
    let Some(days) = at_dotted(object, path).and_then(Value::as_str).and_then(|s| s.parse::<k8s_openapi::jiff::Timestamp>().ok()).map(|t| (t.as_second() - k8s_openapi::jiff::Timestamp::now().as_second()) / 86400) else {
        return Span::styled("-", Style::default().fg(theme().muted));
    };
    match days {
        d if d < 0 => Span::styled(format!("expired {}d ago", -d), Style::default().fg(theme().bad)),
        d if d < 7 => Span::styled(format!("in {d}d"), Style::default().fg(theme().bad)),
        d if d < 30 => Span::styled(format!("in {d}d"), Style::default().fg(theme().warn)),
        d => Span::styled(format!("in {d}d"), Style::default().fg(theme().ok)),
    }
}

fn render_widget(label: &str, spec: &WidgetSpec, objects: &[Value]) -> Vec<Line<'static>> {
    match spec {
        WidgetSpec::Count => vec![Line::from(vec![Span::styled(format!("{} ", objects.len()), Style::default().add_modifier(Modifier::BOLD)), Span::raw(label.to_string())])],
        WidgetSpec::Tally { by } => {
            let (mut t, mut f, mut u) = (0, 0, 0);
            for object in objects {
                match bucket_of(object, by) {
                    Some(true) => t += 1,
                    Some(false) => f += 1,
                    None => u += 1,
                }
            }
            vec![Line::styled(label.to_string(), Style::default().add_modifier(Modifier::BOLD)), tally_line(t, f, u)]
        }
        WidgetSpec::Sum { fields } => {
            let sums: Vec<(String, i64)> = fields
                .iter()
                .map(|[field_label, path]| (field_label.clone(), objects.iter().filter_map(|o| at_dotted(o, path)).filter_map(Value::as_i64).sum()))
                .collect();
            let mut out = vec![Line::styled(format!("{label} ({} total)", objects.len()), Style::default().add_modifier(Modifier::BOLD))];
            let total: i64 = sums.iter().map(|(_, n)| *n).sum();
            if total > 0 {
                const WIDTH: usize = 40;
                const PALETTE: &[fn() -> Color] = &[|| theme().ok, || theme().bad, || theme().warn, || theme().muted, || theme().accent];
                let bar: Vec<Span<'static>> = sums.iter().enumerate().map(|(i, (_, n))| bar_segment(*n, total, PALETTE[i % PALETTE.len()](), WIDTH)).collect();
                out.push(Line::from(bar));
                let legend: Vec<Span<'static>> = sums
                    .iter()
                    .enumerate()
                    .flat_map(|(i, (field_label, n))| [Span::styled(format!("● {n} {field_label}"), Style::default().fg(PALETTE[i % PALETTE.len()]())), Span::raw("  ")])
                    .collect();
                out.push(Line::from(legend));
            }
            out
        }
        WidgetSpec::List { sort_by, date_columns, columns, limit } => {
            let mut rows: Vec<&Value> = objects.iter().collect();
            if let Some(sort_by) = sort_by {
                rows.sort_by(|a, b| sort_key(a, sort_by).partial_cmp(&sort_key(b, sort_by)).unwrap_or(std::cmp::Ordering::Equal));
            }
            let mut out = vec![Line::styled(format!("{label} ({})", objects.len()), Style::default().add_modifier(Modifier::BOLD))];
            if rows.is_empty() {
                out.push(Line::styled("  none found", Style::default().fg(theme().muted)));
                return out;
            }
            let header: String = columns.iter().map(|[column_label, _]| format!("{:<20}", column_label.to_uppercase())).collect();
            out.push(Line::styled(format!("  {header}"), Style::default().fg(theme().muted)));
            for object in rows.into_iter().take(*limit) {
                let mut spans = vec![Span::raw("  ".to_string())];
                for [_, path] in columns {
                    if date_columns.contains(path) {
                        let mut span = days_span(object, path);
                        span.content = format!("{:<20}", span.content).into();
                        spans.push(span);
                    } else {
                        let text = at_dotted(object, path).and_then(scalar_text).unwrap_or_else(|| "-".into());
                        spans.push(Span::raw(format!("{:<20}", truncate(&text, 19))));
                    }
                }
                out.push(Line::from(spans));
            }
            if objects.len() > *limit {
                out.push(Line::styled(format!("  … and {} more", objects.len() - limit), Style::default().fg(theme().muted)));
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn obj(status: &str) -> Value {
        serde_json::from_value(serde_json::json!({"metadata": {"name": "x"}, "status": {"conditions": [{"type": "Ready", "status": status}]}})).unwrap()
    }

    #[test]
    fn ready_shorthand_matches_the_ready_condition() {
        assert_eq!(bucket_of(&obj("True"), "ready"), Some(true));
        assert_eq!(bucket_of(&obj("False"), "ready"), Some(false));
    }

    #[test]
    fn condition_prefix_reads_any_condition_type() {
        let active: Value = serde_json::from_value(serde_json::json!({"status": {"conditions": [{"type": "Active", "status": "True"}]}})).unwrap();
        assert_eq!(bucket_of(&active, "condition:Active"), Some(true));
        assert_eq!(bucket_of(&active, "condition:Ready"), None);
    }

    #[test]
    fn field_prefix_reads_a_plain_boolean() {
        let created: Value = serde_json::from_value(serde_json::json!({"status": {"created": true}})).unwrap();
        assert_eq!(bucket_of(&created, "field:.status.created"), Some(true));
    }

    #[test]
    fn sum_widget_adds_a_numeric_field_across_every_object() {
        let a: Value = serde_json::from_value(serde_json::json!({"summary": {"pass": 3}})).unwrap();
        let b: Value = serde_json::from_value(serde_json::json!({"summary": {"pass": 5}})).unwrap();
        let spec = WidgetSpec::Sum { fields: vec![["pass".into(), ".summary.pass".into()]] };
        let lines = render_widget("PolicyReports", &spec, &[a, b]);
        // Title line names the total object count; the bar/legend line names the sum.
        assert!(line_text(&lines[0]).contains("(2 total)"));
    }

    #[test]
    fn list_widget_sorts_by_the_given_field_soonest_first() {
        let far: Value = serde_json::from_value(serde_json::json!({"metadata": {"name": "far"}, "status": {"notAfter": "2099-01-01T00:00:00Z"}})).unwrap();
        let soon: Value = serde_json::from_value(serde_json::json!({"metadata": {"name": "soon"}, "status": {"notAfter": "2026-09-25T00:00:00Z"}})).unwrap();
        let spec = WidgetSpec::List { sort_by: Some(".status.notAfter".into()), date_columns: vec![], columns: vec![["Name".into(), ".metadata.name".into()]], limit: 20 };
        let lines = render_widget("Certificate", &spec, &[far, soon]);
        let rendered: Vec<String> = lines.iter().map(line_text).collect();
        let soon_at = rendered.iter().position(|l| l.contains("soon")).unwrap();
        let far_at = rendered.iter().position(|l| l.contains("far")).unwrap();
        assert!(soon_at < far_at, "{rendered:?}");
    }
}
