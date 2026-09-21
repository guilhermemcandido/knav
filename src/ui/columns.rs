//! The Overview's catalog columns and the opened-up column popup, plus hit-testing/scrolling for them.

use super::*;

/// Column width for the resource-switcher menu's own tile grid (see
/// `menu_cols`/`draw_menu_popup`) — the Overview page no longer uses
/// fixed-size tiles at all, but the menu still does.
pub(super) const TILE_WIDTH: u16 = 22;
/// One catalog column's fixed width in the Overview browser, including
/// its own rounded border. A 1-cell gap is inserted between columns (see
/// `column_layout`) so each reads as a distinct bordered pane, herdr-style,
/// rather than boxes sharing an edge.
pub(super) const COLUMN_WIDTH: u16 = 28;
/// An item card's height: a rounded-border top edge, one content row (icon
/// on the left, name + live count filling the rest), and a rounded-border
/// bottom edge.
pub(super) const ITEM_HEIGHT: u16 = 3;
/// The taller card, with a second content row, used by a column that has
/// a name too long for one row (`ClusterRoleBindings` -> `ClusterRole` /
/// ` Bindings`). Every card in that column takes this height so they stay
/// aligned.
pub(super) const ITEM_HEIGHT_WRAPPED: u16 = 4;

/// Room for a card's label on one row at a given column width: the column
/// and card borders (2 + 2), the icon (3) and the count with its space.
fn label_room(column_width: u16, count: usize) -> usize {
    (usize::from(column_width)).saturating_sub(7 + count.to_string().len() + 1).max(1)
}

/// Splits a name over two rows at a capital letter, as evenly as it can:
/// `ClusterRoleBindings` -> (`ClusterRole`, ` Bindings`). Falls back to a
/// `.` boundary (API groups) and then to a hard split; the second row is
/// truncated if it still doesn't fit. `None` when it fits on one row.
pub(super) fn wrap_label(label: &str, room: usize) -> Option<(String, String)> {
    let chars: Vec<char> = label.chars().collect();
    if chars.len() <= room {
        return None;
    }
    let boundaries = |pred: &dyn Fn(usize) -> bool| -> Vec<usize> { (1..chars.len()).filter(|&i| pred(i)).collect() };
    let mut candidates = boundaries(&|i| chars[i].is_uppercase() && !chars[i - 1].is_uppercase());
    if candidates.is_empty() {
        candidates = boundaries(&|i| chars[i - 1] == '.');
    }
    if candidates.is_empty() {
        candidates = vec![room.min(chars.len() - 1)];
    }
    // The most even split whose first row fits (the second row gets a
    // leading space to read as a continuation).
    let best = candidates
        .into_iter()
        .filter(|&i| i <= room)
        .min_by_key(|&i| i.max(chars.len() - i + 1))
        .unwrap_or(room.min(chars.len() - 1));
    let first: String = chars[..best].iter().collect();
    let rest: String = chars[best..].iter().collect();
    let second = truncate(&format!(" {rest}"), room);
    Some((first, second))
}

/// The height every card in a column takes: taller when any of its names
/// needs two rows at `column_width`.
pub fn item_height(items: &[(&str, usize)], column_width: u16) -> u16 {
    if items.iter().any(|(label, count)| wrap_label(label, label_room(column_width, *count)).is_some()) {
        ITEM_HEIGHT_WRAPPED
    } else {
        ITEM_HEIGHT
    }
}

/// `item_height` for one Overview column.
pub fn column_item_height(overview: &Overview, col: usize) -> u16 {
    overview.catalog.get(col).map(|(_, items)| item_height(items, COLUMN_WIDTH)).unwrap_or(ITEM_HEIGHT)
}
/// Width of the left/right scroll-affordance gutters flanking the
/// columns area (see `columns_inner`) — just wide enough for a single
/// arrow glyph.
pub(super) const SCROLL_ARROW_WIDTH: u16 = 1;

