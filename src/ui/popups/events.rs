//! The events browser and one event.

use super::*;

/// The full Events browser: every event, uncapped, filterable by severity
/// (`Normal` and `Warning` are all Kubernetes defines).
pub(in crate::ui) fn draw_events_popup(
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
            theme().dim
        } else {
            match (e.severity, e.kind.as_str()) {
                (crate::k8s::EventSeverity::Warning, "Node") => theme().bad,
                (crate::k8s::EventSeverity::Warning, _) => theme().warn,
                (crate::k8s::EventSeverity::Normal, _) => theme().ok,
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
    let active = if dimmed { dim_style() } else { Style::default().fg(theme().warm).add_modifier(Modifier::BOLD) };
    let idle = if dimmed { dim_style() } else { Style::default().fg(theme().muted) };
    let key_style = |this: EventFilter| if filter == this { active } else { idle };
    let mut title_spans = pill_title(&format!("Events ({}/{})", filtered.len(), events.len()), dimmed, theme_border(dimmed)).spans;
    title_spans.extend([
        Span::styled("  (a) all", key_style(EventFilter::All)),
        Span::styled("  (w) warnings", key_style(EventFilter::Warnings)),
        Span::styled("  (n) normal", key_style(EventFilter::Normal)),
    ]);
    let title = Line::from(title_spans);

    let border_style = theme_border(dimmed);
    let table = Table::new(mark_rows(rows, &[], dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(with_search(Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title(title), search, editing, dimmed))
        .highlight_symbol("")
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, dimmed));

    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(filtered.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, state);
}

/// Which row of the Events table is under a terminal position, using the layout
/// of `draw_events_popup`. `offset` is the table's scroll offset, valid only after
/// that state has been rendered once.
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

/// One event's full detail, a plain wrapped-text popup rather than a
/// table row, since the point is showing the *un*truncated message a
/// narrow MESSAGE column would otherwise clip.
pub(in crate::ui) fn draw_event_detail_popup(frame: &mut Frame, entry: &EventEntry) {
    let area = centered_rect(70, 50, frame.area());
    frame.render_widget(Clear, area);

    let color = match (entry.severity, entry.kind.as_str()) {
        (crate::k8s::EventSeverity::Warning, "Node") => theme().bad,
        (crate::k8s::EventSeverity::Warning, _) => theme().warn,
        (crate::k8s::EventSeverity::Normal, _) => theme().ok,
    };
    let type_text = match entry.severity {
        crate::k8s::EventSeverity::Normal => "Normal",
        crate::k8s::EventSeverity::Warning => "Warning",
    };
    let label = Style::default().fg(theme().muted);
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

    let block = Block::default().borders(Borders::ALL).border_set(border_set()).title(pill_title("Event detail", false, Style::default()));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), area);
}
