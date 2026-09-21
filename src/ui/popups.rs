//! Modal popups: search/command/context/notice bars, node detail, events, containers, resources.

use super::*;

/// The `:` command line, k9s-style: a bar right under the header, above
/// the list (which `draw` pushes down to make room), with the live
/// autocomplete as a short dropdown hanging off it. The best match's
/// remaining letters show dimmed after the cursor.
pub(super) fn draw_command_line(frame: &mut Frame, bar: Rect, input: &str, suggestions: &[String], selected: usize) {
    frame.render_widget(Clear, bar);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(bar);
    frame.render_widget(block, bar);

    // "namespaces (ns)": complete against the name, not the alias note.
    let ghost = suggestions
        .get(selected)
        .and_then(|label| label.split(" (").next())
        .and_then(|name| name.strip_prefix(input))
        .unwrap_or("");
    let line = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Yellow)),
        Span::styled(input.to_string(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("▏", Style::default().fg(Color::Yellow)),
        Span::styled(ghost.to_string(), Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(line), inner);

    if suggestions.is_empty() {
        return;
    }
    let below = frame.area().bottom().saturating_sub(bar.bottom());
    let height = (suggestions.len() as u16 + 2).min(below);
    if height < 3 {
        return;
    }
    let width = (suggestions.iter().map(|s| s.chars().count()).max().unwrap_or(0) as u16 + 4).max(24).min(bar.width);
    let list = Rect { x: bar.x, y: bar.bottom(), width, height };
    frame.render_widget(Clear, list);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(list);
    frame.render_widget(block, list);
    let rows = Layout::vertical([Constraint::Length(1)].repeat(inner.height.max(1) as usize)).split(inner);
    for (i, label) in suggestions.iter().enumerate() {
        let Some(row) = rows.get(i) else { break };
        let style = if i == selected {
            Style::default().bg(SELECT_BG).fg(Color::Black).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ROW_FG)
        };
        frame.render_widget(Paragraph::new(Span::styled(format!(" {label:<w$}", w = row.width.saturating_sub(1) as usize), style)), *row);
    }
}

/// The `:ctx` / `C` context browser — same full-size table as the
/// Events browser (and the same geometry, so `event_row_at` hit-tests
/// its rows too). `/` live-filters by name.
pub(super) fn draw_context_popup(
    frame: &mut Frame,
    items: &[(String, String, bool)],
    total: usize,
    filter: &str,
    editing: bool,
    state: &mut TableState,
    error: Option<&str>,
    sort: SortState,
) {
    let area = centered_rect(94, 88, frame.area());
    frame.render_widget(Clear, area);

    const HEADERS: [&str; 3] = ["CONTEXT", "CLUSTER", "STATUS"];
    let window = layout_table(
        &HEADERS,
        items.iter().map(|(name, cluster, _)| vec![cell_width(name), cell_width(cluster), cell_width("current")]),
        area.width.saturating_sub(2),
        None,
        &mut 0,
    );
    let header = header_row(&HEADERS, sort, false, &window);
    let rows = items.iter().map(|(name, cluster, current)| {
        Row::new(window.slice(vec![
            Cell::from(highlight_fuzzy(name, filter, Style::default().add_modifier(Modifier::BOLD))),
            Cell::from(highlight_fuzzy(cluster, filter, Style::default())),
            Cell::from(if *current { "current" } else { "" }).style(Style::default().fg(Color::Green)),
        ]))
    });

    let mut title = colored_slash_title(&format!("Contexts ({}/{total})", items.len()));
    if let Some(span) = search_span(filter, editing, false) {
        title.push_span(span);
    }
    if let Some(err) = error {
        title.push_span(Span::styled(format!("  —  {err}"), Style::default().fg(Color::Red)));
    }

    let table = Table::new(mark_rows(rows, &[], false), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(false))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(title))
        .highlight_symbol("")
        .row_highlight_style(selection_style(false));

    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(items.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, state);
}

