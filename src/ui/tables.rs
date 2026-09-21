//! The resource list tables (pods, deployments, nodes, generic, CRD picker) and their row helpers.

use super::*;

/// A `Terminated` container isn't always a problem: a Job that ran to completion
/// has reason "Completed" and gets the blue k9s/kubectl use. Red is for errors.
pub(super) fn container_dot(c: &ContainerInfo) -> (&'static str, Color) {
    match c.status {
        ContainerStatusKind::Running => ("●", theme().ok),
        ContainerStatusKind::Waiting => ("●", theme().warn),
        ContainerStatusKind::Terminated if c.reason.as_deref() == Some("Completed") => ("●", theme().key),
        ContainerStatusKind::Terminated => ("●", theme().bad),
        ContainerStatusKind::Unknown => ("●", theme().text_soft),
    }
}

/// Compact form: just the colored dots, used for every row except the
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

/// A real floating popup, positioned right next to the cursor, "in
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

    let block = Block::default().borders(Borders::ALL).border_set(border_set()).title(format!("{}/{}", pod.namespace, pod.name));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The rows on screen out of a table of many, so only those get built.
struct Visible {
    start: usize,
    end: usize,
}

impl Visible {
    fn range(&self) -> std::ops::Range<usize> {
        self.start..self.end
    }
}

/// Settles the scroll offset so the selection is on screen, and names the rows to build.
fn visible(state: &mut TableState, total: usize, area: Rect) -> Visible {
    let height = usize::from(area.height.saturating_sub(3)).max(1);
    let mut offset = state.offset().min(total.saturating_sub(1));
    if let Some(selected) = state.selected() {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }
    }
    *state.offset_mut() = offset;
    Visible { start: offset, end: (offset + height).min(total) }
}

/// Draws a table built from only the visible rows, keeping `state` in whole-list terms.
fn render_windowed(frame: &mut Frame, area: Rect, table: Table, state: &mut TableState, vis: &Visible) {
    let selected = state.selected().filter(|s| vis.range().contains(s)).map(|s| s - vis.start);
    let mut local = TableState::default().with_selected(selected);
    frame.render_stateful_widget(table, area, &mut local);
}

