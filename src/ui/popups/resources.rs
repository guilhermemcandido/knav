//! Resource usage, containers and the relations diagram.

use super::*;

/// The relations diagram in a full-size frame.
pub(in crate::ui) fn draw_relations(frame: &mut Frame, title: &str, graph: &crate::k8s::relations::Graph, selected: usize) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(Line::styled(format!(" Related to {title} "), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)).centered())
        .title_bottom(Line::styled(" ←↑↓→ move   enter info   o open list   space follow   backspace back   esc close ", Style::default().fg(theme().muted)).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if graph.nodes.len() <= 1 {
        frame.render_widget(Paragraph::new("Nothing else is related to this object.").style(Style::default().fg(theme().muted)).alignment(Alignment::Center), inner);
        return;
    }
    super::graph::draw_graph(frame, inner, graph, selected);
}

/// A bar `width` cells wide filled to `ratio`.
fn fill_bar(ratio: f64, width: usize, color: Color, dimmed: bool) -> Line<'static> {
    let filled = ((ratio.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    let (on, off) = if dimmed { (dim_style(), dim_style()) } else { (Style::default().fg(color), Style::default().fg(theme().muted)) };
    Line::from(vec![Span::styled("█".repeat(filled), on), Span::styled("░".repeat(width - filled), off)])
}

/// One card of the Resources view: what is used, and what the pods ask for.
fn resource_card(frame: &mut Frame, area: Rect, title: &str, used: Option<f64>, asked: Option<f64>, capacity: f64, show: &dyn Fn(f64) -> String, dimmed: bool) {
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(if dimmed { dim_style() } else { theme_border(false) }).title(Line::styled(format!(" {title} "), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let width = usize::from(inner.width);
    let muted = Style::default().fg(theme().muted);
    let ratio = |v: f64| if capacity > 0.0 { v / capacity } else { 0.0 };
    let percent = |label: &str, value: Option<f64>| match value {
        Some(v) => Line::from(vec![Span::styled(format!("{:.0}%", ratio(v) * 100.0), if dimmed { dim_style() } else { Style::default().fg(usage_color(ratio(v), false)).add_modifier(Modifier::BOLD) }), Span::styled(format!("  {label}"), muted)]),
        None => Line::from(vec![Span::styled("n/a", muted), Span::styled(format!("  {label} (no metrics-server)"), muted)]),
    };
    let amount = |value: Option<f64>| Line::styled(value.map(|v| format!("{} / {}", show(v), show(capacity))).unwrap_or_default(), muted);
    let mut lines = vec![percent("in use", used), fill_bar(used.map_or(0.0, ratio), width, usage_color(used.map_or(0.0, ratio), dimmed), dimmed), amount(used)];
    if let Some(asked) = asked {
        lines.push(percent("asked by pods", Some(asked)));
        lines.push(fill_bar(ratio(asked), width, theme().accent, dimmed));
        lines.push(amount(Some(asked)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The Resources view: usage and requests against what the nodes offer, the
/// busiest nodes, how the pods are doing and where they are. Long lists are cut to
/// what fits, with a count of the rest.
pub(in crate::ui) fn draw_resources_detail_popup(frame: &mut Frame, overview: &Overview, nodes: &[crate::k8s::NodeRow], dimmed: bool) {
    let area = body_area(frame.area(), false);
    frame.render_widget(Clear, area);
    let outer = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(if dimmed { dim_style() } else { theme_border(false) }).title(Line::styled(" Resources ", Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let report = overview.report.as_ref();
    let muted = Style::default().fg(theme().muted);

    let rows = Layout::vertical([Constraint::Length(8), Constraint::Min(6)]).split(inner);
    let cards = Layout::horizontal([Constraint::Ratio(1, 3); 3]).spacing(1).split(rows[0]);
    let cores = |v: f64| format!("{:.2} cores", v / 1000.0);
    let usage = overview.metrics_available;
    resource_card(frame, cards[0], "CPU", usage.then_some(overview.cpu_usage_millicores as f64), report.map(|r| r.cpu_requests_millicores as f64), overview.cpu_capacity_millicores as f64, &cores, dimmed);
    resource_card(frame, cards[1], "Memory", usage.then_some(overview.memory_usage_bytes as f64), report.map(|r| r.memory_requests_bytes as f64), overview.memory_capacity_bytes as f64, &format_bytes, dimmed);
    resource_card(frame, cards[2], "Pods", Some(workloads_pod_count(overview) as f64), None, overview.pod_capacity as f64, &|v| format!("{v:.0}"), dimmed);

    let halves = Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)]).spacing(1).split(rows[1]);

    // The busiest nodes first, so a big cluster shows what needs attention.
    let pressure = |n: &crate::k8s::NodeRow| {
        let ratio = |used: Option<i64>, cap: i64| if cap > 0 { used.unwrap_or(0) as f64 / cap as f64 } else { 0.0 };
        ratio(n.cpu_millicores, n.cpu_capacity).max(ratio(n.memory_bytes, n.memory_capacity))
    };
    let mut ordered: Vec<&crate::k8s::NodeRow> = nodes.iter().collect();
    ordered.sort_by(|a, b| (a.ready, a.schedulable).cmp(&(b.ready, b.schedulable)).then(pressure(b).total_cmp(&pressure(a))).then_with(|| a.name.cmp(&b.name)));
    let (not_ready, cordoned) = (nodes.iter().filter(|n| !n.ready).count(), nodes.iter().filter(|n| n.ready && !n.schedulable).count());
    let summary = format!("{} ready", nodes.len() - not_ready - cordoned);
    let mut title = vec![Span::styled(format!(" Nodes ({}) ", nodes.len()), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)), Span::styled(summary, Style::default().fg(theme().ok))];
    if cordoned > 0 {
        title.push(Span::styled(format!(" · {cordoned} cordoned"), Style::default().fg(theme().warn)));
    }
    if not_ready > 0 {
        title.push(Span::styled(format!(" · {not_ready} not ready"), Style::default().fg(theme().bad)));
    }
    title.push(Span::raw(" "));
    let nodes_block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(if dimmed { dim_style() } else { theme_border(false) }).title(Line::from(title));
    let nodes_inner = nodes_block.inner(halves[0]);
    frame.render_widget(nodes_block, halves[0]);
    let nodes_inner = Rect { x: nodes_inner.x + 1, width: nodes_inner.width.saturating_sub(2), ..nodes_inner };
    let room = usize::from(nodes_inner.height).saturating_sub(1);
    let shown = if ordered.len() > room { room.saturating_sub(1) } else { ordered.len() };
    let mut table_rows: Vec<Row> = ordered
        .iter()
        .take(shown)
        .map(|n| {
            let (cpu_req, mem_req) = report.and_then(|r| r.node_requests.get(&n.name).copied()).unzip();
            let status = if !n.ready { ("NotReady", theme().bad) } else if !n.schedulable { ("Cordoned", theme().warn) } else { ("Ready", theme().ok) };
            let asked = |req: Option<i64>, cap: i64| match (report, cap > 0) {
                (Some(_), true) => format!("{:.0}%", req.unwrap_or(0) as f64 / cap as f64 * 100.0),
                _ => "-".to_string(),
            };
            Row::new(vec![
                Cell::from(truncate(&n.name, 30)),
                Cell::from(Span::styled(status.0, if dimmed { dim_style() } else { Style::default().fg(status.1) })),
                Cell::from(usage_bar(n.cpu_millicores, n.cpu_capacity, dimmed)),
                Cell::from(usage_bar(n.memory_bytes, n.memory_capacity, dimmed)),
                Cell::from(format!("{}/{}", asked(cpu_req, n.cpu_capacity), asked(mem_req, n.memory_capacity))),
                Cell::from(format!("{}/{}", n.pod_count, n.pod_capacity)),
            ])
        })
        .collect();
    if shown < ordered.len() {
        table_rows.push(Row::new(vec![Cell::from(Span::styled(format!("+{} more, busiest first", ordered.len() - shown), muted))]));
    }
    let header = Row::new(["NAME", "STATUS", "CPU", "MEMORY", "ASKED", "PODS"]).style(Style::default().fg(theme().header).add_modifier(Modifier::BOLD));
    let table = Table::new(table_rows, [Constraint::Min(20), Constraint::Length(9), Constraint::Length(16), Constraint::Length(16), Constraint::Length(10), Constraint::Length(8)]).header(header).column_spacing(2).style(theme_row(dimmed));
    frame.render_widget(table, nodes_inner);

    // How the pods are doing, and which namespaces hold the most.
    let pods_block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(if dimmed { dim_style() } else { theme_border(false) }).title(Line::styled(" Pods ", Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)));
    let pods_inner = pods_block.inner(halves[1]);
    frame.render_widget(pods_block, halves[1]);
    let pods_inner = Rect { x: pods_inner.x + 1, width: pods_inner.width.saturating_sub(2), ..pods_inner };
    let Some(report) = report else { return };
    let phase_color = |phase: &str| match phase {
        "Running" => theme().ok,
        "Pending" | "Unknown" => theme().warn,
        "Failed" => theme().bad,
        _ => theme().muted,
    };
    let total: usize = report.phases.iter().map(|(_, n)| n).sum::<usize>().max(1);
    let width = usize::from(pods_inner.width);
    let mut lines: Vec<Line> = Vec::new();
    // One bar for all pods, a colour per phase.
    let mut used = 0;
    let mut segments: Vec<Span> = Vec::new();
    for (i, (phase, n)) in report.phases.iter().enumerate() {
        let cells = if i + 1 == report.phases.len() { width.saturating_sub(used) } else { (n * width / total).max(1).min(width.saturating_sub(used)) };
        used += cells;
        segments.push(Span::styled("█".repeat(cells), if dimmed { dim_style() } else { Style::default().fg(phase_color(phase)) }));
    }
    lines.push(Line::from(segments));
    for (phase, n) in &report.phases {
        lines.push(Line::from(vec![Span::styled("● ", if dimmed { dim_style() } else { Style::default().fg(phase_color(phase)) }), Span::raw(format!("{phase:<11}")), Span::styled(n.to_string(), Style::default().add_modifier(Modifier::BOLD))]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Top namespace activity", Style::default().fg(theme().heading).add_modifier(Modifier::BOLD)));
    let biggest = report.namespaces.first().map_or(1, |(_, n)| *n).max(1);
    let name_room = 18.min(width / 2);
    let bar_room = width.saturating_sub(name_room + 6).max(2);
    for (name, n) in &report.namespaces {
        let cells = (n * bar_room / biggest).max(1);
        lines.push(Line::from(vec![Span::raw(format!("{:<w$}", truncate(name, name_room), w = name_room + 1)), Span::styled("█".repeat(cells), if dimmed { dim_style() } else { Style::default().fg(theme().namespace) }), Span::styled(format!(" {n}"), Style::default().add_modifier(Modifier::BOLD))]));
    }
    if report.namespace_count > report.namespaces.len() {
        lines.push(Line::styled(format!("+{} more namespaces", report.namespace_count - report.namespaces.len()), muted));
    }
    frame.render_widget(Paragraph::new(lines), pods_inner);
}

pub(in crate::ui) fn draw_containers_popup(frame: &mut Frame, title: &str, containers: &[ContainerInfo], state: &mut TableState, sort: SortState, dimmed: bool) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);

    let muted = dim_style();
    const HEADERS: [&str; 4] = ["", "NAME", "STATE", "RESTARTS"];
    let state_text_of = |c: &ContainerInfo| {
        c.reason.clone().unwrap_or_else(|| match c.status {
            ContainerStatusKind::Running => "Running".into(),
            ContainerStatusKind::Waiting => "Waiting".into(),
            ContainerStatusKind::Terminated => "Terminated".into(),
            ContainerStatusKind::Unknown => "Unknown".into(),
        })
    };
    let window = layout_table(
        &HEADERS,
        containers.iter().map(|c| vec![1, cell_width(&c.name), cell_width(&state_text_of(c)), c.restarts.to_string().len()]),
        area.width.saturating_sub(2),
        None,
        &mut 0,
    );
    let header = header_row(&HEADERS, sort, dimmed, &window);
    let cell_style = theme_row(dimmed);
    let rows = containers.iter().map(|c| {
        let (glyph, color) = container_dot(c);
        let dot_style = if dimmed { muted } else { Style::default().fg(color) };
        Row::new(window.slice(vec![
            Cell::from(Span::styled(glyph, dot_style)),
            Cell::from(c.name.clone()).style(cell_style),
            Cell::from(state_text_of(c)).style(dot_style),
            Cell::from(c.restarts.to_string()).style(cell_style),
        ]))
    });

    let border_style = theme_border(dimmed);
    let table = Table::new(mark_rows(rows, &[], dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_set(border_set())
                .border_style(border_style)
                .title(if dimmed { Line::styled(title.to_string(), muted) } else { colored_slash_title(title) }),
        )
        .highlight_symbol("")
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, dimmed));

    frame.render_stateful_widget(table, area, state);
}

#[cfg(test)]
mod events_popup_tests {
    use super::*;

    fn entry(severity: crate::k8s::EventSeverity) -> EventEntry {
        EventEntry {
            message: "m".into(),
            reason: "r".into(),
            object: "o".into(),
            namespace: String::new(),
            kind: "Pod".into(),
            age: "1m".into(),
            age_secs: 60,
            severity,
        }
    }

    #[test]
    fn workloads_pod_count_reads_the_pods_tile_from_the_catalog() {
        let overview = Overview {
            events: vec![],
            cpu_usage_millicores: 0,
            cpu_capacity_millicores: 0,
            memory_usage_bytes: 0,
            memory_capacity_bytes: 0,
            pod_capacity: 0,
            metrics_available: false,
            catalog: vec![("Workloads", vec![("Pods", 17), ("Deployments", 4)])],
            health: Default::default(),
            report: None,
        };
        assert_eq!(workloads_pod_count(&overview), 17);
    }

    #[test]
    fn event_row_at_resolves_the_first_row_and_respects_offset() {
        let frame_area = Rect { x: 0, y: 0, width: 100, height: 40 };
        let area = centered_rect(94, 88, frame_area);
        let top = area.y + 2;
        assert_eq!(event_row_at(frame_area, 5, 0, top), Some(0));
        assert_eq!(event_row_at(frame_area, 5, 2, top), Some(2));
        assert_eq!(event_row_at(frame_area, 5, 0, top - 1), None); // header row, not a data row
    }

    #[test]
    fn event_row_at_is_none_past_the_filtered_list_or_the_table() {
        let frame_area = Rect { x: 0, y: 0, width: 100, height: 40 };
        let area = centered_rect(94, 88, frame_area);
        let top = area.y + 2;
        assert_eq!(event_row_at(frame_area, 1, 0, top + 1), None); // only 1 row exists
        let bottom = area.y + area.height - 1;
        assert_eq!(event_row_at(frame_area, 100, 0, bottom), None); // bottom border row
    }

    #[test]
    fn event_filter_matches_the_right_severities() {
        let normal = entry(crate::k8s::EventSeverity::Normal);
        let warning = entry(crate::k8s::EventSeverity::Warning);
        assert!(EventFilter::All.matches(&normal) && EventFilter::All.matches(&warning));
        assert!(EventFilter::Warnings.matches(&warning) && !EventFilter::Warnings.matches(&normal));
        assert!(EventFilter::Normal.matches(&normal) && !EventFilter::Normal.matches(&warning));
    }

    #[test]
    fn events_search_matches_reason_object_kind_and_message_case_insensitively() {
        let mut crash = entry(crate::k8s::EventSeverity::Warning);
        crash.reason = "BackOff".into();
        crash.message = "Back-off restarting failed container".into();
        crash.object = "web-1".into();
        let ok = entry(crate::k8s::EventSeverity::Normal);
        let events = vec![crash, ok];
        let found = |filter, search: &str| crate::k8s::filter_events(&events, filter, search, None).len();
        assert_eq!(found(EventFilter::All, ""), 2);
        assert_eq!(found(EventFilter::All, "backoff"), 1); // reason
        assert_eq!(found(EventFilter::All, "WEB-1"), 1); // object
        assert_eq!(found(EventFilter::All, "restarting"), 1); // message
        assert_eq!(found(EventFilter::All, "pod"), 2); // kind, on both
        assert_eq!(found(EventFilter::All, "zzz"), 0);
        assert_eq!(found(EventFilter::Normal, "backoff"), 0); // severity filter still applies
    }
}