/// The `n` namespace picker — the same full-size table as the context
/// browser (same geometry, so `event_row_at` hit-tests its rows too).
/// Enter on a row moves on to choosing that namespace's number key.
pub(super) fn draw_namespace_picker(
    frame: &mut Frame,
    items: &[(String, Option<usize>)],
    total: usize,
    filter: &str,
    editing: bool,
    state: &mut TableState,
    sort: SortState,
) {
    let area = centered_rect(94, 88, frame.area());
    frame.render_widget(Clear, area);

    const HEADERS: [&str; 2] = ["NAMESPACE", "KEY"];
    let window = layout_table(&HEADERS, items.iter().map(|(name, _)| vec![cell_width(name), 1]), area.width.saturating_sub(2), None, &mut 0);
    let header = header_row(&HEADERS, sort, false, &window);
    let rows = items.iter().map(|(name, key)| {
        Row::new(window.slice(vec![
            Cell::from(highlight_fuzzy(name, filter, Style::default().add_modifier(Modifier::BOLD))),
            Cell::from(key.map(|k| k.to_string()).unwrap_or_default()).style(Style::default().fg(Color::Rgb(240, 160, 110))),
        ]))
    });

    let mut title = colored_slash_title(&format!("Choose the namespace to filter by ({}/{total})", items.len()));
    if let Some(span) = search_span(filter, editing, false) {
        title.push_span(span);
    }

    let table = Table::new(mark_rows(rows, &[], false), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(false))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(title))
        .highlight_symbol("")
        .row_highlight_style(selection_style(false));

    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(items.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, state);
}

/// The key picker: `0` (always "all", not assignable) and keys 1-9 with
/// what each holds; the highlighted key is where Enter puts the namespace.
pub(super) fn draw_slots_popup(frame: &mut Frame, namespace: &str, slots: &[Option<String>], selected: usize) {
    let bar = centered_box(frame.area(), 2 + 1 + 9 + 1 + 1);
    frame.render_widget(Clear, bar);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(format!(" Choose the key for '{namespace}' "));
    let inner = block.inner(bar);
    frame.render_widget(block, bar);

    let rows = Layout::vertical([Constraint::Length(1)].repeat(inner.height.max(1) as usize)).split(inner);
    let key_style = Style::default().fg(Color::Rgb(240, 160, 110));
    let fixed = Style::default().fg(Color::DarkGray);

    if let Some(row) = rows.first() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled("<0> ", fixed), Span::styled("all", fixed)])),
            *row,
        );
    }
    for (i, slot) in slots.iter().enumerate() {
        let Some(row) = rows.get(i + 1) else { break };
        let text = match slot.as_deref() {
            Some(ns) => ns.to_string(),
            None => "—".to_string(),
        };
        let line = if i == selected {
            let style = Style::default().bg(SELECT_BG).fg(Color::Black).add_modifier(Modifier::BOLD);
            Line::styled(format!("{:<width$}", format!("<{}> {text}", i + 1), width = row.width as usize), style)
        } else {
            Line::from(vec![Span::styled(format!("<{}> ", i + 1), key_style), Span::raw(text)])
        };
        frame.render_widget(Paragraph::new(line), *row);
    }
    if let Some(row) = rows.get(slots.len() + 2) {
        frame.render_widget(
            Paragraph::new(Line::styled("1-9 assign  d clear", Style::default().fg(Color::DarkGray))),
            *row,
        );
    }
}

/// A small centered message box — green-bordered for success, red for
/// an error. Sized to the text so a one-liner doesn't get a huge box.
pub(super) fn draw_notice_popup(frame: &mut Frame, text: &str, error: bool) {
    let full = frame.area();
    let width = (full.width * 3 / 5).max(30).min(full.width);
    let inner_w = width.saturating_sub(2).max(1) as usize;
    let lines: usize = text.lines().map(|l| l.chars().count().div_ceil(inner_w).max(1)).sum::<usize>().max(1);
    let height = (lines as u16 + 2).min(full.height);
    let area = Rect {
        x: full.x + full.width.saturating_sub(width) / 2,
        y: full.y + full.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, area);
    let color = if error { Color::Red } else { Color::Green };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .title(if error { "Failed" } else { "Done" });
    frame.render_widget(Paragraph::new(text.to_string()).wrap(Wrap { trim: false }).block(block), area);
}