pub(super) fn draw_table(frame: &mut Frame, area: Rect, pods: &[PodRow], table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, marked: &HashSet<String>, wide: bool, dimmed: bool) {
    let border_style = list_border(dimmed);

    let window = pod_window(pods, area.width, hscroll, wide);
    let header = header_row(&pod_headers(wide), sort, dimmed, &window);
    let vis = visible(table_state, pods.len(), area);

    let rows = pods[vis.range()].iter().map(|p| {
        // The whole row wears its state: red when broken, orange while
        // starting, grey when finished.
        let tone = crate::k8s::status_tone(&p.phase);
        let cell_style = row_tone_style(tone, dimmed);
        let status_style = cell_style;
        let ready_style = if dimmed || tone != crate::k8s::describe::Tone::Plain { cell_style } else { Style::default().fg(pod_ready_color(&p.ready, &p.phase)) };
        let mut cells = vec![
            Cell::from(highlight_fuzzy(&p.namespace, search.text, cell_style)),
            Cell::from(highlight_fuzzy(&p.name, search.text, cell_style)),
            Cell::from(p.ready.clone()).style(ready_style),
            Cell::from(p.phase.clone()).style(status_style),
            Cell::from(p.restarts.to_string()).style(cell_style),
            Cell::from(p.controlled_by.clone()).style(cell_style),
            Cell::from(p.node.clone()).style(cell_style),
            Cell::from(p.qos.clone()).style(cell_style),
            Cell::from(p.age.clone()).style(cell_style),
        ];
        if wide {
            cells.push(Cell::from(p.ip.clone()).style(cell_style));
            cells.push(Cell::from(p.images.clone()).style(cell_style));
        }
        cells.push(Cell::from(containers_cell(&p.containers, dimmed)));
        Row::new(window.slice(cells))
    });

    let flags: Vec<bool> = pods[vis.range()].iter().map(|p| marked.contains(&mark_key(&p.namespace, &p.name))).collect();
    let selected_tone = table_state.selected().and_then(|i| pods.get(i)).map(|p| crate::k8s::status_tone(&p.phase)).unwrap_or(crate::k8s::describe::Tone::Plain);
    let title = table_title("Pods", pods.len(), &window, dimmed);

    let table = Table::new(mark_rows(rows, &flags, dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(with_search(Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title(title), search.text, search.editing, dimmed))
        .highlight_symbol("")
        .row_highlight_style(selection_style(selected_tone, dimmed));

    render_windowed(frame, area, table, table_state, &vis);
}

/// The pods table's headers; the wide view adds IP and IMAGES before
/// CONTAINERS, which stays last.
fn pod_headers(wide: bool) -> Vec<&'static str> {
    let mut headers = vec!["NAMESPACE", "NAME", "READY", "STATUS", "RESTARTS", "CONTROLLER", "NODE", "QOS", "AGE"];
    if wide {
        headers.extend(["IP", "IMAGES"]);
    }
    headers.push("CONTAINERS");
    headers
}

/// The pods table's visible columns, shared by drawing and by hover
/// hit-testing so they can't disagree about where CONTAINERS is.
fn pod_window(pods: &[PodRow], table_width: u16, hscroll: &mut usize, wide: bool) -> Window {
    let rows = pods.iter().map(|p| {
        let mut widths = vec![
            cell_width(&p.namespace),
            cell_width(&p.name),
            cell_width(&p.ready),
            cell_width(&p.phase),
            p.restarts.to_string().len(),
            cell_width(&p.controlled_by),
            cell_width(&p.node),
            cell_width(&p.qos),
            cell_width(&p.age),
        ];
        if wide {
            widths.push(cell_width(&p.ip));
            widths.push(cell_width(&p.images));
        }
        widths.push(p.containers.len() * 2);
        widths
    });
    layout_list(&pod_headers(wide), (pods.as_ptr() as usize, pods.len()), rows, table_width.saturating_sub(2), None, hscroll)
}

/// Which data row of a bordered table a screen row falls on, given its scroll `offset`.
pub fn list_row_at(table_area: Rect, offset: usize, row_count: usize, row: u16) -> Option<usize> {
    let first = table_area.y.saturating_add(2);
    let last = (table_area.y + table_area.height).saturating_sub(1); // the bottom border
    if row < first || row >= last {
        return None;
    }
    let index = offset + usize::from(row - first);
    (index < row_count).then_some(index)
}

/// Whether a terminal column is over the pods table's CONTROLLER column.
pub fn controller_at(frame_area: Rect, pods: &[PodRow], wide: bool, hscroll: usize, column: u16) -> bool {
    const CONTROLLER: usize = 5;
    let inner = Rect { x: frame_area.x.saturating_add(1), y: frame_area.y.saturating_add(2), width: frame_area.width.saturating_sub(2), height: frame_area.height.saturating_sub(3) };
    let window = pod_window(pods, frame_area.width, &mut { hscroll }, wide);
    let range = window.range();
    if !range.contains(&CONTROLLER) {
        return false;
    }
    let columns = Layout::horizontal(window.constraints.clone()).spacing(COLUMN_GAP).split(inner);
    columns.get(CONTROLLER - range.start).is_some_and(|c| column >= c.x && column < c.x + c.width)
}

/// Which pod row is under a terminal position, only over the CONTAINERS column so
/// the popup fires on the dots. It solves the table's `Layout` to match its widths.
pub fn row_at(frame_area: Rect, pods: &[PodRow], wide: bool, hscroll: usize, table_state: &TableState, row_count: usize, column: u16, row: u16) -> Option<usize> {
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

    let window = pod_window(pods, table_area.width, &mut { hscroll }, wide);
    let columns = Layout::horizontal(window.constraints.clone()).spacing(COLUMN_GAP).split(inner);
    // CONTAINERS is the last column; nothing to hover if it's scrolled away.
    let containers_col = if window.range().end == pod_headers(wide).len() { columns.last()? } else { return None };
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
        (Some(_), Some(0)) => theme().muted,
        (Some(have), Some(want)) if have >= want => theme().ok,
        _ => theme().warn,
    }
}

/// A pod's READY colour: like a Deployment's, except a pod that ran to
/// completion (a finished Job's `0/1 Succeeded`) is grey, not "not ready".
pub(super) fn pod_ready_color(ready: &str, phase: &str) -> Color {
    if matches!(phase, "Succeeded" | "Completed") { theme().muted } else { ready_color(ready) }
}

/// The state of a `have/want` ready count for colouring a whole row.
pub(super) fn ready_tone(ready: &str) -> crate::k8s::describe::Tone {
    use crate::k8s::describe::Tone;
    let mut parts = ready.split('/').filter_map(|p| p.parse::<i64>().ok());
    match (parts.next(), parts.next()) {
        (Some(0), Some(want)) if want > 0 => Tone::Bad,
        (Some(have), Some(want)) if have < want => Tone::Warn,
        _ => Tone::Plain,
    }
}

