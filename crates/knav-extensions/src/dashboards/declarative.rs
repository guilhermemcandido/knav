//! Draws a manifest's `[[extension.dashboard]]` widgets: the one place that knows how
//! to fetch, compute and draw each fixed widget shape.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_yaml::Value;

use crate::manifest::{DashboardWidget, WidgetSpec};
use knav_common::theme::theme;
use knav_common::util::text::truncate;

use knav_k8s::EventEntry;

use super::kit::{Slice, activity, grid, legend, sparkline, stacked};
use super::{Dashboard, DashboardContext};

/// A manifest's widgets, each with the `(group, kind)` pairs it reads, checked at parse time.
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
        let mut cards: Vec<Card> = Vec::new();
        let mut lists = Vec::new();
        let mut kinds: Vec<&str> = Vec::new();
        for (widget, sources) in &self.widgets {
            let label = widget.label.clone().unwrap_or_else(|| {
                let mut names = vec![widget.kind.clone()];
                names.extend(widget.extra_kinds.clone());
                names.join(" / ")
            });
            let own: Vec<&str> = std::iter::once(widget.kind.as_str()).chain(widget.extra_kinds.iter().map(String::as_str)).collect();
            for kind in &own {
                if !kinds.contains(kind) {
                    kinds.push(kind);
                }
            }
            let recent = activity(ctx.events, &own);
            match widget.spec {
                // `count` only needs the number; the other widgets need the objects.
                WidgetSpec::Count => {
                    let total: usize = sources.iter().map(|(group, kind)| ctx.count(group, kind)).sum();
                    cards.push(Card { label, spec: widget.spec.clone(), objects: Vec::new(), total, recent });
                }
                WidgetSpec::Tally { .. } | WidgetSpec::Sum { .. } | WidgetSpec::Breakdown { .. } => {
                    let objects: Vec<Value> = sources.iter().flat_map(|(group, kind)| ctx.fetch(group, kind)).collect();
                    let total = objects.len();
                    cards.push(Card { label, spec: widget.spec.clone(), objects, total, recent });
                }
                WidgetSpec::List { .. } => {
                    let objects: Vec<Value> = sources.iter().flat_map(|(group, kind)| ctx.fetch(group, kind)).collect();
                    if !objects.is_empty() {
                        lists.push(render_widget(&label, &widget.spec, &objects));
                    }
                }
            }
        }
        // A kind with nothing in the cluster has nothing to draw.
        cards.retain(|c| c.total > 0);
        let mut out = vec![Line::default()];
        if cards.is_empty() && lists.is_empty() {
            out.push(Line::styled(format!(" Nothing from {} is in this cluster yet.", self.title), Style::default().fg(theme().muted)));
            return out;
        }
        out.extend(grid(ctx.width, 34, &cards, card_widget));
        for list in lists {
            out.push(Line::default());
            out.extend(list);
        }
        let recent: Vec<&EventEntry> = ctx.events.iter().filter(|e| kinds.contains(&e.kind.as_str())).take(20).collect();
        if !recent.is_empty() {
            out.push(Line::default());
            out.extend(events_section(&recent, ctx.width));
        }
        out
    }
}

/// One count, tally, sum or breakdown, ready to draw as a card.
struct Card {
    label: String,
    spec: WidgetSpec,
    objects: Vec<Value>,
    total: usize,
    /// Events about the widget's kinds in each five minutes of the last hour.
    recent: Vec<usize>,
}

/// Recent events as rows: age, kind, object, message, sized to `width`.
fn events_section(events: &[&EventEntry], width: usize) -> Vec<Line<'static>> {
    let mut out = vec![Line::styled(format!(" Recent changes ({})", events.len()), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD))];
    // The object and message share what the age and kind leave; a narrow screen drops the message.
    let left = width.saturating_sub(4 + 6 + 15);
    let object_room = (left / 3).clamp(8, 25);
    let message_room = left.saturating_sub(object_room + 1);
    for e in events {
        let color = if e.severity == knav_k8s::EventSeverity::Warning { theme().warn } else { theme().ok };
        out.push(Line::from(vec![
            Span::styled("  ● ", Style::default().fg(color)),
            Span::styled(format!("{:<6}", e.age), Style::default().fg(theme().muted)),
            Span::styled(format!("{:<15}", truncate(&e.kind, 14)), Style::default().fg(color)),
            Span::raw(format!("{:<w$}", truncate(&e.object, object_room - 1), w = object_room)),
            Span::styled(if message_room >= 8 { truncate(&e.message, message_room) } else { String::new() }, Style::default().fg(theme().muted)),
        ]));
    }
    out
}

