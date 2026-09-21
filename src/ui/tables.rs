//! The resource list tables (pods, deployments, nodes, generic, CRD picker) and their row helpers.

use super::*;

/// A `Terminated` container isn't necessarily a problem — a Job/init
/// container that ran to completion and exited 0 gets this same status
/// kind, distinguished only by `reason` being "Completed" rather than
/// something like "Error"/"OOMKilled". Red is for the latter; a clean
/// completion gets the same blue k9s/kubectl use for it.
pub(super) fn container_dot(c: &ContainerInfo) -> (&'static str, Color) {
    match c.status {
        ContainerStatusKind::Running => ("●", Color::Green),
        ContainerStatusKind::Waiting => ("●", Color::Yellow),
        ContainerStatusKind::Terminated if c.reason.as_deref() == Some("Completed") => ("●", Color::Blue),
        ContainerStatusKind::Terminated => ("●", Color::Red),
        ContainerStatusKind::Unknown => ("●", Color::Gray),
    }
}

/// Compact form: just the colored dots — used for every row except the
/// one that's hovered/selected.
pub(super) fn containers_cell(containers: &[ContainerInfo], muted: bool) -> Line<'static> {
    let mut spans = Vec::with_capacity(containers.len() * 2);
    for c in containers {
        let (glyph, color) = container_dot(c);
        let style = if muted { dim_style() } else { Style::default().fg(color) };
        spans.push(Span::styled(glyph, style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

pub(super) fn container_state_text(c: &ContainerInfo) -> String {
    match c.status {
        ContainerStatusKind::Running => "Running".to_string(),
        ContainerStatusKind::Waiting => c.reason.clone().unwrap_or_else(|| "Waiting".into()),
        ContainerStatusKind::Terminated => c.reason.clone().unwrap_or_else(|| "Terminated".into()),
        ContainerStatusKind::Unknown => "Unknown".into(),
    }
}

pub(super) fn draw_status_line(frame: &mut Frame, area: Rect, pods: &[PodRow], row: Option<usize>, dimmed: bool) {
    let line = match (dimmed, row.and_then(|i| pods.get(i))) {
        (false, Some(pod)) => {
            let mut spans = namespace_name_spans(&pod.namespace, &pod.name);
            spans.push(Span::raw(" ["));
            for (i, c) in pod.containers.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" : ", Style::default().fg(Color::DarkGray)));
                }
                let (glyph, color) = container_dot(c);
                let style = Style::default().fg(color);
                spans.push(Span::styled(format!("{glyph} "), style));
                spans.push(Span::styled(c.name.clone(), style));
                spans.push(Span::styled(format!("({})", container_state_text(c)), style));
            }
            spans.push(Span::raw("]"));
            Line::from(spans)
        }
        _ => Line::raw(""),
    };
    frame.render_widget(Paragraph::new(line), area);
}

