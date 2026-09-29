//! The Overview: the Resources and Events boxes, and moving the selection around it.

use super::*;

/// The Events box shows this many entries and a "+N more" line; Enter opens them all.
pub(super) const MAX_VISIBLE_EVENTS: usize = 5;
/// Where the Overview's selection and scroll are, and whether it is dimmed.
#[derive(Clone, Copy)]
pub(super) struct OverviewView {
    pub selection: OverviewSelection,
    /// The column scroll, and the item scroll within the selected column.
    pub col_scroll: usize,
    pub item_scroll: usize,
    pub dimmed: bool,
}

/// The home screen: Resources and Events on top, then scrollable category columns.
pub(super) fn draw_overview(frame: &mut Frame, area: Rect, overview: &Overview, view: OverviewView, icons: &mut IconCache) {
    let OverviewView { selection, dimmed, .. } = view;
    let top_h = top_area_height(overview);
    let chunks = Layout::vertical([Constraint::Length(top_h), Constraint::Length(1), Constraint::Min(0)]).split(area);
    let span = Rect { y: chunks[0].y, height: chunks[0].height, ..columns_span(area, overview.catalog.len()) };
    draw_top_panel(frame, span, overview, selection, dimmed);
    draw_columns(frame, chunks[2], overview, view, icons);
}

/// The Resources box height: borders plus three meters, or the two-line unavailable message.
pub(super) fn resources_box_height(overview: &Overview) -> u16 {
    2 + if overview.metrics_available { 3 } else { 2 }
}

pub(super) fn events_box_height(overview: &Overview) -> u16 {
    2 + events_content_height(overview)
}

/// The top strip's height: Resources, a blank row, Events. Hit-testing uses it too.
pub(super) fn top_area_height(overview: &Overview) -> u16 {
    resources_box_height(overview) + 1 + events_box_height(overview)
}

/// The Events box content height: the empty message, or a header, the entries and "+N more".
pub(super) fn events_content_height(overview: &Overview) -> u16 {
    if overview.events.is_empty() {
        return 2;
    }
    let shown = overview.events.len().min(MAX_VISIBLE_EVENTS);
    let more = usize::from(overview.events.len() > MAX_VISIBLE_EVENTS);
    1 + (shown + more) as u16
}

/// The Resources and Events boxes. A selected one has its border highlighted.
pub(super) fn draw_top_panel(frame: &mut Frame, area: Rect, overview: &Overview, selection: OverviewSelection, dimmed: bool) {
    let resources_h = resources_box_height(overview);
    let chunks = Layout::vertical([Constraint::Length(resources_h), Constraint::Length(1), Constraint::Min(0)]).split(area);

    let highlight = Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD);
    let resources_border = if dimmed {
        dim_style()
    } else if selection == OverviewSelection::Resources {
        highlight
    } else {
        Style::default()
    };
    let resources_block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(resources_border)
        .title(Line::styled(" Resources ", if dimmed { resources_border } else { Style::default().add_modifier(Modifier::BOLD) }));
    let resources_inner = resources_block.inner(chunks[0]);
    frame.render_widget(resources_block, chunks[0]);
    draw_metrics_lines(frame, resources_inner, overview, dimmed);

    // Only selection colours the border; each event line keeps its severity colour.
    let events_border = if dimmed {
        dim_style()
    } else if matches!(selection, OverviewSelection::Events(_)) {
        highlight
    } else {
        Style::default()
    };
    let events_block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(events_border)
        .title(Line::styled(format!(" Events ({}) ", overview.events.len()), if dimmed { events_border } else { Style::default().add_modifier(Modifier::BOLD) }));
    let events_inner = events_block.inner(chunks[2]);
    frame.render_widget(events_block, chunks[2]);

    if overview.events.is_empty() {
        draw_events_empty(frame, events_inner, dimmed);
    } else {
        let shown = overview.events.len().min(MAX_VISIBLE_EVENTS);
        let has_more = overview.events.len() > MAX_VISIBLE_EVENTS;
        let lines = Layout::vertical((0..1 + shown + usize::from(has_more)).map(|_| Constraint::Length(1))).split(events_inner);
        draw_events_header(frame, lines[0], dimmed);
        for (i, e) in overview.events.iter().take(shown).enumerate() {
            draw_event_line(frame, lines[i + 1], e, dimmed);
        }
        if has_more {
            let more = overview.events.len() - shown;
            let style = if dimmed { dim_style() } else { Style::default().fg(theme().muted).add_modifier(Modifier::ITALIC) };
            frame.render_widget(Paragraph::new(Line::styled(format!("… and {more} more"), style)).alignment(Alignment::Center), lines[1 + shown]);
        }
    }
}