/// The columns area is whatever's left below the fixed dashboard strip
/// — callers (keyboard navigation, mouse hit-testing) need this same
/// rectangle to stay in sync with what's actually rendered.
pub fn columns_area(frame_area: Rect, overview: &Overview) -> Rect {
    // +1 for the same gap `draw_overview` puts between the top strip and
    // the columns — Resources-to-Events and top-strip-to-columns are now
    // both a single blank row, not one bigger than the other.
    let top_h = top_area_height(overview) + 1;
    Rect { x: frame_area.x, y: frame_area.y + top_h, width: frame_area.width, height: frame_area.height.saturating_sub(top_h) }
}

/// The columns area minus its left/right scroll-arrow gutters — every
/// place that lays out or hit-tests the column boxes themselves
/// (`draw_columns`, `column_hit`) works within this narrower rect so the
/// arrows always sit outside the boxes rather than overlapping them.
pub(super) fn columns_inner(area: Rect) -> Rect {
    let shrink = SCROLL_ARROW_WIDTH * 2;
    Rect { x: area.x + SCROLL_ARROW_WIDTH, y: area.y, width: area.width.saturating_sub(shrink), height: area.height }
}

pub fn visible_columns(width: u16, total_columns: usize) -> usize {
    // Each column takes `COLUMN_WIDTH` plus a 1-cell gap before the next
    // one (see `column_layout`) — so `n` columns actually need
    // `n * (COLUMN_WIDTH + 1) - 1` cells, not `n * COLUMN_WIDTH`.
    let cols = ((width + 1) / (COLUMN_WIDTH + 1)).max(1) as usize;
    cols.min(total_columns.max(1))
}

/// How many item cards fit vertically inside one column, given the whole
/// columns area's height — every column shares that same height
/// regardless of how many items it actually holds, so this one number is
/// right for all of them. Used both to size the keyboard auto-scroll
/// window and (implicitly, via the same math in `draw_column`) to decide
/// how many cards actually get drawn.
pub fn visible_items_per_column(columns_area_height: u16, item_height: u16) -> usize {
    (columns_area_height.saturating_sub(2) / item_height).max(1) as usize
}

/// The shared column-rect layout — `draw_columns` and `column_hit` must
/// agree on exactly where each column's box sits, or clicks stop lining
/// up with what's on screen.
pub(super) fn column_layout(area: Rect, cols_visible: usize) -> std::rc::Rc<[Rect]> {
    // The block of columns sits in the middle of the space, not against its left edge.
    let used = (cols_visible as u16 * (COLUMN_WIDTH + 1)).saturating_sub(1).min(area.width);
    let area = Rect { x: area.x + (area.width - used) / 2, width: used, ..area };
    let constraints: Vec<Constraint> = (0..cols_visible).map(|_| Constraint::Length(COLUMN_WIDTH)).collect();
    Layout::horizontal(constraints).spacing(1).split(area)
}

pub(super) fn column_len(overview: &Overview, col: usize) -> usize {
    overview.catalog.get(col).map(|(_, items)| items.len()).unwrap_or(0)
}

/// The area a column-detail popup (see `Overlay::ColumnDetail`) actually
/// renders into — one place so its own draw pass, the grid column count,
/// and the visible-row count can't drift apart.
pub(super) fn column_detail_area(frame_area: Rect) -> Rect {
    centered_rect(85, 80, frame_area)
}

/// How many item cards fit per row in a column-detail popup — same
/// card width the compact Overview columns use, so a kind's card looks
/// identical whether you're looking at it there or here.
pub fn column_detail_cols(frame_area: Rect) -> usize {
    let inner = Block::default().borders(Borders::ALL).inner(column_detail_area(frame_area));
    ((inner.width + 1) / (COLUMN_WIDTH + 1)).max(1) as usize
}

/// How many grid rows of item cards fit vertically in a column-detail
/// popup at once.
pub fn column_detail_visible_rows(frame_area: Rect, items: &[(&str, usize)]) -> usize {
    let inner = Block::default().borders(Borders::ALL).inner(column_detail_area(frame_area));
    (inner.height / item_height(items, COLUMN_WIDTH)).max(1) as usize
}