/// A real floating popup, positioned right next to the cursor — "in
/// front," on top of everything, only while actively hovering.
pub(super) fn draw_hover_popup(frame: &mut Frame, pod: &PodRow, column: u16, row: u16, bounds: Rect) {
    let lines: Vec<Line> = pod
        .containers
        .iter()
        .map(|c| {
            let (glyph, color) = container_dot(c);
            let style = Style::default().fg(color);
            Line::from(vec![
                Span::styled(format!("{glyph} "), style),
                Span::raw(format!("{}: ", c.name)),
                Span::styled(container_state_text(c), style),
            ])
        })
        .collect();

    let width = lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(10)
        .max(pod.name.len() + pod.namespace.len() + 1)
        .saturating_add(4) as u16;
    let height = (lines.len() as u16).saturating_add(2).max(3);

    let area = popup_near(column, row, width, height, bounds);
    frame.render_widget(Clear, area);

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(format!("{}/{}", pod.namespace, pod.name));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

pub(super) fn draw_table(frame: &mut Frame, area: Rect, pods: &[PodRow], table_state: &mut TableState, search: Search, sort: SortState, dimmed: bool) {
    let muted = dim_style();
    let border_style = theme_border(dimmed);

    let header = header_row(&["NAMESPACE", "NAME", "READY", "STATUS", "RESTARTS", "NODE", "AGE", "CONTAINERS"], sort, dimmed);

    let rows = pods.iter().map(|p| {
        let status_style = if dimmed {
            muted
        } else {
            let color = match p.phase.as_str() {
                "Running" => ROW_FG,
                "Pending" => Color::Yellow,
                "Failed" => Color::Red,
                _ => Color::Gray,
            };
            Style::default().fg(color)
        };
        let cell_style = theme_row(dimmed);
        Row::new(vec![
            Cell::from(highlight_fuzzy(&p.namespace, search.text, cell_style)),
            Cell::from(highlight_fuzzy(&p.name, search.text, cell_style)),
            Cell::from(p.ready.clone()).style(cell_style),
            Cell::from(p.phase.clone()).style(status_style),
            Cell::from(p.restarts.to_string()).style(cell_style),
            Cell::from(p.node.clone()).style(cell_style),
            Cell::from(p.age.clone()).style(cell_style),
            Cell::from(containers_cell(&p.containers, dimmed)),
        ])
    });

    let title = table_title("Pods", pods.len(), search, dimmed);

    let highlight_style = theme_highlight(dimmed);

    let table = Table::new(rows, pod_table_widths())
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol("");

    frame.render_stateful_widget(table, area, table_state);
}

pub(super) fn pod_table_widths() -> [Constraint; 8] {
    [
        Constraint::Fill(2),   // namespace
        Constraint::Fill(3),   // name
        Constraint::Length(11), // (3)READY ▲
        Constraint::Fill(2),   // status
        Constraint::Length(14), // (5)RESTARTS ▲
        Constraint::Fill(2),   // node
        Constraint::Length(9), // (7)AGE ▲
        Constraint::Fill(3),   // containers
    ]
}

/// Which pod row sits under an absolute terminal position, restricted to
/// the CONTAINERS column specifically — hovering anywhere else in the row
/// shouldn't trigger the popup, only the dots themselves. Reuses
/// `Table`'s own column constraints through a real `Layout` solve (same
/// widths, same default 1-cell `column_spacing`) rather than
/// hand-guessing pixel math that could silently drift out of sync with
/// what's actually rendered.
pub fn row_at(frame_area: Rect, table_state: &TableState, row_count: usize, column: u16, row: u16) -> Option<usize> {
    let table_area =
        Rect { x: frame_area.x, y: frame_area.y, width: frame_area.width, height: frame_area.height.saturating_sub(1) };

    let inner = Rect {
        x: table_area.x.saturating_add(1),
        y: table_area.y.saturating_add(2), // border + header
        width: table_area.width.saturating_sub(2),
        height: table_area.height.saturating_sub(3), // top border + header + bottom border
    };

    if row < inner.y || row >= inner.y + inner.height {
        return None;
    }

    let columns = Layout::horizontal(pod_table_widths()).spacing(1).split(inner);
    let containers_col = columns[7];
    if column < containers_col.x || column >= containers_col.x + containers_col.width {
        return None;
    }

    let offset = table_state.offset();
    let index = offset + usize::from(row - inner.y);
    (index < row_count).then_some(index)
}

pub(super) fn draw_deployment_table(frame: &mut Frame, area: Rect, deployments: &[DeploymentRow], table_state: &mut TableState, search: Search, sort: SortState, dimmed: bool) {
    let border_style = theme_border(dimmed);
    let cell_style = theme_row(dimmed);

    let header = header_row(&["NAMESPACE", "NAME", "READY", "UP-TO-DATE", "AVAILABLE", "AGE"], sort, dimmed);

    let rows = deployments.iter().map(|d| {
        Row::new(vec![
            Cell::from(highlight_fuzzy(&d.namespace, search.text, cell_style)),
            Cell::from(highlight_fuzzy(&d.name, search.text, cell_style)),
            Cell::from(d.ready.clone()).style(cell_style),
            Cell::from(d.up_to_date.to_string()).style(cell_style),
            Cell::from(d.available.to_string()).style(cell_style),
            Cell::from(d.age.clone()).style(cell_style),
        ])
    });

    let widths = [
        Constraint::Fill(2),
        Constraint::Fill(3),
        Constraint::Length(11),
        Constraint::Length(17),
        Constraint::Length(14),
        Constraint::Length(9),
    ];

    let title = table_title("Deployments", deployments.len(), search, dimmed);

    let highlight_style = theme_highlight(dimmed);

    let table = Table::new(rows, widths)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol("");

    frame.render_stateful_widget(table, area, table_state);
}

/// A compact inline usage bar for a table cell: `▓▓▓░░░░░ 34%`, or
/// `n/a` in gray when metrics-server isn't installed. Same block-style
/// bar `draw_meter` uses for the full-width Cluster Resources meters,
/// just narrow enough to fit a column.
pub(super) fn usage_bar(used: Option<i64>, capacity: i64, dimmed: bool) -> Line<'static> {
    const WIDTH: usize = 10;
    let Some(used) = used else {
        return Line::styled("n/a", Style::default().fg(Color::DarkGray));
    };
    let ratio = if capacity > 0 { (used as f64 / capacity as f64).clamp(0.0, 1.0) } else { 0.0 };
    let filled = (ratio * WIDTH as f64).round() as usize;
    let color = usage_color(ratio, dimmed);
    let bracket = if dimmed { dim_style() } else { Style::default().fg(Color::DarkGray) };
    Line::from(vec![
        Span::styled("[", bracket),
        Span::styled("▓".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(WIDTH - filled), Style::default().fg(Color::DarkGray)),
        Span::styled("]", bracket),
        Span::raw(format!(" {:.0}%", ratio * 100.0)),
    ])
}