/// A small centred box with a title and body lines, for the question popups.
fn small_popup(frame: &mut Frame, title: &str, color: Color, body: Vec<Line<'static>>) {
    let full = frame.area();
    let width = (full.width * 3 / 5).max(30).min(full.width);
    let height = (body.len() as u16 + 2).min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 2, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(color)).title(title.to_string());
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }).block(block), area);
}

pub(super) fn draw_confirm_popup(frame: &mut Frame, text: &str) {
    let key = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let body = vec![
        Line::from(text.to_string()),
        Line::from(vec![Span::styled("y", key), Span::raw(" yes   "), Span::styled("n", key), Span::raw(" no")]),
    ];
    small_popup(frame, "Confirm", Color::Yellow, body);
}

/// The port-forward dialog, laid out like k9s's: labelled fields, a warning
/// when the port is a guess, and OK / Cancel.
pub(super) fn draw_port_forward_popup(frame: &mut Frame, title: &str, form: &crate::portforward::PortForm) {
    use crate::portforward::Field;
    let full = frame.area();
    let width = (full.width * 3 / 5).clamp(44, full.width.max(1)).min(full.width);
    let height = 11u16.min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 3, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme_border(false))
        .title(Line::styled("<PortForward>", Style::default().fg(Color::Rgb(120, 230, 230)).add_modifier(Modifier::BOLD)).centered());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label = Style::default().fg(Color::Rgb(214, 146, 120));
    let value = Style::default().fg(Color::Rgb(226, 232, 240));
    let hint = Style::default().fg(MUTED_FG);
    let field = |name: &str, text: &str, placeholder: &str, focused: bool| {
        let shown = if text.is_empty() && !focused { Span::styled(placeholder.to_string(), hint) } else { Span::styled(format!("{text}{}", if focused { "▏" } else { "" }), value) };
        Line::from(vec![Span::styled(format!(" {name:<16}"), label), shown])
    };
    let button = |name: &str, focused: bool| {
        let style = if focused { Style::default().bg(SELECT_BG).fg(Color::Black).add_modifier(Modifier::BOLD) } else { Style::default().fg(value.fg.unwrap_or(Color::White)) };
        Span::styled(format!(" {name} "), style)
    };
    let mut lines = vec![
        Line::styled(title.to_string(), value.add_modifier(Modifier::BOLD)).centered(),
        Line::raw(""),
        field("Container Port:", &form.container, "Enter the container port", form.focus == Field::Container),
        field("Local Port:", &form.local, "Enter a local port", form.focus == Field::Local),
        field("Address:", &form.address, "localhost", form.focus == Field::Address),
        Line::raw(""),
    ];
    let note = match (&form.error, form.warning()) {
        (Some(e), _) => Some(Line::styled(format!(" {e}"), Style::default().fg(BAD_FG))),
        (None, Some(w)) => Some(Line::styled(format!(" ⚠ {w}"), Style::default().fg(WARN_FG))),
        _ => None,
    };
    lines.push(note.unwrap_or_else(|| Line::raw("")));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![button("OK", form.focus == Field::Ok), Span::raw("   "), button("Cancel", form.focus == Field::Cancel)]).centered());
    frame.render_widget(Paragraph::new(lines), inner);
}