/// Same movement rules as `move_selection`/`move_menu_selection`, for a
/// column-detail popup's own single-list item grid — reuses `move_selection`
/// with exactly one "section" (there's nothing to jump to when you run off
/// an edge, so it just clamps there, which is exactly what a single list
/// needs).
pub fn move_column_detail_selection(items_len: usize, cols: usize, selected: usize, dir: Direction) -> usize {
    move_selection(&[items_len], cols, (0, selected), dir).1
}

/// Adjusts a scroll offset (if needed) so `target` is fully within the
/// `visible` window currently on screen — scrolls back immediately if the
/// selection moved before the window, or forward just far enough if it
/// moved past it. Dimension-agnostic: used both for the Overview's
/// horizontal column scroll and its vertical within-column item scroll.
pub fn scroll_columns_to_show(col_scroll: usize, cols_visible: usize, target_col: usize) -> usize {
    if target_col < col_scroll {
        target_col
    } else if target_col >= col_scroll + cols_visible {
        target_col + 1 - cols_visible
    } else {
        col_scroll
    }
}

/// Which column header or item (if any) sits under an absolute terminal
/// position, or the Resources/Events box above them — same layout
/// `draw_top_panel`/`draw_columns` actually render with, so a click
/// always resolves to what's really on screen. `active_col`/`item_scroll`
/// must be whatever was actually passed to the last `draw_columns` call —
/// only the active column's items are vertically scrolled, everything
/// else always renders starting from its own first item.
pub fn column_hit(
    frame_area: Rect,
    overview: &Overview,
    col_scroll: usize,
    active_col: usize,
    item_scroll: usize,
    column: u16,
    row: u16,
) -> Option<OverviewSelection> {
    if row < frame_area.y {
        return None;
    }
    let resources_h = resources_box_height(overview);
    let rel = row - frame_area.y;
    if rel < resources_h {
        return Some(OverviewSelection::Resources);
    }
    let events_start = resources_h + 1;
    if rel >= events_start && rel < events_start + events_box_height(overview) {
        return Some(OverviewSelection::Events);
    }

    let area = columns_inner(columns_area(frame_area, overview));
    if row < area.y || row >= area.y + area.height || column < area.x || column >= area.x + area.width {
        return None;
    }
    let total = overview.catalog.len();
    if total == 0 {
        return None;
    }
    let cols_visible = visible_columns(area.width, total);
    let col_scroll = col_scroll.min(total - cols_visible);
    let areas = column_layout(area, cols_visible);
    let col_i = areas.iter().position(|r| column >= r.x && column < r.x + r.width)?;
    let col_area = areas[col_i];
    let col_idx = col_scroll + col_i;

    if row == col_area.y {
        return Some(OverviewSelection::Header(col_idx));
    }
    let inner = Block::default().borders(Borders::ALL).inner(col_area);
    if row < inner.y || row >= inner.y + inner.height {
        return None;
    }
    let scroll = if col_idx == active_col { item_scroll } else { 0 };
    let (_, items) = &overview.catalog[col_idx];
    let item_i = ((row - inner.y) / item_height(items, COLUMN_WIDTH)) as usize + scroll;
    if item_i < items.len() { Some(OverviewSelection::Item(col_idx, item_i)) } else { None }
}

/// Draws the columns themselves plus, in the 1-cell gutters flanking
/// them, a "◀"/"▶" arrow whenever scrolling that way would actually
/// reveal another column — the replacement for the old per-column
/// collapse toggle as the way to signal "there's more here."
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_columns(frame: &mut Frame, area: Rect, overview: &Overview, selection: OverviewSelection, col_scroll: usize, item_scroll: usize, dimmed: bool, icons: &mut IconCache) {
    let total = overview.catalog.len();
    if total == 0 {
        return;
    }
    let inner = columns_inner(area);
    let cols_visible = visible_columns(inner.width, total);
    let col_scroll = col_scroll.min(total - cols_visible);
    let areas = column_layout(inner, cols_visible);
    let active_col = match selection {
        OverviewSelection::Header(c) | OverviewSelection::Item(c, _) => c,
        OverviewSelection::Resources | OverviewSelection::Events => usize::MAX,
    };
    for (i, col_area) in areas.iter().enumerate() {
        let col_idx = col_scroll + i;
        let (title, items) = &overview.catalog[col_idx];
        let scroll = if col_idx == active_col { item_scroll } else { 0 };
        draw_column(frame, *col_area, col_idx, title, items, selection, scroll, dimmed, icons);
    }

    let arrow_style = if dimmed { dim_style() } else { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) };
    if col_scroll > 0 {
        let left = Rect { x: area.x, y: area.y, width: SCROLL_ARROW_WIDTH, height: 1 };
        frame.render_widget(Paragraph::new(Span::styled("◀", arrow_style)), left);
    }
    if col_scroll + cols_visible < total {
        let right = Rect { x: area.x + area.width - SCROLL_ARROW_WIDTH, y: area.y, width: SCROLL_ARROW_WIDTH, height: 1 };
        frame.render_widget(Paragraph::new(Span::styled("▶", arrow_style)), right);
    }
}

