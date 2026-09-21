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

pub(super) fn draw_table(frame: &mut Frame, area: Rect, pods: &[PodRow], table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, dimmed: bool) {
    let muted = dim_style();
    let border_style = theme_border(dimmed);

    let window = pod_window(pods, area.width, hscroll);
    let header = header_row(&POD_HEADERS, sort, dimmed, &window);

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
        Row::new(window.slice(vec![
            Cell::from(highlight_fuzzy(&p.namespace, search.text, cell_style)),
            Cell::from(highlight_fuzzy(&p.name, search.text, cell_style)),
            Cell::from(p.ready.clone()).style(if dimmed { cell_style } else { Style::default().fg(pod_ready_color(&p.ready, &p.phase)) }),
            Cell::from(p.phase.clone()).style(status_style),
            Cell::from(p.restarts.to_string()).style(cell_style),
            Cell::from(p.controlled_by.clone()).style(cell_style),
            Cell::from(p.node.clone()).style(cell_style),
            Cell::from(p.qos.clone()).style(cell_style),
            Cell::from(p.age.clone()).style(cell_style),
            Cell::from(containers_cell(&p.containers, dimmed)),
        ]))
    });

    let title = table_title("Pods", pods.len(), search, &window, dimmed);

    let table = Table::new(select_rows(rows, table_state.selected(), dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .highlight_symbol("");

    frame.render_stateful_widget(table, area, table_state);
}

const POD_HEADERS: [&str; 10] = ["NAMESPACE", "NAME", "READY", "STATUS", "RESTARTS", "CONTROLLER", "NODE", "QOS", "AGE", "CONTAINERS"];

/// The pods table's visible columns — shared by drawing and by hover
/// hit-testing so they can't disagree about where CONTAINERS is.
fn pod_window(pods: &[PodRow], table_width: u16, hscroll: &mut usize) -> Window {
    let rows = pods.iter().map(|p| {
        vec![
            cell_width(&p.namespace),
            cell_width(&p.name),
            cell_width(&p.ready),
            cell_width(&p.phase),
            p.restarts.to_string().len(),
            cell_width(&p.controlled_by),
            cell_width(&p.node),
            cell_width(&p.qos),
            cell_width(&p.age),
            p.containers.len() * 2,
        ]
    });
    layout_table(&POD_HEADERS, rows, table_width.saturating_sub(2), None, hscroll)
}

/// Which pod row sits under an absolute terminal position, restricted to
/// the CONTAINERS column specifically — hovering anywhere else in the row
/// shouldn't trigger the popup, only the dots themselves. Reuses
/// `Table`'s own column constraints through a real `Layout` solve (same
/// widths, same default 1-cell `column_spacing`) rather than
/// hand-guessing pixel math that could silently drift out of sync with
/// what's actually rendered.
/// Which data row of a bordered table (border, header, rows, border) a
/// screen row falls on, given the table's own scroll `offset`.
pub fn list_row_at(table_area: Rect, offset: usize, row_count: usize, row: u16) -> Option<usize> {
    let first = table_area.y.saturating_add(2);
    let last = (table_area.y + table_area.height).saturating_sub(1); // the bottom border
    if row < first || row >= last {
        return None;
    }
    let index = offset + usize::from(row - first);
    (index < row_count).then_some(index)
}

pub fn row_at(frame_area: Rect, pods: &[PodRow], hscroll: usize, table_state: &TableState, row_count: usize, column: u16, row: u16) -> Option<usize> {
    let table_area = frame_area;

    let inner = Rect {
        x: table_area.x.saturating_add(1),
        y: table_area.y.saturating_add(2), // border + header
        width: table_area.width.saturating_sub(2),
        height: table_area.height.saturating_sub(3), // top border + header + bottom border
    };

    if row < inner.y || row >= inner.y + inner.height {
        return None;
    }

    let window = pod_window(pods, table_area.width, &mut { hscroll });
    let columns = Layout::horizontal(window.constraints.clone()).spacing(COLUMN_GAP).split(inner);
    // CONTAINERS is the last column; nothing to hover if it's scrolled away.
    let containers_col = if window.range().end == POD_HEADERS.len() { columns.last()? } else { return None };
    if column < containers_col.x || column >= containers_col.x + containers_col.width {
        return None;
    }

    let offset = table_state.offset();
    let index = offset + usize::from(row - inner.y);
    (index < row_count).then_some(index)
}

/// Green when every desired replica is ready (`3/3`), yellow when some
/// aren't (`0/1`, `2/3`), grey when nothing is desired (`0/0`).
pub(super) fn ready_color(ready: &str) -> Color {
    let mut parts = ready.split('/').filter_map(|p| p.parse::<i64>().ok());
    match (parts.next(), parts.next()) {
        (Some(_), Some(0)) => Color::DarkGray,
        (Some(have), Some(want)) if have >= want => Color::Green,
        _ => Color::Yellow,
    }
}

/// A pod's READY colour: like a Deployment's, except a pod that ran to
/// completion (a finished Job's `0/1 Succeeded`) is grey, not "not ready".
pub(super) fn pod_ready_color(ready: &str, phase: &str) -> Color {
    if phase == "Succeeded" { Color::DarkGray } else { ready_color(ready) }
}

pub(super) fn draw_deployment_table(frame: &mut Frame, area: Rect, deployments: &[DeploymentRow], table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, dimmed: bool) {
    let border_style = theme_border(dimmed);
    let cell_style = theme_row(dimmed);

    const HEADERS: [&str; 6] = ["NAMESPACE", "NAME", "READY", "UP-TO-DATE", "AVAILABLE", "AGE"];
    let window = layout_table(
        &HEADERS,
        deployments.iter().map(|d| vec![cell_width(&d.namespace), cell_width(&d.name), cell_width(&d.ready), d.up_to_date.to_string().len(), d.available.to_string().len(), cell_width(&d.age)]),
        area.width.saturating_sub(2),
        None,
        hscroll,
    );
    let header = header_row(&HEADERS, sort, dimmed, &window);

    let rows = deployments.iter().map(|d| {
        Row::new(window.slice(vec![
            Cell::from(highlight_fuzzy(&d.namespace, search.text, cell_style)),
            Cell::from(highlight_fuzzy(&d.name, search.text, cell_style)),
            Cell::from(d.ready.clone()).style(if dimmed { cell_style } else { Style::default().fg(ready_color(&d.ready)) }),
            Cell::from(d.up_to_date.to_string()).style(cell_style),
            Cell::from(d.available.to_string()).style(cell_style),
            Cell::from(d.age.clone()).style(cell_style),
        ]))
    });

    let title = table_title("Deployments", deployments.len(), search, &window, dimmed);

    let table = Table::new(select_rows(rows, table_state.selected(), dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
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

/// kubectl's own convention: ",SchedulingDisabled" is appended to STATUS
/// rather than being a separate column.
fn node_status_text(n: &NodeRow) -> String {
    match (n.ready, n.schedulable) {
        (true, true) => "Ready".to_string(),
        (true, false) => "Ready,SchedulingDisabled".to_string(),
        (false, true) => "NotReady".to_string(),
        (false, false) => "NotReady,SchedulingDisabled".to_string(),
    }
}

pub(super) fn draw_nodes_table(frame: &mut Frame, area: Rect, nodes: &[NodeRow], table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, dimmed: bool) {
    let muted = dim_style();
    let border_style = theme_border(dimmed);
    let cell_style = theme_row(dimmed);

    const HEADERS: [&str; 9] = ["NAME", "STATUS", "ROLES", "TAINTS", "CPU", "MEMORY", "PODS", "AGE", "VERSION"];
    // `[▓▓▓▓▓▓▓▓▓▓] 100%` — the usage bars are a fixed width.
    const BAR_WIDTH: usize = 17;
    let window = layout_table(
        &HEADERS,
        nodes.iter().map(|n| {
            vec![
                cell_width(&n.name),
                cell_width(&node_status_text(n)),
                cell_width(&n.roles),
                n.taints.to_string().len(),
                BAR_WIDTH,
                BAR_WIDTH,
                cell_width(&format!("{}/{}", n.pod_count, n.pod_capacity)),
                cell_width(&n.age),
                cell_width(&n.version),
            ]
        }),
        area.width.saturating_sub(2),
        None,
        hscroll,
    );
    let header = header_row(&HEADERS, sort, dimmed, &window);

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
        let status = node_status_text(n);
        Row::new(window.slice(vec![
            Cell::from(highlight_fuzzy(&n.name, search.text, cell_style)),
            Cell::from(status).style(status_style),
            Cell::from(n.roles.clone()).style(cell_style),
            Cell::from(n.taints.to_string()).style(cell_style),
            Cell::from(usage_bar(n.cpu_millicores, n.cpu_capacity, dimmed)),
            Cell::from(usage_bar(n.memory_bytes, n.memory_capacity, dimmed)),
            Cell::from(format!("{}/{}", n.pod_count, n.pod_capacity)).style(cell_style),
            Cell::from(n.age.clone()).style(cell_style),
            Cell::from(n.version.clone()).style(cell_style),
        ]))
    });

    let title = table_title("Nodes", nodes.len(), search, &window, dimmed);

    let table = Table::new(select_rows(rows, table_state.selected(), dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
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

pub(super) fn draw_generic_table(frame: &mut Frame, area: Rect, rows: &[GenericRow], label: &str, kind_headers: &[&'static str], table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, dimmed: bool) {
    let border_style = theme_border(dimmed);
    let cell_style = theme_row(dimmed);

    let show_namespace = any_row_has_namespace(rows);

    // NAMESPACE (if any row has one), NAME, the kind's own columns, AGE.
    let mut headers: Vec<&str> = Vec::new();
    if show_namespace {
        headers.push("NAMESPACE");
    }
    headers.push("NAME");
    headers.extend(kind_headers.iter().copied());
    headers.push("AGE");

    let window = layout_table(
        &headers,
        rows.iter().map(|r| {
            let mut widths = Vec::with_capacity(headers.len());
            if show_namespace {
                widths.push(cell_width(&r.namespace));
            }
            widths.push(cell_width(&r.name));
            widths.extend(r.extras.iter().map(|c| cell_width(&c.text)));
            widths.push(cell_width(&r.age));
            widths
        }),
        area.width.saturating_sub(2),
        None,
        hscroll,
    );
    let header = header_row(&headers, sort, dimmed, &window);

    let table_rows = rows.iter().map(|r| {
        let mut cells = Vec::with_capacity(headers.len());
        if show_namespace {
            cells.push(Cell::from(highlight_fuzzy(&r.namespace, search.text, cell_style)));
        }
        cells.push(Cell::from(highlight_fuzzy(&r.name, search.text, cell_style)));
        cells.extend(r.extras.iter().map(|c| Cell::from(c.text.clone()).style(tone_style(c.tone, dimmed))));
        cells.push(Cell::from(r.age.clone()).style(cell_style));
        Row::new(window.slice(cells))
    });

    let title = table_title(label, rows.len(), search, &window, dimmed);

    let table = Table::new(select_rows(table_rows, table_state.selected(), dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
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
pub(super) fn draw_crd_list_table(frame: &mut Frame, area: Rect, crds: &[(usize, CrdInfo)], heading: &str, table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, dimmed: bool) {
    let border_style = theme_border(dimmed);
    let cell_style = theme_row(dimmed);

    const HEADERS: [&str; 3] = ["GROUP", "KIND", "SCOPE"];
    let window = layout_table(
        &HEADERS,
        crds.iter().map(|(_, c)| vec![cell_width(c.group), cell_width(c.kind), cell_width("Namespaced")]),
        area.width.saturating_sub(2),
        None,
        hscroll,
    );
    let header = header_row(&HEADERS, sort, dimmed, &window);

    let rows = crds.iter().map(|(_, c)| {
        Row::new(window.slice(vec![
            Cell::from(highlight_fuzzy(c.group, search.text, cell_style)),
            Cell::from(highlight_fuzzy(c.kind, search.text, cell_style)),
            Cell::from(if c.namespaced { "Namespaced" } else { "Cluster" }).style(cell_style),
        ]))
    });

    let title = table_title(heading, crds.len(), search, &window, dimmed);

    let table = Table::new(select_rows(rows, table_state.selected(), dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .highlight_symbol("");

    frame.render_stateful_widget(table, area, table_state);
}

#[cfg(test)]
mod generic_table_tests {
    use super::*;

    #[test]
    fn a_click_lands_on_the_row_under_it() {
        let area = Rect { x: 0, y: 2, width: 80, height: 10 }; // border 2, header 3, rows 4..=10, border 11
        assert_eq!(list_row_at(area, 0, 20, 4), Some(0));
        assert_eq!(list_row_at(area, 5, 20, 6), Some(7));
        assert_eq!(list_row_at(area, 0, 20, 2), None, "the border");
        assert_eq!(list_row_at(area, 0, 20, 3), None, "the header");
        assert_eq!(list_row_at(area, 0, 20, 11), None, "the bottom border");
        assert_eq!(list_row_at(area, 0, 2, 8), None, "below the last row");
    }

    #[test]
    fn a_completed_pod_is_grey_not_unready() {
        assert_eq!(pod_ready_color("0/1", "Succeeded"), Color::DarkGray);
        assert_eq!(pod_ready_color("0/1", "Running"), Color::Yellow);
        assert_eq!(pod_ready_color("2/2", "Running"), Color::Green);
    }

    #[test]
    fn ready_is_green_when_complete_yellow_when_not_grey_when_nothing_is_wanted() {
        assert_eq!(ready_color("1/1"), Color::Green);
        assert_eq!(ready_color("3/3"), Color::Green);
        assert_eq!(ready_color("0/1"), Color::Yellow);
        assert_eq!(ready_color("2/3"), Color::Yellow);
        assert_eq!(ready_color("0/0"), Color::DarkGray);
        assert_eq!(ready_color("junk"), Color::Yellow);
    }

    fn row(namespace: &str) -> GenericRow {
        GenericRow { namespace: namespace.to_string(), name: "x".to_string(), age: "1d".to_string(), age_secs: 0, extras: Vec::new(), status: None, uid: String::new(), owners: Vec::new() }
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