/// One YAML line coloured by role: keys blue, the rest plain, list dashes muted.
fn yaml_line(line: &str) -> Line<'static> {
    let indent = line.len() - line.trim_start().len();
    let (lead, rest) = line.split_at(indent);
    let (dash, rest) = match rest.strip_prefix("- ") {
        Some(after) => ("- ", after),
        None => ("", rest),
    };
    let key_style = Style::default().fg(Color::Rgb(84, 148, 255));
    let plain = Style::default().fg(Color::Rgb(200, 205, 218));
    let mut spans = vec![Span::raw(lead.to_string()), Span::styled(dash.to_string(), Style::default().fg(MUTED_FG))];
    match rest.split_once(": ").or_else(|| rest.strip_suffix(':').map(|k| (k, ""))) {
        Some((key, value)) if !key.contains(' ') || key.starts_with('"') => {
            spans.push(Span::styled(key.to_string(), key_style));
            spans.push(Span::styled(":", Style::default().fg(MUTED_FG)));
            if !value.is_empty() {
                spans.push(Span::styled(format!(" {value}"), plain));
            }
        }
        _ => spans.push(Span::styled(rest.to_string(), plain)),
    }
    Line::from(spans)
}

/// A manifest as scrollable text over the whole body of the screen.
pub(super) fn draw_yaml_popup(frame: &mut Frame, title: &str, text: &str, scroll: usize) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme_border(false))
        .title(Line::styled(format!(" {title} "), Style::default().fg(Color::Rgb(120, 230, 230)).add_modifier(Modifier::BOLD)).centered());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines: Vec<Line> = text.lines().skip(scroll).take(usize::from(inner.height)).map(yaml_line).collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_prompt_popup(frame: &mut Frame, title: &str, value: &str, hint: &str) {
    let mut body = vec![Line::from(vec![Span::raw("> "), Span::styled(format!("{value}▏"), Style::default().fg(Color::Yellow))])];
    if !hint.is_empty() {
        body.push(Line::styled(hint.to_string(), Style::default().fg(Color::DarkGray)));
    }
    small_popup(frame, title, Color::Cyan, body);
}

/// Freelens-style node drill-down: that node's own CPU/Memory/Pods
/// gauges (reusing the exact same `draw_gauge` the Overview panel uses)
/// above the pods actually scheduled on it (reusing the exact same pod
/// table Pods' own list view uses, including its container dots).
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_node_detail_popup(
    frame: &mut Frame,
    name: &str,
    cpu_usage: Option<i64>,
    cpu_capacity: i64,
    memory_usage: Option<i64>,
    memory_capacity: i64,
    pod_capacity: i64,
    info: Option<&crate::k8s::NodeDetailInfo>,
    pods: &[PodRow],
    state: &mut TableState,
    sort: SortState,
    search: Search,
    dimmed: bool,
) {
    let area = centered_rect(94, 92, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(format!("Node: {name}"));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let info_h = info.map(node_info_height).unwrap_or(0);
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Length(info_h), Constraint::Min(0)]).split(inner);

    match (cpu_usage, memory_usage) {
        (Some(cpu), Some(mem)) => {
            let lines = Layout::vertical([Constraint::Length(1); 3]).split(chunks[0]);
            draw_meter(frame, lines[0], "CPU", cpu as f64, cpu_capacity as f64, |v| format!("{:.2} cores", v / 1000.0), dimmed);
            draw_meter(frame, lines[1], "Memory", mem as f64, memory_capacity as f64, format_bytes, dimmed);
            draw_meter(frame, lines[2], "Pods", pods.len() as f64, pod_capacity as f64, |v| format!("{v:.0}"), dimmed);
        }
        _ => {
            let text = Paragraph::new(Line::styled("metrics unavailable", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)))
                .alignment(Alignment::Center);
            frame.render_widget(text, chunks[0]);
        }
    }

    if let Some(info) = info {
        draw_node_info_panel(frame, chunks[1], info, dimmed);
    }

    draw_table(frame, chunks[2], pods, state, if dimmed { Search::default() } else { search }, if dimmed { SortState::default() } else { sort }, &mut 0, &HashSet::new(), false, dimmed);
}