pub(super) fn draw_deployment_table(frame: &mut Frame, area: Rect, deployments: &[DeploymentRow], table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, marked: &HashSet<String>, wide: bool, dimmed: bool) {
    let border_style = list_border(dimmed);

    let mut headers = vec!["NAMESPACE", "NAME", "READY", "UP-TO-DATE", "AVAILABLE", "AGE"];
    if wide {
        headers.push("IMAGES");
    }
    let window = layout_list(
        &headers,
        (deployments.as_ptr() as usize, deployments.len()),
        deployments.iter().map(|d| {
            let mut widths = vec![cell_width(&d.namespace), cell_width(&d.name), cell_width(&d.ready), d.up_to_date.to_string().len(), d.available.to_string().len(), cell_width(&d.age)];
            if wide {
                widths.push(cell_width(&d.images));
            }
            widths
        }),
        area.width.saturating_sub(2),
        None,
        hscroll,
    );
    let header = header_row(&headers, sort, dimmed, &window);
    let vis = visible(table_state, deployments.len(), area);

    let rows = deployments[vis.range()].iter().map(|d| {
        // A deployment short of its replicas turns orange (red with none available).
        let cell_style = row_tone_style(ready_tone(&d.ready), dimmed);
        let mut cells = vec![
            Cell::from(highlight_fuzzy(&d.namespace, search.text, cell_style)),
            Cell::from(highlight_fuzzy(&d.name, search.text, cell_style)),
            Cell::from(d.ready.clone()).style(if dimmed || ready_tone(&d.ready) != crate::k8s::describe::Tone::Plain { cell_style } else { Style::default().fg(ready_color(&d.ready)) }),
            Cell::from(d.up_to_date.to_string()).style(cell_style),
            Cell::from(d.available.to_string()).style(cell_style),
            Cell::from(d.age.clone()).style(cell_style),
        ];
        if wide {
            cells.push(Cell::from(d.images.clone()).style(cell_style));
        }
        Row::new(window.slice(cells))
    });

    let flags: Vec<bool> = deployments[vis.range()].iter().map(|d| marked.contains(&mark_key(&d.namespace, &d.name))).collect();
    let selected_tone = table_state.selected().and_then(|i| deployments.get(i)).map(|d| ready_tone(&d.ready)).unwrap_or(crate::k8s::describe::Tone::Plain);
    let title = table_title("Deployments", deployments.len(), &window, dimmed);

    let table = Table::new(mark_rows(rows, &flags, dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(with_search(Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title(title), search.text, search.editing, dimmed))
        .highlight_symbol("")
        .row_highlight_style(selection_style(selected_tone, dimmed));

    render_windowed(frame, area, table, table_state, &vis);
}

/// A compact usage bar for a table cell: `▓▓▓░░░░░ 34%`, or gray `n/a` when
/// metrics-server isn't installed.
pub(super) fn usage_bar(used: Option<i64>, capacity: i64, dimmed: bool) -> Line<'static> {
    const WIDTH: usize = 10;
    let Some(used) = used else {
        return Line::styled("n/a", Style::default().fg(theme().muted));
    };
    let ratio = if capacity > 0 { (used as f64 / capacity as f64).clamp(0.0, 1.0) } else { 0.0 };
    let filled = (ratio * WIDTH as f64).round() as usize;
    let color = usage_color(ratio, dimmed);
    let bracket = if dimmed { dim_style() } else { Style::default().fg(theme().muted) };
    Line::from(vec![
        Span::styled("[", bracket),
        Span::styled("▓".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(WIDTH - filled), Style::default().fg(theme().muted)),
        Span::styled("]", bracket),
        Span::raw(format!(" {:.0}%", ratio * 100.0)),
    ])
}

pub(super) fn usage_color(ratio: f64, dimmed: bool) -> Color {
    if dimmed {
        theme().dim
    } else if ratio > 0.9 {
        theme().bad
    } else if ratio > 0.7 {
        theme().warn
    } else {
        theme().ok
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

/// A node's state for colouring its row: cordoned is orange, NotReady red.
fn node_tone(n: &NodeRow) -> crate::k8s::describe::Tone {
    use crate::k8s::describe::Tone;
    match (n.ready, n.schedulable) {
        (true, true) => Tone::Plain,
        (true, false) => Tone::Warn,
        (false, _) => Tone::Bad,
    }
}

/// A row's own colour when it has a notable state (broken, starting, finished).
fn generic_row_tone(r: &GenericRow) -> crate::k8s::describe::Tone {
    use crate::k8s::describe::Tone;
    match &r.status {
        Some((tone @ (Tone::Bad | Tone::Warn | Tone::Muted), _)) => *tone,
        _ => Tone::Plain,
    }
}

pub(super) fn draw_nodes_table(frame: &mut Frame, area: Rect, nodes: &[NodeRow], table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, marked: &HashSet<String>, wide: bool, dimmed: bool) {
    let border_style = list_border(dimmed);

    let mut headers = vec!["NAME", "STATUS", "ROLES", "TAINTS", "CPU", "MEMORY", "PODS", "AGE", "VERSION"];
    if wide {
        headers.extend(["INTERNAL-IP", "OS-IMAGE", "KERNEL", "RUNTIME"]);
    }
    // `[▓▓▓▓▓▓▓▓▓▓] 100%`, the usage bars are a fixed width.
    const BAR_WIDTH: usize = 17;
    let window = layout_list(
        &headers,
        (nodes.as_ptr() as usize, nodes.len()),
        nodes.iter().map(|n| {
            let mut widths = vec![
                cell_width(&n.name),
                cell_width(&node_status_text(n)),
                cell_width(&n.roles),
                n.taints.to_string().len(),
                BAR_WIDTH,
                BAR_WIDTH,
                cell_width(&format!("{}/{}", n.pod_count, n.pod_capacity)),
                cell_width(&n.age),
                cell_width(&n.version),
            ];
            if wide {
                widths.extend([cell_width(&n.internal_ip), cell_width(&n.os_image), cell_width(&n.kernel), cell_width(&n.runtime)]);
            }
            widths
        }),
        area.width.saturating_sub(2),
        None,
        hscroll,
    );
    let header = header_row(&headers, sort, dimmed, &window);
    let vis = visible(table_state, nodes.len(), area);

    let rows = nodes[vis.range()].iter().map(|n| {
        let tone = node_tone(n);
        let cell_style = row_tone_style(tone, dimmed);
        let status_style = cell_style;
        let status = node_status_text(n);
        let mut cells = vec![
            Cell::from(highlight_fuzzy(&n.name, search.text, cell_style)),
            Cell::from(status).style(status_style),
            Cell::from(n.roles.clone()).style(cell_style),
            Cell::from(n.taints.to_string()).style(cell_style),
            Cell::from(usage_bar(n.cpu_millicores, n.cpu_capacity, dimmed)),
            Cell::from(usage_bar(n.memory_bytes, n.memory_capacity, dimmed)),
            Cell::from(format!("{}/{}", n.pod_count, n.pod_capacity)).style(cell_style),
            Cell::from(n.age.clone()).style(cell_style),
            Cell::from(n.version.clone()).style(cell_style),
        ];
        if wide {
            for text in [&n.internal_ip, &n.os_image, &n.kernel, &n.runtime] {
                cells.push(Cell::from(text.clone()).style(cell_style));
            }
        }
        Row::new(window.slice(cells))
    });

    let flags: Vec<bool> = nodes[vis.range()].iter().map(|n| marked.contains(&mark_key("-", &n.name))).collect();
    let selected_tone = table_state.selected().and_then(|i| nodes.get(i)).map(node_tone).unwrap_or(crate::k8s::describe::Tone::Plain);
    let title = table_title("Nodes", nodes.len(), &window, dimmed);

    let table = Table::new(mark_rows(rows, &flags, dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(with_search(Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title(title), search.text, search.editing, dimmed))
        .highlight_symbol("")
        .row_highlight_style(selection_style(selected_tone, dimmed));

    render_windowed(frame, area, table, table_state, &vis);
}

/// Whether any row has a namespace. Cluster-scoped kinds show `-` everywhere, so
/// `draw_generic_table` drops that column when this is false.
pub(super) fn any_row_has_namespace(rows: &[GenericRow]) -> bool {
    rows.iter().any(|r| r.namespace != "-")
}

pub(super) fn draw_generic_table(frame: &mut Frame, area: Rect, rows: &[GenericRow], label: &str, kind_headers: &[&'static str], table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, marked: &HashSet<String>, wide: bool, dimmed: bool) {
    let border_style = list_border(dimmed);

    let show_namespace = any_row_has_namespace(rows);

    // NAMESPACE (if any row has one), NAME, the kind's own columns, AGE.
    let mut headers: Vec<&str> = Vec::new();
    if show_namespace {
        headers.push("NAMESPACE");
    }
    headers.push("NAME");
    headers.extend(kind_headers.iter().copied());
    headers.push("AGE");
    if wide {
        headers.push("LABELS");
    }

    let window = layout_list(
        &headers,
        (rows.as_ptr() as usize, rows.len()),
        rows.iter().map(|r| {
            let mut widths = Vec::with_capacity(headers.len());
            if show_namespace {
                widths.push(cell_width(&r.namespace));
            }
            widths.push(cell_width(&r.name));
            widths.extend(r.extras.iter().map(|c| cell_width(&c.text)));
            widths.push(cell_width(&r.age));
            if wide {
                widths.push(cell_width(&r.labels));
            }
            widths
        }),
        area.width.saturating_sub(2),
        None,
        hscroll,
    );
    let header = header_row(&headers, sort, dimmed, &window);
    let vis = visible(table_state, rows.len(), area);

    let table_rows = rows[vis.range()].iter().map(|r| {
        use crate::k8s::describe::Tone;
        let row_tone = generic_row_tone(r);
        let cell_style = row_tone_style(row_tone, dimmed);
        let mut cells = Vec::with_capacity(headers.len());
        if show_namespace {
            cells.push(Cell::from(highlight_fuzzy(&r.namespace, search.text, cell_style)));
        }
        cells.push(Cell::from(highlight_fuzzy(&r.name, search.text, cell_style)));
        cells.extend(r.extras.iter().map(|c| Cell::from(c.text.clone()).style(if c.tone == Tone::Plain { cell_style } else { tone_style(c.tone, dimmed) })));
        cells.push(Cell::from(r.age.clone()).style(cell_style));
        if wide {
            cells.push(Cell::from(r.labels.clone()).style(cell_style));
        }
        Row::new(window.slice(cells))
    });

    let flags: Vec<bool> = rows[vis.range()].iter().map(|r| marked.contains(&mark_key(&r.namespace, &r.name))).collect();
    let selected_tone = table_state.selected().and_then(|i| rows.get(i)).map(generic_row_tone).unwrap_or(crate::k8s::describe::Tone::Plain);
    let title = table_title(label, rows.len(), &window, dimmed);

    let table = Table::new(mark_rows(table_rows, &flags, dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(with_search(Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title(title), search.text, search.editing, dimmed))
        .highlight_symbol("")
        .row_highlight_style(selection_style(selected_tone, dimmed));

    render_windowed(frame, area, table, table_state, &vis);
}

/// The Custom Resources picker: every discovered CRD kind, sorted by GROUP so
/// same-group kinds sit together. Enter starts watching the kind; nothing is
/// live-watched until then.
pub(super) fn draw_crd_list_table(frame: &mut Frame, area: Rect, crds: &[(usize, CrdInfo)], heading: &str, table_state: &mut TableState, search: Search, sort: SortState, hscroll: &mut usize, dimmed: bool) {
    let border_style = list_border(dimmed);
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

    let title = table_title(heading, crds.len(), &window, dimmed);

    let table = Table::new(mark_rows(rows, &[], dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(with_search(Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title(title), search.text, search.editing, dimmed))
        .highlight_symbol("")
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, dimmed));

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
        assert_eq!(pod_ready_color("0/1", "Completed"), theme().muted);
        assert_eq!(pod_ready_color("0/1", "Succeeded"), theme().muted);
        assert_eq!(pod_ready_color("0/1", "Running"), theme().warn);
        assert_eq!(pod_ready_color("2/2", "Running"), theme().ok);
    }

    #[test]
    fn ready_is_green_when_complete_yellow_when_not_grey_when_nothing_is_wanted() {
        assert_eq!(ready_color("1/1"), theme().ok);
        assert_eq!(ready_color("3/3"), theme().ok);
        assert_eq!(ready_color("0/1"), theme().warn);
        assert_eq!(ready_color("2/3"), theme().warn);
        assert_eq!(ready_color("0/0"), theme().muted);
        assert_eq!(ready_color("junk"), theme().warn);
    }

    fn row(namespace: &str) -> GenericRow {
        GenericRow { namespace: namespace.to_string(), name: "x".to_string(), age: "1d".to_string(), age_secs: 0, extras: Vec::new(), status: None, uid: String::new(), owners: Vec::new(), labels: String::new() }
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