pub(super) fn usage_color(ratio: f64, dimmed: bool) -> Color {
    if dimmed {
        Color::Rgb(40, 40, 40)
    } else if ratio > 0.9 {
        Color::Red
    } else if ratio > 0.7 {
        Color::Yellow
    } else {
        Color::Green
    }
}

pub(super) fn draw_nodes_table(frame: &mut Frame, area: Rect, nodes: &[NodeRow], table_state: &mut TableState, search: Search, sort: SortState, dimmed: bool) {
    let muted = dim_style();
    let border_style = theme_border(dimmed);
    let cell_style = theme_row(dimmed);

    let header = header_row(&["NAME", "STATUS", "ROLES", "CPU", "MEMORY", "PODS", "AGE", "VERSION"], sort, dimmed);

    let rows = nodes.iter().map(|n| {
        let status_style = if dimmed {
            muted
        } else if n.ready && n.schedulable {
            Style::default().fg(ROW_FG)
        } else if n.ready {
            Style::default().fg(Color::Yellow) // cordoned, but otherwise healthy
        } else {
            Style::default().fg(Color::Red)
        };
        // kubectl's own convention: append ",SchedulingDisabled" to STATUS
        // rather than a separate column.
        let status = match (n.ready, n.schedulable) {
            (true, true) => "Ready".to_string(),
            (true, false) => "Ready,SchedulingDisabled".to_string(),
            (false, true) => "NotReady".to_string(),
            (false, false) => "NotReady,SchedulingDisabled".to_string(),
        };
        Row::new(vec![
            Cell::from(highlight_fuzzy(&n.name, search.text, cell_style)),
            Cell::from(status).style(status_style),
            Cell::from(n.roles.clone()).style(cell_style),
            Cell::from(usage_bar(n.cpu_millicores, n.cpu_capacity, dimmed)),
            Cell::from(usage_bar(n.memory_bytes, n.memory_capacity, dimmed)),
            Cell::from(format!("{}/{}", n.pod_count, n.pod_capacity)).style(cell_style),
            Cell::from(n.age.clone()).style(cell_style),
            Cell::from(n.version.clone()).style(cell_style),
        ])
    });

    let widths = [
        Constraint::Fill(2),
        Constraint::Length(24),
        Constraint::Fill(1),
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(12),
    ];

    let title = table_title("Nodes", nodes.len(), search, dimmed);

    let highlight_style = theme_highlight(dimmed);

    let table = Table::new(rows, widths)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol("");

    frame.render_stateful_widget(table, area, table_state);
}