/// How tall the node-info panel is: three summary lines, a blank
/// separator, the conditions header + one row per condition, then (if
/// there are any) a blank separator and a taints line — computed once so
/// sizing and drawing can't drift apart, same pattern as the Overview's
/// `top_area_height`/`issues_content_height`.
pub(super) fn node_info_height(info: &crate::k8s::NodeDetailInfo) -> u16 {
    let base = 3 + 1 + 1 + info.conditions.len() as u16;
    if info.taints.is_empty() { base } else { base + 1 + info.taints.len() as u16 }
}

/// Freelens-style node summary: schedulability/roles/version, network
/// addresses and host OS/runtime details, the full condition list
/// (healthy conditions included — unlike the Cluster Issues panel, this
/// is a diagnostic view), and any taints.
pub(super) fn draw_node_info_panel(frame: &mut Frame, area: Rect, info: &crate::k8s::NodeDetailInfo, dimmed: bool) {
    let label = Style::default().fg(Color::DarkGray);
    let value = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let field = |l: &'static str, v: String| vec![Span::styled(format!("{l}: "), label), Span::styled(v, value)];

    let schedulable_text = if info.schedulable { "Schedulable".to_string() } else { "Cordoned".to_string() };
    let schedulable_style = if dimmed {
        dim_style()
    } else if info.schedulable {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let mut line1 = field("Roles", info.roles.clone());
    line1.push(Span::raw("   "));
    line1.extend(vec![Span::styled("Status: ", label), Span::styled(schedulable_text, schedulable_style)]);
    line1.push(Span::raw("   "));
    line1.extend(field("Kubelet", info.kubelet_version.clone()));

    let mut line2 = field("Internal IP", info.internal_ip.clone());
    line2.push(Span::raw("   "));
    line2.extend(field("External IP", info.external_ip.clone()));

    let mut line3 = field("OS", info.os_image.clone());
    line3.push(Span::raw("   "));
    line3.extend(field("Kernel", info.kernel_version.clone()));
    line3.push(Span::raw("   "));
    line3.extend(field("Runtime", info.container_runtime.clone()));

    let mut lines = vec![Line::from(line1), Line::from(line2), Line::from(line3), Line::raw("")];

    lines.push(Line::styled("CONDITIONS", value));
    for c in &info.conditions {
        let is_healthy = (c.type_ == "Ready") == (c.status == "True");
        let color = if dimmed {
            Color::Rgb(40, 40, 40)
        } else if is_healthy {
            Color::Green
        } else {
            Color::Red
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:<20}", c.type_), if dimmed { dim_style() } else { Style::default() }),
            Span::styled(format!("{:<8}", c.status), Style::default().fg(color)),
            Span::styled(c.reason.clone(), label),
        ]));
    }

    if !info.taints.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled("Taints: ", label), Span::styled(info.taints.join(", "), value)]));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// The full Events browser — every event (not capped, unlike the