/// One column: a rounded-border box — herdr-style, the whole box's border
/// takes on the highlight color when its header is selected — carrying
/// the category name as its title, with that category's kinds listed
/// vertically inside as their own item cards (see `draw_column_item`).
/// `item_scroll` is only meaningful for whichever column is actually the
/// current selection's — every other column always renders from its own
/// first item.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_column(
    frame: &mut Frame,
    area: Rect,
    col_idx: usize,
    title: &str,
    items: &[(&str, usize)],
    selection: OverviewSelection,
    item_scroll: usize,
    dimmed: bool,
    icons: &mut IconCache,
) {
    let header_selected = matches!(selection, OverviewSelection::Header(c) if c == col_idx);
    let highlight = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);

    let (border_style, title_style) = if dimmed {
        (dim_style(), dim_style())
    } else if header_selected {
        (highlight, highlight)
    } else {
        (Style::default(), Style::default().add_modifier(Modifier::BOLD))
    };

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BORDER)
        .border_style(border_style)
        .title(Line::styled(format!(" {title} "), title_style));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let item_h = item_height(items, COLUMN_WIDTH);
    if items.is_empty() || inner.height < item_h {
        return;
    }

    let visible = visible_items_per_column(area.height, item_h);
    let scroll = item_scroll.min(items.len().saturating_sub(visible));
    let shown: Vec<(usize, &(&str, usize))> = items.iter().enumerate().skip(scroll).take(visible).collect();

    let constraints: Vec<Constraint> = shown.iter().map(|_| Constraint::Length(item_h)).collect();
    let rows = Layout::vertical(constraints).split(inner);

    for (slot, (i, (label, count))) in shown.into_iter().enumerate() {
        let selected = matches!(selection, OverviewSelection::Item(c, it) if c == col_idx && it == i);
        draw_column_item(frame, rows[slot], label, *count, title, selected, dimmed, icons);
    }
}

/// A label reaching here is a fixed kind name for every item except the
/// dynamically discovered CRD-group ones (raw API group strings like
/// "gateway.networking.k8s.io", which `from_label` can't know about
/// ahead of time) — those live only in the "Custom Resources" column, so
/// that's the signal to fall back to the generic CRD icon instead of a
/// "no icon" glyph.
pub(super) fn resolve_icon_kind(label: &str, column_title: &str) -> Option<ResourceKind> {
    ResourceKind::from_label(label).or_else(|| (column_title == "Custom Resources").then_some(ResourceKind::CustomResourceList))
}