fn at_dotted<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.trim_start_matches('.').split('.').try_fold(value, |v, key| v.get(key))
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        // A list of plain values reads as a comma-separated line (hosts, gateways).
        Value::Sequence(items) => {
            let parts: Vec<String> = items.iter().filter_map(scalar_text).collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
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

/// A field's value ordered by what it is: a timestamp, a number or text.
/// Missing values sort last.
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

/// Days until (or since) the timestamp at `path`, coloured by how soon.
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

fn color_named(name: &str) -> Color {
    match name {
        "ok" => theme().ok,
        "warn" => theme().warn,
        "bad" => theme().bad,
        "accent" => theme().accent,
        _ => theme().muted,
    }
}

/// What a tally's "true" is called: `ready`, the condition's name, or the field's last key.
fn word_of(by: &str) -> String {
    match by.strip_prefix("condition:").or_else(|| by.strip_prefix("field:")) {
        Some(rest) => rest.rsplit('.').next().unwrap_or(rest).to_lowercase(),
        None => "ready".into(),
    }
}

fn big(n: usize, color: Color) -> Span<'static> {
    Span::styled(n.to_string(), Style::default().fg(color).add_modifier(Modifier::BOLD))
}

/// The card: a headline number, a bar, a legend and, with activity, a sparkline.
fn card_widget(item: &Card, width: usize) -> Vec<Line<'static>> {
    let Card { label, spec, objects, total, recent } = item;
    let total = *total;
    let inner = width.saturating_sub(4);
    let muted = Style::default().fg(theme().muted);
    let mut body = match spec {
        WidgetSpec::Tally { by, suspend } => {
            let (mut t, mut f, mut u, mut sus) = (0, 0, 0, 0);
            for object in objects {
                if suspend.as_deref().is_some_and(|path| matches!(at_dotted(object, path), Some(Value::Bool(true)))) {
                    sus += 1;
                    continue;
                }
                match bucket_of(object, by) {
                    Some(true) => t += 1,
                    Some(false) => f += 1,
                    None => u += 1,
                }
            }
            let word = word_of(by);
            let color = if f > 0 { theme().bad } else if t == total { theme().ok } else { theme().warn };
            let mut body = vec![
                Line::from(vec![big(t, color), Span::styled(format!(" / {total}  {word}"), muted)]),
                Line::from(stacked(inner, &[Slice::new(t, theme().ok), Slice::new(f, theme().bad), Slice::new(sus, theme().muted), Slice::new(u, theme().warn)])),
            ];
            body.extend(legend(&[(t, word.as_str(), theme().ok), (f, "failing", theme().bad), (sus, "suspended", theme().muted), (u, "unknown", theme().warn)], inner));
            body
        }
        WidgetSpec::Breakdown { by, values } => {
            let mut counts = vec![0usize; values.len()];
            let mut other = 0;
            for object in objects {
                let text = at_dotted(object, by).and_then(scalar_text);
                match values.iter().position(|[value, _]| Some(value) == text.as_ref()) {
                    Some(i) => counts[i] += 1,
                    None => other += 1,
                }
            }
            let mut slices: Vec<Slice> = values.iter().zip(&counts).map(|([_, color], n)| Slice::new(*n, color_named(color))).collect();
            slices.push(Slice::new(other, theme().muted));
            let mut entries: Vec<(usize, &str, Color)> = values.iter().zip(&counts).map(|([value, color], n)| (*n, value.as_str(), color_named(color))).collect();
            entries.push((other, "other", theme().muted));
            let first = values.first().map(|[value, color]| (value.to_lowercase(), color_named(color))).unwrap_or_default();
            let mut body = vec![Line::from(vec![big(counts[0], first.1), Span::styled(format!(" / {total}  {}", first.0), muted)]), Line::from(stacked(inner, &slices))];
            body.extend(legend(&entries, inner));
            body
        }
        WidgetSpec::Sum { fields } => {
            let sums: Vec<(String, usize)> = fields
                .iter()
                .map(|[field_label, path]| (field_label.clone(), objects.iter().filter_map(|o| at_dotted(o, path)).filter_map(Value::as_i64).sum::<i64>().max(0) as usize))
                .collect();
            let palette = [theme().ok, theme().bad, theme().warn, theme().muted, theme().accent];
            let slices: Vec<Slice> = sums.iter().enumerate().map(|(i, (_, n))| Slice::new(*n, palette[i % palette.len()])).collect();
            let entries: Vec<(usize, &str, Color)> = sums.iter().enumerate().map(|(i, (name, n))| (*n, name.as_str(), palette[i % palette.len()])).collect();
            let mut body = vec![Line::from(vec![big(total, theme().text_strong), Span::styled("  total", muted)]), Line::from(stacked(inner, &slices))];
            body.extend(legend(&entries, inner));
            body
        }
        _ => vec![Line::from(vec![big(total, theme().text_strong), Span::styled("  total", muted)])],
    };
    if recent.iter().any(|n| *n > 0) {
        let mut spans = sparkline(recent, theme().accent);
        spans.push(Span::styled("  last hour", muted));
        body.push(Line::from(spans));
    }
    super::kit::card(label, width, body)
}