/// dashboard preview), filterable by severity. Kubernetes only defines
/// `Normal`/`Warning` as event types, so that's the full set of filters;
/// there's no separate "Errors" bucket to add since the API doesn't have
/// one.
pub(super) fn draw_events_popup(
    frame: &mut Frame,
    events: &[EventEntry],
    filter: EventFilter,
    search: &str,
    editing: bool,
    state: &mut TableState,
    sort: SortState,
    dimmed: bool,
) {
    let area = centered_rect(94, 88, frame.area());
    frame.render_widget(Clear, area);

    let filtered = crate::k8s::filter_events(events, filter, search, sort.spec());

    const HEADERS: [&str; 6] = ["TYPE", "REASON", "OBJECT", "KIND", "MESSAGE", "AGE"];
    // MESSAGE is free text: it takes whatever room the other columns leave.
    let window = layout_table(
        &HEADERS,
        filtered.iter().map(|e| vec![cell_width("Warning"), cell_width(&e.reason), cell_width(&e.object), cell_width(&e.kind), cell_width(&e.message), cell_width(&e.age)]),
        area.width.saturating_sub(2),
        Some(4),
        &mut 0,
    );
    let header = header_row(&HEADERS, sort, dimmed, &window);
    let cell_style = theme_row(dimmed);
    let rows = filtered.iter().map(|e| {
        let color = if dimmed {
            Color::Rgb(40, 40, 40)
        } else {
            match (e.severity, e.kind.as_str()) {
                (crate::k8s::EventSeverity::Warning, "Node") => Color::Red,
                (crate::k8s::EventSeverity::Warning, _) => Color::Yellow,
                (crate::k8s::EventSeverity::Normal, _) => Color::Green,
            }
        };
        let type_text = match e.severity {
            crate::k8s::EventSeverity::Normal => "Normal",
            crate::k8s::EventSeverity::Warning => "Warning",
        };
        Row::new(window.slice(vec![
            Cell::from(type_text).style(Style::default().fg(color)),
            Cell::from(Line::from(highlight_matches(&e.reason, search, cell_style))),
            Cell::from(Line::from(highlight_matches(&e.object, search, cell_style))),
            Cell::from(Line::from(highlight_matches(&e.kind, search, cell_style))),
            Cell::from(Line::from(highlight_matches(&e.message, search, cell_style))),
            Cell::from(e.age.clone()).style(cell_style),
        ]))
    });

    // `Events (3/11)  (a) all  (w) warnings  (n) normal`, the active
    // severity highlighted, and `/text` while a search is applied.
    let active = if dimmed { dim_style() } else { Style::default().fg(Color::Rgb(240, 160, 110)).add_modifier(Modifier::BOLD) };
    let idle = if dimmed { dim_style() } else { Style::default().fg(Color::DarkGray) };
    let key_style = |this: EventFilter| if filter == this { active } else { idle };
    let count_style = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let mut title_spans = vec![
        Span::styled(format!("Events ({}/{})", filtered.len(), events.len()), count_style),
        Span::styled("  (a) all", key_style(EventFilter::All)),
        Span::styled("  (w) warnings", key_style(EventFilter::Warnings)),
        Span::styled("  (n) normal", key_style(EventFilter::Normal)),
    ];
    title_spans.extend(search_span(search, editing, dimmed));
    let title = Line::from(title_spans);

    let border_style = theme_border(dimmed);
    let table = Table::new(mark_rows(rows, &[], dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .highlight_symbol("")
        .row_highlight_style(selection_style(dimmed));

    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(filtered.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, state);
}

/// Which row of the Events browser's table (if any) sits under an
/// absolute terminal position — same `centered_rect(94, 88, ...)` and
/// border/header layout `draw_events_popup` actually renders with.
/// `offset` must be the table's own current scroll offset (`TableState::
/// offset()`, valid only after that state has actually been rendered
/// with once — same reasoning `row_at` already relies on for Pods).
pub fn event_row_at(frame_area: Rect, filtered_len: usize, offset: usize, row: u16) -> Option<usize> {
    let area = centered_rect(94, 88, frame_area);
    let top = area.y + 2; // top border + header row
    let bottom = area.y + area.height.saturating_sub(1); // bottom border
    if row < top || row >= bottom {
        return None;
    }
    let index = offset + usize::from(row - top);
    (index < filtered_len).then_some(index)
}

/// One event's full detail — a plain wrapped-text popup rather than a
/// table row, since the point is showing the *un*truncated message a
/// narrow MESSAGE column would otherwise clip.
pub(super) fn draw_event_detail_popup(frame: &mut Frame, entry: &EventEntry) {
    let area = centered_rect(70, 50, frame.area());
    frame.render_widget(Clear, area);

    let color = match (entry.severity, entry.kind.as_str()) {
        (crate::k8s::EventSeverity::Warning, "Node") => Color::Red,
        (crate::k8s::EventSeverity::Warning, _) => Color::Yellow,
        (crate::k8s::EventSeverity::Normal, _) => Color::Green,
    };
    let type_text = match entry.severity {
        crate::k8s::EventSeverity::Normal => "Normal",
        crate::k8s::EventSeverity::Warning => "Warning",
    };
    let label = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let lines = vec![
        Line::from(vec![Span::styled("Type:   ", label), Span::styled(type_text, Style::default().fg(color).add_modifier(Modifier::BOLD))]),
        Line::from(vec![Span::styled("Reason: ", label), Span::styled(entry.reason.clone(), bold)]),
        Line::from(vec![Span::styled("Object: ", label), Span::raw(format!("{} ({})", entry.object, entry.kind))]),
        Line::from(vec![Span::styled("Age:    ", label), Span::raw(entry.age.clone())]),
        Line::raw(""),
        Line::styled("Message:", bold),
        Line::raw(entry.message.clone()),
    ];

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title("Event detail");
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), area);
}