/// One item card: a rounded-border box, the kind's icon on the left of a
/// single content row, its name and live count filling the rest —
/// `<image> Pods           17`. Selecting it, herdr-style, turns the
/// whole card's border into a solid highlight color rather than just
/// tinting the background.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_column_item(frame: &mut Frame, area: Rect, label: &str, count: usize, column_title: &str, selected: bool, dimmed: bool, icons: &mut IconCache) {
    let highlight = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let (border_style, text_style, count_style) = if dimmed {
        let muted = dim_style();
        (muted, muted, muted)
    } else if selected {
        (highlight, highlight, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
    } else {
        (Style::default(), Style::default().add_modifier(Modifier::BOLD), Style::default().fg(Color::Cyan))
    };

    let block = Block::default().borders(Borders::ALL).border_type(BORDER).border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let icon_w = 3u16.min(inner.width);
    let split = Layout::horizontal([Constraint::Length(icon_w), Constraint::Min(0)]).split(inner);

    // A real vendored icon image where the terminal can render one; the
    // small emoji glyph is a fallback for when it can't (halfblocks
    // rendering). Skipped entirely while dimmed — a color emoji glyph
    // can't be muted via ANSI styling the way everything else here is,
    // so it would just sit there in full color on top of a background
    // that's supposed to read as out of focus.
    if !dimmed {
        match resolve_icon_kind(label, column_title) {
            Some(kind) => icons.draw(frame, icons.centered_square(split[0]), kind),
            None => frame.render_widget(Paragraph::new(icon_for(label)).alignment(Alignment::Center), split[0]),
        }
    }

    let count_text = count.to_string();
    let label_width = (split[1].width as usize).saturating_sub(count_text.chars().count() + 1).max(1);
    // A name too long for the row wraps onto a second one when the card is
    // tall enough to have it; otherwise it's cut with an ellipsis.
    let wrapped = if inner.height >= 2 { wrap_label(label, label_width) } else { None };
    let mut lines = match wrapped {
        Some((first, second)) => vec![
            Line::from(vec![Span::styled(format!("{first:<label_width$}"), text_style), Span::styled(count_text, count_style)]),
            Line::styled(second, text_style),
        ],
        None => vec![Line::from(vec![
            Span::styled(format!("{:<label_width$}", truncate(label, label_width)), text_style),
            Span::styled(count_text, count_style),
        ])],
    };
    // A single-row label sits on the card's first row.
    lines.truncate(inner.height as usize);
    frame.render_widget(Paragraph::new(lines), split[1]);
}

pub(super) fn icon_for(label: &str) -> &'static str {
    match label {
        "Nodes" => "🖥",
        "Namespaces" => "🗂",
        "Pods" => "📦",
        "Deployments" => "🚀",
        "ReplicaSets" => "📑",
        "StatefulSets" => "🧱",
        "DaemonSets" => "👻",
        "Jobs" => "⚙",
        "CronJobs" => "⏰",
        "ConfigMaps" => "🔧",
        "Secrets" => "🔐",
        "HPAs" => "📈",
        "Services" => "🔌",
        "Endpoints" => "🎯",
        "Ingresses" => "🚪",
        "NetworkPolicies" => "🛡",
        "PVCs" => "💿",
        "PVs" => "💾",
        "StorageClasses" => "🗄",
        "ServiceAccounts" => "🪪",
        "Roles" | "ClusterRoles" => "📜",
        "RoleBindings" | "ClusterRoleBindings" => "🔗",
        "Custom Resources" => "🧩",
        // Any other label reaching here is a dynamically discovered CRD
        // group name — same reasoning as `resolve_icon_kind`'s fallback.
        _ => "🧩",
    }
}