/// The live pod count, from the Pods tile.
pub(super) fn workloads_pod_count(overview: &Overview) -> usize {
    overview
        .catalog
        .iter()
        .find(|(section, _)| *section == "Workloads")
        .and_then(|(_, tiles)| tiles.iter().find(|(label, _)| *label == "Pods"))
        .map(|(_, count)| *count)
        .unwrap_or(0)
}

pub(super) fn draw_metrics_lines(frame: &mut Frame, area: Rect, overview: &Overview, dimmed: bool) {
    if !overview.metrics_available {
        let text = vec![
            Line::styled("metrics unavailable", Style::default().fg(theme().muted).add_modifier(Modifier::BOLD)),
            Line::styled("install metrics-server to see CPU/Memory usage", Style::default().fg(theme().muted)),
        ];
        frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), area);
        return;
    }

    let pod_usage = workloads_pod_count(overview);

    let lines = Layout::vertical([Constraint::Length(1); 3]).split(area);
    draw_meter(
        frame,
        lines[0],
        "CPU",
        overview.cpu_usage_millicores as f64,
        overview.cpu_capacity_millicores as f64,
        |v| format!("{:.2} cores", v / 1000.0),
        dimmed,
    );
    draw_meter(
        frame,
        lines[1],
        "Memory",
        overview.memory_usage_bytes as f64,
        overview.memory_capacity_bytes as f64,
        format_bytes,
        dimmed,
    );
    draw_meter(frame, lines[2], "Pods", pod_usage as f64, overview.pod_capacity as f64, |v| format!("{v:.0}"), dimmed);
}

