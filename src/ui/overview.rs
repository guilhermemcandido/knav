//! The Overview dashboard: top panel gauges, events feed and keyboard selection movement.

use super::*;

/// The Events panel is a fixed-size strip: it shows a capped number of entries and a
/// "+N more" line. The full feed is one Enter away (`Overlay::Events`).
pub(super) const MAX_VISIBLE_EVENTS: usize = 5;
/// The home screen: a fixed dashboard strip (Resources, then Events, each a rounded
/// box) above horizontally scrollable columns, one per category, listing its kinds.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_overview(
    frame: &mut Frame,
    area: Rect,
    overview: &Overview,
    selection: OverviewSelection,
    col_scroll: usize,
    item_scroll: usize,
    dimmed: bool,
    icons: &mut IconCache,
) {
    let top_h = top_area_height(overview);
    let chunks = Layout::vertical([Constraint::Length(top_h), Constraint::Length(1), Constraint::Min(0)]).split(area);
    let span = Rect { y: chunks[0].y, height: chunks[0].height, ..columns_span(area, overview.catalog.len()) };
    draw_top_panel(frame, span, overview, selection, dimmed);
    draw_columns(frame, chunks[2], overview, selection, col_scroll, item_scroll, dimmed, icons);
}

/// How tall the Resources box is: a rounded border top/bottom (2) plus
/// either 3 meter lines or the 2-line "unavailable" message.
pub(super) fn resources_box_height(overview: &Overview) -> u16 {
    2 + if overview.metrics_available { 3 } else { 2 }
}

/// How tall the Events box is: a rounded border top/bottom (2) plus
/// `events_content_height`.
pub(super) fn events_box_height(overview: &Overview) -> u16 {
    2 + events_content_height(overview)
}

/// How tall the top dashboard strip is: Resources box, a 1-row gap, Events box.
/// Hit-testing and the columns area use this so they match what is drawn.
pub(super) fn top_area_height(overview: &Overview) -> u16 {
    resources_box_height(overview) + 1 + events_box_height(overview)
}

/// Height of the Events box content: the 2-line empty message, or the header row
/// plus up to `MAX_VISIBLE_EVENTS` entries and a "+N more" line.
pub(super) fn events_content_height(overview: &Overview) -> u16 {
    if overview.events.is_empty() {
        return 2;
    }
    let shown = overview.events.len().min(MAX_VISIBLE_EVENTS);
    let more = usize::from(overview.events.len() > MAX_VISIBLE_EVENTS);
    1 + (shown + more) as u16
}

/// Resources and Events, each a rounded box like the columns. Selecting one
/// highlights its border; otherwise the Events border shows cluster health.
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

    // Plain styling like the Resources box: only the selection highlight colours the
    // border. Each event line keeps its own severity colour.
    let events_border = if dimmed {
        dim_style()
    } else if selection == OverviewSelection::Events {
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

/// Live pod count, read from the catalog tile that Pods' reflector already feeds.
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

/// A single-line usage meter: `CPU     ▓▓▓▓▓▓░░░░░░░░░░░░░░░░  71m / 2000m (3%)`.
/// Hand-built because `Gauge` centres a percentage label that clashes with the
/// numbers. The bar width adapts to the space available.
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

pub(super) fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1}{}", UNITS[unit])
}

/// Selection on the Overview: the Resources box, the Events box, a column header,
/// or an item in a column. Resources and Events sit above the columns: Up from a
/// header lands on Events, Down from Events returns to the first header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverviewSelection {
    Resources,
    Events,
    Header(usize),
    Item(usize, usize),
}

pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Moves the Overview selection one step. Up/Down move between a column's items,
/// its header and `Events`; Left/Right move between columns at the same item index
/// (or land on the header if that column is shorter). Resources and Events only go up/down.
pub fn move_overview_selection(overview: &Overview, selection: OverviewSelection, dir: Direction) -> OverviewSelection {
    let total = overview.catalog.len();
    match selection {
        OverviewSelection::Resources => match dir {
            Direction::Down => OverviewSelection::Events,
            _ => selection,
        },
        OverviewSelection::Events => match dir {
            Direction::Up => OverviewSelection::Resources,
            Direction::Down => {
                if total == 0 { selection } else { OverviewSelection::Header(0) }
            }
            _ => selection,
        },
        OverviewSelection::Header(col) => {
            if total == 0 {
                return selection;
            }
            let col = col.min(total - 1);
            match dir {
                Direction::Down => {
                    if column_len(overview, col) > 0 { OverviewSelection::Item(col, 0) } else { selection }
                }
                Direction::Up => OverviewSelection::Events,
                Direction::Left => if col > 0 { OverviewSelection::Header(col - 1) } else { selection },
                Direction::Right => if col + 1 < total { OverviewSelection::Header(col + 1) } else { selection },
            }
        }
        OverviewSelection::Item(col, item) => {
            if total == 0 {
                return selection;
            }
            let col = col.min(total - 1);
            let len = column_len(overview, col).max(1);
            let item = item.min(len - 1);
            match dir {
                Direction::Up => {
                    if item > 0 { OverviewSelection::Item(col, item - 1) } else { OverviewSelection::Header(col) }
                }
                Direction::Down => {
                    if item + 1 < len { OverviewSelection::Item(col, item + 1) } else { selection }
                }
                Direction::Left => {
                    if col == 0 {
                        selection
                    } else {
                        let target_len = column_len(overview, col - 1);
                        if target_len == 0 { OverviewSelection::Header(col - 1) } else { OverviewSelection::Item(col - 1, item.min(target_len - 1)) }
                    }
                }
                Direction::Right => {
                    if col + 1 >= total {
                        selection
                    } else {
                        let target_len = column_len(overview, col + 1);
                        if target_len == 0 { OverviewSelection::Header(col + 1) } else { OverviewSelection::Item(col + 1, item.min(target_len - 1)) }
                    }
                }
            }
        }
    }
}

/// Same movement rules as before, for the resource-switcher menu's own
/// section/tile grid, unrelated to the Overview's column browser, which
/// doesn't wrap tiles into rows at all anymore.
pub(super) fn next_nonempty_section(lens: &[usize], from: usize) -> Option<usize> {
    (from + 1..lens.len()).find(|&i| lens[i] > 0)
}

pub(super) fn prev_nonempty_section(lens: &[usize], from: usize) -> Option<usize> {
    (0..from).rev().find(|&i| lens[i] > 0)
}