/// The shared table for every resource kind that doesn't get specialized
/// columns — Namespace/Name/Age is all that's generically knowable about
/// an arbitrary Kubernetes object.
/// Cluster-scoped kinds (Nodes, ClusterRoles, PVs, StorageClasses, ...)
/// show "-" for every row's namespace — a column that's all dashes isn't
/// telling anyone anything, so `draw_generic_table` drops it entirely
/// when this is false.
pub(super) fn any_row_has_namespace(rows: &[GenericRow]) -> bool {
    rows.iter().any(|r| r.namespace != "-")
}

pub(super) fn draw_generic_table(frame: &mut Frame, area: Rect, rows: &[GenericRow], label: &str, table_state: &mut TableState, search: Search, sort: SortState, dimmed: bool) {
    let border_style = theme_border(dimmed);
    let cell_style = theme_row(dimmed);

    let show_namespace = any_row_has_namespace(rows);

    let (header, widths): (Row, Vec<Constraint>) = if show_namespace {
        (header_row(&["NAMESPACE", "NAME", "AGE"], sort, dimmed), vec![Constraint::Fill(2), Constraint::Fill(3), Constraint::Length(9)])
    } else {
        (header_row(&["NAME", "AGE"], sort, dimmed), vec![Constraint::Fill(1), Constraint::Length(9)])
    };

    let table_rows = rows.iter().map(|r| {
        let mut cells = Vec::with_capacity(3);
        if show_namespace {
            cells.push(Cell::from(highlight_fuzzy(&r.namespace, search.text, cell_style)));
        }
        cells.push(Cell::from(highlight_fuzzy(&r.name, search.text, cell_style)));
        cells.push(Cell::from(r.age.clone()).style(cell_style));
        Row::new(cells)
    });

    let title = table_title(label, rows.len(), search, dimmed);

    let highlight_style = theme_highlight(dimmed);

    let table = Table::new(table_rows, widths)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol("");

    frame.render_stateful_widget(table, area, table_state);
}

/// The Custom Resources picker: every discovered CRD kind, grouped
/// visually just by sorting on GROUP (already the order `discover_crds`
/// returns them in) rather than a nested per-group tile browser — simpler,
/// and still scannable since same-group kinds land next to each other.
/// Selecting a row and pressing Enter is what actually starts watching
/// that kind (see the CustomResourceList Enter handler in main.rs) —
/// nothing here is live-watched itself, consistent with the "list only
/// until opened" design.
pub(super) fn draw_crd_list_table(frame: &mut Frame, area: Rect, crds: &[(usize, CrdInfo)], heading: &str, table_state: &mut TableState, search: Search, sort: SortState, dimmed: bool) {
    let border_style = theme_border(dimmed);
    let cell_style = theme_row(dimmed);

    let header = header_row(&["GROUP", "KIND", "SCOPE"], sort, dimmed);

    let rows = crds.iter().map(|(_, c)| {
        Row::new(vec![
            Cell::from(highlight_fuzzy(c.group, search.text, cell_style)),
            Cell::from(highlight_fuzzy(c.kind, search.text, cell_style)),
            Cell::from(if c.namespaced { "Namespaced" } else { "Cluster" }).style(cell_style),
        ])
    });

    let widths = [Constraint::Fill(3), Constraint::Fill(2), Constraint::Length(11)];
    let title = table_title(heading, crds.len(), search, dimmed);

    let highlight_style = theme_highlight(dimmed);

    let table = Table::new(rows, widths)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol("");

    frame.render_stateful_widget(table, area, table_state);
}

#[cfg(test)]
mod generic_table_tests {
    use super::*;

    fn row(namespace: &str) -> GenericRow {
        GenericRow { namespace: namespace.to_string(), name: "x".to_string(), age: "1d".to_string(), age_secs: 0, uid: String::new(), owners: Vec::new() }
    }

    #[test]
    fn namespace_column_hidden_when_every_row_is_cluster_scoped() {
        assert!(!any_row_has_namespace(&[row("-"), row("-")]));
    }

    #[test]
    fn namespace_column_shown_when_any_row_has_a_real_namespace() {
        assert!(any_row_has_namespace(&[row("-"), row("default")]));
        assert!(!any_row_has_namespace(&[]));
    }
}