/// A one-line usage meter like `CPU  ▓▓▓▓░░░░  71m / 2000m (3%)`, built by hand since
/// `Gauge` centres a label that clashes with the numbers.
pub(super) fn draw_meter(frame: &mut Frame, area: Rect, label: &str, used: f64, capacity: f64, format_value: impl Fn(f64) -> String, dimmed: bool) {
    let ratio = if capacity > 0.0 { (used / capacity).clamp(0.0, 1.0) } else { 0.0 };
    let color = usage_color(ratio, dimmed);
    let detail = format!("{} / {} ({:.0}%)", format_value(used), format_value(capacity), ratio * 100.0);

    let label_text = format!("{label:<8}");
    let reserved = cell_width(&label_text) as u16 + cell_width(&detail) as u16 + 5;
    let bar_width = area.width.saturating_sub(reserved).max(4) as usize;
    let filled = ((ratio * bar_width as f64).round() as usize).min(bar_width);

    let label_style = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let detail_style = if dimmed { dim_style() } else { Style::default() };
    let bracket = if dimmed { dim_style() } else { Style::default().fg(theme().muted) };

    let line = Line::from(vec![
        Span::styled(label_text, label_style),
        Span::styled("[", bracket),
        Span::styled("▓".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(bar_width - filled), Style::default().fg(theme().muted)),
        Span::styled("]", bracket),
        Span::styled(format!(" {detail}"), detail_style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

pub(super) fn draw_events_header(frame: &mut Frame, area: Rect, dimmed: bool) {
    let style = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let line = format!("{:<8}{:<44} {:<18} {:<12} AGE", "TYPE", "MESSAGE", "OBJECT", "KIND");
    frame.render_widget(Paragraph::new(Line::styled(line, style)), area);
}

pub(super) fn draw_events_empty(frame: &mut Frame, area: Rect, dimmed: bool) {
    let ok_style = if dimmed { dim_style() } else { Style::default().fg(theme().ok).add_modifier(Modifier::BOLD) };
    let text = vec![Line::styled("✓ No events", ok_style)];
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), area);
}

/// One event line coloured by severity: a node warning red, other warnings yellow,
/// normal events green.
pub(super) fn draw_event_line(frame: &mut Frame, area: Rect, entry: &EventEntry, dimmed: bool) {
    let color = if dimmed {
        theme().dim
    } else {
        match (entry.severity, entry.kind.as_str()) {
            (crate::k8s::EventSeverity::Warning, "Node") => theme().bad,
            (crate::k8s::EventSeverity::Warning, _) => theme().warn,
            (crate::k8s::EventSeverity::Normal, _) => theme().ok,
        }
    };
    let type_text = match entry.severity {
        crate::k8s::EventSeverity::Normal => "Normal",
        crate::k8s::EventSeverity::Warning => "Warning",
    };
    let message = truncate(&entry.message, 42);
    let line = format!("{type_text:<8}{message:<44} {:<18} {:<12} {}", entry.object, entry.kind, entry.age);
    frame.render_widget(Paragraph::new(Line::styled(line, Style::default().fg(color))), area);
}

#[cfg(test)]
mod overview_selection_tests {
    use super::*;

    fn test_overview(catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>) -> Overview {
        Overview {
            events: vec![],
            cpu_usage_millicores: 0,
            cpu_capacity_millicores: 0,
            memory_usage_bytes: 0,
            memory_capacity_bytes: 0,
            pod_capacity: 0,
            metrics_available: false,
            catalog,
            health: Default::default(),
            report: None,
        }
    }

    #[test]
    fn down_moves_to_next_item_within_a_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Item(0, 0), Direction::Down),
            OverviewSelection::Item(0, 1)
        );
    }

    #[test]
    fn down_stops_at_the_last_item_of_a_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        let last = OverviewSelection::Item(0, 1);
        assert_eq!(move_overview_selection(&overview, last, Direction::Down), last);
    }

    #[test]
    fn up_at_the_first_item_goes_to_the_columns_own_header() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Item(0, 0), Direction::Up),
            OverviewSelection::Header(0)
        );
    }

    #[test]
    fn down_from_a_header_enters_its_first_item() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Header(0), Direction::Down),
            OverviewSelection::Item(0, 0)
        );
    }

    #[test]
    fn up_from_any_column_header_goes_to_events_remembering_that_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0)]), ("B", vec![("b1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Header(0), Direction::Up), OverviewSelection::Events(0));
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Header(1), Direction::Up), OverviewSelection::Events(1));
    }

    #[test]
    fn down_from_events_returns_to_the_column_it_was_left_from_not_always_the_first() {
        let overview = test_overview(vec![("A", vec![("a1", 0)]), ("B", vec![("b1", 0)]), ("C", vec![("c1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events(2), Direction::Down), OverviewSelection::Header(2));
        // Round-tripping through Events (Up then Down) is a no-op on the column.
        let up = move_overview_selection(&overview, OverviewSelection::Header(1), Direction::Up);
        assert_eq!(move_overview_selection(&overview, up, Direction::Down), OverviewSelection::Header(1));
    }

    #[test]
    fn events_and_resources_navigate_vertically_into_each_other_and_the_columns() {
        let overview = test_overview(vec![("A", vec![("a1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Resources, Direction::Down), OverviewSelection::Events(0));
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events(0), Direction::Up), OverviewSelection::Resources);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events(0), Direction::Down), OverviewSelection::Header(0));
    }

    #[test]
    fn resources_and_events_ignore_left_right_and_resources_ignores_up() {
        let overview = test_overview(vec![("A", vec![("a1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Resources, Direction::Up), OverviewSelection::Resources);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Resources, Direction::Left), OverviewSelection::Resources);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events(0), Direction::Right), OverviewSelection::Events(0));
    }

    #[test]
    fn left_right_move_headers_directly_between_columns() {
        let overview = test_overview(vec![("A", vec![("a1", 0)]), ("B", vec![("b1", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Header(0), Direction::Right),
            OverviewSelection::Header(1)
        );
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Header(1), Direction::Left),
            OverviewSelection::Header(0)
        );
    }

    #[test]
    fn left_right_move_items_at_the_same_index_between_columns() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)]), ("B", vec![("b1", 0), ("b2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Item(0, 1), Direction::Right),
            OverviewSelection::Item(1, 1)
        );
    }

    #[test]
    fn movement_clamps_at_the_first_and_last_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0)])]);
        let only = OverviewSelection::Header(0);
        assert_eq!(move_overview_selection(&overview, only, Direction::Left), only);
        assert_eq!(move_overview_selection(&overview, only, Direction::Right), only);
    }

    #[test]
    fn scroll_columns_to_show_brings_a_column_off_either_edge_into_view() {
        assert_eq!(scroll_columns_to_show(0, 3, 5), 3); // off the right edge
        assert_eq!(scroll_columns_to_show(3, 3, 1), 1); // off the left edge
        assert_eq!(scroll_columns_to_show(2, 3, 3), 2); // already visible
    }

    #[test]
    fn column_hit_resolves_resources_events_header_and_item_rows() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        let frame_area = Rect { x: 0, y: 0, width: 80, height: 40 };
        // +1 for the blank row between the top strip and the columns.
        let top_h = top_area_height(&overview) + 1;
        // The columns are centred, so ask the layout where the first one is.
        let x0 = column_layout(columns_inner(columns_area(frame_area, &overview)), 1)[0].x + 1;
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, 1, 0), Some(OverviewSelection::Resources));
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, 1, resources_box_height(&overview) + 1), Some(OverviewSelection::Events(0)));
        // Row 0 is the column's top border (its header), row 1 the empty ▲ lane,
        // rows 2 to 5 the first card.
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, x0, top_h), Some(OverviewSelection::Header(0)));
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, x0, top_h + 1), None);
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, x0, top_h + 2), Some(OverviewSelection::Item(0, 0)));
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, x0, top_h + 6), Some(OverviewSelection::Item(0, 1)));
    }

    #[test]
    fn column_hit_uses_item_scroll_only_for_the_active_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0), ("a3", 0)]), ("B", vec![("b1", 0), ("b2", 0)])]);
        let frame_area = Rect { x: 0, y: 0, width: 80, height: 40 };
        let top_h = top_area_height(&overview) + 1;
        let layout = column_layout(columns_inner(columns_area(frame_area, &overview)), 2);
        let x0 = layout[0].x + 1;
        // Column 0 is active with item_scroll 1, so its first visible card is item 1.
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 1, x0, top_h + 2), Some(OverviewSelection::Item(0, 1)));
        // Column 1 isn't active, so it starts at item 0. The area has a 1-cell arrow gutter.
        let col1_x = layout[1].x + 1;
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 1, col1_x, top_h + 2), Some(OverviewSelection::Item(1, 0)));
    }

    #[test]
    fn visible_columns_accounts_for_the_inter_column_gap() {
        // Two columns need one gap between them.
        let two_cols_width = COLUMN_WIDTH * 2 + 1;
        assert_eq!(visible_columns(two_cols_width, 5), 2);
        assert_eq!(visible_columns(two_cols_width - 1, 5), 1);
    }
}