fn render_widget(label: &str, spec: &WidgetSpec, objects: &[Value]) -> Vec<Line<'static>> {
    match spec {
        WidgetSpec::Count => vec![Line::from(vec![Span::styled(format!("{} ", objects.len()), Style::default().add_modifier(Modifier::BOLD)), Span::raw(label.to_string())])],
        WidgetSpec::Tally { by, .. } => {
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
        WidgetSpec::Breakdown { .. } => vec![Line::styled(label.to_string(), Style::default().add_modifier(Modifier::BOLD))],
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

    fn app(sync: &str, health: &str) -> Value {
        serde_json::from_value(serde_json::json!({"status": {"sync": {"status": sync}, "health": {"status": health}}})).unwrap()
    }

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(line_text).collect()
    }

    #[test]
    fn a_breakdown_card_counts_each_value_and_leaves_the_rest_as_other() {
        let objects = vec![app("Synced", "Healthy"), app("Synced", "Degraded"), app("OutOfSync", "Healthy"), app("Weird", "Healthy")];
        let spec = WidgetSpec::Breakdown { by: ".status.sync.status".into(), values: vec![["Synced".into(), "ok".into()], ["OutOfSync".into(), "warn".into()]] };
        let card = Card { label: "Argo CD sync".into(), spec, objects, total: 4, recent: vec![0; 12] };
        let lines = text(&card_widget(&card, 40));
        assert!(lines[0].contains("Argo CD sync"), "{lines:?}");
        assert!(lines[1].contains("2 / 4  synced"), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("● 2 Synced") && l.contains("● 1 OutOfSync") && l.contains("● 1 other")), "{lines:?}");
    }

    #[test]
    fn a_tally_counts_suspended_objects_apart() {
        let suspended: Value = serde_json::from_value(serde_json::json!({"spec": {"suspend": true}, "status": {"conditions": [{"type": "Ready", "status": "True"}]}})).unwrap();
        let spec = WidgetSpec::Tally { by: "ready".into(), suspend: Some(".spec.suspend".into()) };
        let card = Card { label: "Kustomization".into(), spec, objects: vec![obj("True"), obj("False"), suspended], total: 3, recent: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2] };
        let lines = text(&card_widget(&card, 44));
        assert!(lines.iter().any(|l| l.contains("1 / 3  ready")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("● 1 suspended") && l.contains("● 1 failing")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("last hour")), "{lines:?}");
    }

    #[test]
    fn a_list_of_values_reads_as_one_comma_separated_line() {
        let object: Value = serde_json::from_value(serde_json::json!({"spec": {"hosts": ["a.example.com", "b.example.com"]}})).unwrap();
        assert_eq!(at_dotted(&object, ".spec.hosts").and_then(scalar_text).as_deref(), Some("a.example.com, b.example.com"));
    }
}