/// One category column, opened up into a bigger grid of the exact same
/// item cards `draw_column` draws in the compact Overview — for a
/// category with more kinds than fit in that narrow column at once
/// (Custom Resources, with many discovered API groups, is the case this
/// exists for). Scrolls vertically the same way the compact column does,
/// just over grid rows instead of single items.
pub(super) fn draw_column_detail_popup(frame: &mut Frame, title: &str, items: &[(&str, usize)], selected: usize, row_scroll: usize, icons: &mut IconCache) {
    let area = column_detail_area(frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BORDER)
        .title(title.to_string());
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if items.is_empty() {
        frame.render_widget(Paragraph::new("Nothing here.").alignment(Alignment::Center), inner);
        return;
    }

    let cols = ((inner.width + 1) / (COLUMN_WIDTH + 1)).max(1) as usize;
    let total_rows = items.len().div_ceil(cols);
    let item_h = item_height(items, COLUMN_WIDTH);
    let visible_rows = (inner.height / item_h).max(1) as usize;
    let row_scroll = row_scroll.min(total_rows.saturating_sub(visible_rows));
    let rows_shown = visible_rows.min(total_rows.saturating_sub(row_scroll));

    let row_constraints: Vec<Constraint> = (0..rows_shown).map(|_| Constraint::Length(item_h)).collect();
    let row_areas = Layout::vertical(row_constraints).split(inner);

    for (slot, row_area) in row_areas.iter().enumerate() {
        let row_idx = row_scroll + slot;
        let start = row_idx * cols;
        let row_items = &items[start..(start + cols).min(items.len())];
        let col_constraints: Vec<Constraint> = row_items.iter().map(|_| Constraint::Length(COLUMN_WIDTH)).collect();
        let col_areas = Layout::horizontal(col_constraints).spacing(1).split(*row_area);
        for (i, (item_area, (label, count))) in col_areas.iter().zip(row_items.iter()).enumerate() {
            let idx = start + i;
            draw_column_item(frame, *item_area, label, *count, title, idx == selected, false, icons);
        }
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    #[test]
    fn a_name_that_fits_is_left_alone() {
        assert_eq!(wrap_label("Pods", 20), None);
    }

    #[test]
    fn a_long_name_splits_at_the_capital_that_balances_the_rows() {
        assert_eq!(wrap_label("ClusterRoleBindings", 12), Some(("ClusterRole".into(), " Bindings".into())));
        assert_eq!(wrap_label("NetworkPolicies", 10), Some(("Network".into(), " Policies".into())));
    }

    #[test]
    fn api_groups_split_after_a_dot() {
        let (first, second) = wrap_label("gateway.networking.k8s.io", 16).unwrap();
        assert!(first.ends_with('.') && first.len() <= 16, "{first}");
        assert!(second.starts_with(' '));
    }

    #[test]
    fn an_unbreakable_name_is_hard_split_and_the_tail_truncated() {
        let (first, second) = wrap_label("Supercalifragilisticexpialidocious", 10).unwrap();
        assert_eq!(first.chars().count(), 10);
        assert!(second.chars().count() <= 10 && second.ends_with('…'));
    }

    #[test]
    fn a_column_is_tall_only_when_one_of_its_names_needs_two_rows() {
        assert_eq!(item_height(&[("Pods", 3), ("Jobs", 1)], COLUMN_WIDTH), ITEM_HEIGHT);
        assert_eq!(item_height(&[("Pods", 3), ("ClusterRoleBindings", 61)], 22), ITEM_HEIGHT_WRAPPED);
    }
}

#[cfg(test)]
mod column_detail_tests {
    use super::*;

    #[test]
    fn move_column_detail_selection_wraps_rows_within_a_single_grid() {
        // 5 items, 2 per row: [0 1] [2 3] [4]
        assert_eq!(move_column_detail_selection(5, 2, 0, Direction::Right), 1);
        assert_eq!(move_column_detail_selection(5, 2, 1, Direction::Down), 3);
        assert_eq!(move_column_detail_selection(5, 2, 4, Direction::Right), 4); // clamps, nothing after
        assert_eq!(move_column_detail_selection(5, 2, 0, Direction::Left), 0); // clamps, nothing before
    }

    #[test]
    fn column_detail_cols_and_visible_rows_are_at_least_one() {
        let tiny = Rect { x: 0, y: 0, width: 1, height: 1 };
        assert!(column_detail_cols(tiny) >= 1);
        assert!(column_detail_visible_rows(tiny, &[("Pods", 1)]) >= 1);
    }
}

#[cfg(test)]
mod centring_tests {
    use super::*;

    #[test]
    fn columns_sit_in_the_middle_of_the_space() {
        let area = Rect { x: 1, y: 0, width: 200, height: 20 };
        let columns = column_layout(area, 5);
        let used = 5 * (COLUMN_WIDTH + 1) - 1;
        let left = columns[0].x - area.x;
        let right = area.x + area.width - (columns[4].x + columns[4].width);
        assert_eq!(columns[4].x + columns[4].width - columns[0].x, used);
        assert!(left.abs_diff(right) <= 1, "left {left} right {right}");
    }

    #[test]
    fn a_full_row_of_columns_has_no_slack_to_split() {
        let area = Rect { x: 0, y: 0, width: 3 * (COLUMN_WIDTH + 1) - 1, height: 10 };
        assert_eq!(column_layout(area, 3)[0].x, 0);
    }
}