/// The Overview's Resources panel, opened up — the exact same
/// cluster-wide gauges (`draw_metrics_lines`) and per-node usage table
/// (`draw_nodes_table`) the compact panel and the Nodes list already
/// draw, just given a full-screen popup's worth of room instead of three
/// cramped lines.
/// One cluster-wide meter as a real gauge, not a hand-drawn bar — the
/// popup has room to spare, unlike the compact Overview panel's
/// three cramped lines (`draw_meter`), so there's no need for the
/// label-clipping tradeoffs that ruled `Gauge` out there. The used/
/// capacity/percentage text lives inside the gauge's own centered
/// label instead of needing a separate line for it.
pub(super) fn draw_gauge_box(frame: &mut Frame, area: Rect, label: &str, used: f64, capacity: f64, format_value: impl Fn(f64) -> String, dimmed: bool) {
    let ratio = if capacity > 0.0 { (used / capacity).clamp(0.0, 1.0) } else { 0.0 };
    let border_style = if dimmed { dim_style() } else { Style::default() };
    let title_style = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let gauge_style = if dimmed { dim_style() } else { Style::default().fg(usage_color(ratio, dimmed)) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Line::styled(format!(" {label} "), title_style));
    let label_text = format!("{} / {} ({:.0}%)", format_value(used), format_value(capacity), ratio * 100.0);
    let gauge = Gauge::default().block(block).gauge_style(gauge_style).ratio(ratio).label(label_text);
    frame.render_widget(gauge, area);
}

/// The Overview's Resources panel, opened up — cluster-wide CPU/Memory/
/// Pods only, as real gauges now there's room for them. Deliberately
/// doesn't repeat the per-node breakdown the Nodes list already owns —
/// that duplication was the actual complaint, not "the bars aren't
/// gauge-shaped enough."
pub(super) fn draw_resources_detail_popup(frame: &mut Frame, overview: &Overview, dimmed: bool) {
    let area = centered_rect(60, 30, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let outer = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title("Resources");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if !overview.metrics_available {
        let text = vec![
            Line::styled("metrics unavailable", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Line::styled("install metrics-server to see CPU/Memory usage", Style::default().fg(Color::DarkGray)),
        ];
        frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
        return;
    }

    let pod_usage = workloads_pod_count(overview);
    let chunks = Layout::vertical([Constraint::Length(3); 3]).split(inner);
    draw_gauge_box(
        frame,
        chunks[0],
        "CPU",
        overview.cpu_usage_millicores as f64,
        overview.cpu_capacity_millicores as f64,
        |v| format!("{:.2} cores", v / 1000.0),
        dimmed,
    );
    draw_gauge_box(
        frame,
        chunks[1],
        "Memory",
        overview.memory_usage_bytes as f64,
        overview.memory_capacity_bytes as f64,
        format_bytes,
        dimmed,
    );
    draw_gauge_box(frame, chunks[2], "Pods", pod_usage as f64, overview.pod_capacity as f64, |v| format!("{v:.0}"), dimmed);
}

pub(super) fn draw_containers_popup(frame: &mut Frame, title: &str, containers: &[ContainerInfo], state: &mut TableState, sort: SortState, dimmed: bool) {
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
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .title(if dimmed { Line::styled(title.to_string(), muted) } else { colored_slash_title(title) }),
        )
        .highlight_symbol("")
        .row_highlight_style(selection_style(dimmed));

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