pub(super) fn move_selection(section_lens: &[usize], cols: usize, current: (usize, usize), dir: Direction) -> (usize, usize) {
    let section_count = section_lens.len();
    if section_count == 0 {
        return current;
    }
    let section = current.0.min(section_count - 1);
    let len = section_lens[section].max(1);
    let tile = current.1.min(len - 1);
    let cols = cols.max(1);
    let row = tile / cols;
    let col = tile % cols;

    match dir {
        Direction::Left => {
            if col > 0 {
                (section, tile - 1)
            } else {
                match prev_nonempty_section(section_lens, section) {
                    Some(s) => (s, section_lens[s].saturating_sub(1)),
                    None => (section, tile),
                }
            }
        }
        Direction::Right => {
            if tile + 1 < len {
                (section, tile + 1)
            } else {
                match next_nonempty_section(section_lens, section) {
                    Some(s) => (s, 0),
                    None => (section, tile),
                }
            }
        }
        Direction::Up => {
            if row > 0 {
                (section, (row - 1) * cols + col)
            } else {
                match prev_nonempty_section(section_lens, section) {
                    Some(s) => {
                        let prev_len = section_lens[s];
                        let prev_rows = prev_len.div_ceil(cols).max(1);
                        let target = ((prev_rows - 1) * cols + col).min(prev_len.saturating_sub(1));
                        (s, target)
                    }
                    None => (section, tile),
                }
            }
        }
        Direction::Down => {
            let next = (row + 1) * cols + col;
            if next < len {
                (section, next)
            } else {
                match next_nonempty_section(section_lens, section) {
                    Some(s) => (s, col.min(section_lens[s].saturating_sub(1))),
                    None => (section, tile),
                }
            }
        }
    }
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

/// One dashboard line coloured by severity: a not-ready node is red, a Warning
/// yellow, a Normal event muted green.
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

pub(super) fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if cell_width(s) <= max {
        return s.to_string();
    }
    // Leave a cell for the ellipsis, cutting on whole characters.
    let (mut out, mut used) = (String::new(), 0);
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
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
    fn up_from_any_column_header_goes_to_events() {
        let overview = test_overview(vec![("A", vec![("a1", 0)]), ("B", vec![("b1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Header(0), Direction::Up), OverviewSelection::Events);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Header(1), Direction::Up), OverviewSelection::Events);
    }

    #[test]
    fn events_and_resources_navigate_vertically_into_each_other_and_the_columns() {
        let overview = test_overview(vec![("A", vec![("a1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Resources, Direction::Down), OverviewSelection::Events);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events, Direction::Up), OverviewSelection::Resources);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events, Direction::Down), OverviewSelection::Header(0));
    }

    #[test]
    fn resources_and_events_ignore_left_right_and_resources_ignores_up() {
        let overview = test_overview(vec![("A", vec![("a1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Resources, Direction::Up), OverviewSelection::Resources);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Resources, Direction::Left), OverviewSelection::Resources);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events, Direction::Right), OverviewSelection::Events);
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
        // +1 for the gap `columns_area` now puts between the top strip
        // and the columns, matching the Resources-to-Events gap.
        let top_h = top_area_height(&overview) + 1;
        // The columns are centred, so ask the layout where the first one is.
        let x0 = column_layout(columns_inner(columns_area(frame_area, &overview)), 1)[0].x + 1;
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, 1, 0), Some(OverviewSelection::Resources));
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, 1, resources_box_height(&overview) + 1), Some(OverviewSelection::Events));
        // Row 0 of the columns area is the column box's top border (the
        // header); rows 1-3 are the first item card (border/content/border).
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, x0, top_h), Some(OverviewSelection::Header(0)));
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, x0, top_h + 1), Some(OverviewSelection::Item(0, 0)));
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, x0, top_h + 4), Some(OverviewSelection::Item(0, 1)));
    }

    #[test]
    fn column_hit_uses_item_scroll_only_for_the_active_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0), ("a3", 0)]), ("B", vec![("b1", 0), ("b2", 0)])]);
        let frame_area = Rect { x: 0, y: 0, width: 80, height: 40 };
        let top_h = top_area_height(&overview) + 1;
        let layout = column_layout(columns_inner(columns_area(frame_area, &overview)), 2);
        let x0 = layout[0].x + 1;
        // Column 0 is active with item_scroll 1: its first visible card is
        // actually item index 1, not 0.
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 1, x0, top_h + 1), Some(OverviewSelection::Item(0, 1)));
        // Column 1 isn't active, so it renders from item 0. The columns area has a 1-cell
        // left scroll-arrow gutter.
        let col1_x = layout[1].x + 1;
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 1, col1_x, top_h + 1), Some(OverviewSelection::Item(1, 0)));
    }

    #[test]
    fn visible_columns_accounts_for_the_inter_column_gap() {
        // Two columns need 2*COLUMN_WIDTH + 1 cells (one gap between them),
        // not 2*COLUMN_WIDTH.
        let two_cols_width = COLUMN_WIDTH * 2 + 1;
        assert_eq!(visible_columns(two_cols_width, 5), 2);
        assert_eq!(visible_columns(two_cols_width - 1, 5), 1);
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    #[test]
    fn truncate_counts_terminal_cells_not_characters() {
        assert_eq!(cell_width("日本語"), 6);
        let cut = truncate("日本語日本語", 7);
        assert!(cell_width(&cut) <= 7 && cut.ends_with('…'), "{cut}");
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 0), "");
    }
}
