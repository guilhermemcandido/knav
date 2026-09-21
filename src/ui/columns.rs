//! The Overview's catalog columns and the opened-up column popup, plus hit-testing/scrolling for them.

use super::*;
use crate::k8s::Health;

/// Column width for the resource-switcher menu's own tile grid (see
/// `menu_cols`/`draw_menu_popup`), the Overview page no longer uses
/// fixed-size tiles at all, but the menu still does.
pub(super) const TILE_WIDTH: u16 = 22;
/// One catalog column's fixed width in the Overview, borders included. A 1-cell
/// gap separates columns (see `column_layout`).
pub(super) const COLUMN_WIDTH: u16 = 28;
/// An item card's height: a rounded-border top edge, one content row (icon
/// on the left, name + live count filling the rest), and a rounded-border
/// bottom edge.
pub(super) const ITEM_HEIGHT: u16 = 3;
/// The taller card for a column with a name too long for one row. Every card in
/// that column uses it so they stay aligned.
pub(super) const ITEM_HEIGHT_WRAPPED: u16 = 4;

/// Room for a card's label on one row at a given column width: the column
/// and card borders (2 + 2), the icon (3) and the count with its space.
fn label_room(column_width: u16, count: usize) -> usize {
    (usize::from(column_width)).saturating_sub(7 + count.to_string().len() + 1).max(1)
}

/// Splits a name over two rows at a capital, e.g. `ClusterRoleBindings` becomes
/// `ClusterRole` and ` Bindings`. Falls back to a `.` boundary, then a hard split.
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

/// Kinds whose card carries a health bar on a second row.
pub(super) fn shows_health(label: &str) -> bool {
    matches!(label, "Nodes" | "Namespaces" | "Pods" | "Deployments" | "ReplicaSets" | "StatefulSets" | "DaemonSets" | "Jobs" | "CronJobs" | "HPAs" | "Services" | "Endpoints" | "Ingresses" | "PVCs" | "PVs")
}

/// The opened-up category view draws bigger cards, with a health readout where a kind has one.
pub(super) const DETAIL_WIDTH: u16 = 42;
pub(super) const DETAIL_HEIGHT: u16 = 6;

/// The card size in the opened-up category view: the same for every kind.
pub(super) fn detail_card(_items: &[(&str, usize)]) -> (u16, u16) {
    (DETAIL_WIDTH, DETAIL_HEIGHT)
}

/// A bar `width` cells wide: green for what is fine, yellow for what needs a
/// look, red for what is broken, the rest muted.
pub(super) fn health_bar(health: Health, total: usize, width: usize, dimmed: bool) -> Line<'static> {
    let paint = |color: Color| if dimmed { dim_style() } else { Style::default().fg(color) };
    let bar = width.max(1);
    if total == 0 {
        return Line::styled("░".repeat(bar), paint(theme().muted));
    }
    // Each state gets its share of the bar (largest remainders, so the bar is
    // always full), and never disappears if it exists. The neutral rest is what
    // is neither ok, warning nor error (finished, nothing wanted).
    let counts = [health.good, health.warn, health.bad, total.saturating_sub(health.good + health.warn + health.bad)];
    let sum: usize = counts.iter().sum();
    let mut cells = counts.map(|n| n * bar / sum);
    let mut order: Vec<usize> = (0..4).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(counts[i] * bar % sum));
    for &i in order.iter().cycle().take(bar - cells.iter().sum::<usize>()) {
        cells[i] += 1;
    }
    for i in [2, 1, 0] {
        if counts[i] > 0 && cells[i] == 0 {
            let biggest = (0..4).max_by_key(|&j| cells[j]).unwrap_or(0);
            cells[biggest] -= 1;
            cells[i] = 1;
        }
    }
    let [good, warn, bad, rest] = cells;
    Line::from(vec![
        Span::styled("█".repeat(good), paint(theme().ok)),
        Span::styled("█".repeat(warn), paint(theme().warn)),
        Span::styled("█".repeat(bad), paint(theme().bad)),
        Span::styled("░".repeat(rest), paint(theme().muted)),
    ])
}

/// 1234 as `1.2k`, 15000 as `15k`, so big clusters still fit.
fn compact(n: usize) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=9_999 => format!("{}.{}k", n / 1000, n % 1000 / 100),
        10_000..=999_999 => format!("{}k", n / 1000),
        _ => format!("{}M", n / 1_000_000),
    }
}

/// `● 14 ok  ● 2 warning  ● 1 error`, only the states that have objects. It gives up
/// the words, then exact numbers, to fit `width`.
pub(super) fn health_legend(health: Health, total: usize, width: usize, dimmed: bool) -> Line<'static> {
    let paint = |color: Color| if dimmed { dim_style() } else { Style::default().fg(color) };
    if total == 0 {
        return Line::styled("none", paint(theme().muted));
    }
    let groups = [(health.good, "ok", theme().ok), (health.warn, "warning", theme().warn), (health.bad, "error", theme().bad)];
    let build = |words: bool, short: bool| -> Vec<(String, Color)> {
        groups
            .iter()
            .filter(|(n, _, _)| *n > 0)
            .map(|(n, word, color)| {
                let number = if short { compact(*n) } else { n.to_string() };
                (if words { format!("● {number} {word}") } else { format!("● {number}") }, *color)
            })
            .collect()
    };
    let fits = |parts: &[(String, Color)]| parts.iter().map(|(t, _)| t.chars().count()).sum::<usize>() + 2 * parts.len().saturating_sub(1) <= width;
    let parts = [(true, false), (false, false), (true, true), (false, true)].into_iter().map(|(w, s)| build(w, s)).find(|p| fits(p)).unwrap_or_default();
    if parts.is_empty() {
        return Line::styled(format!("{} total", compact(total)), paint(theme().muted));
    }
    let mut spans = Vec::new();
    for (text, color) in parts {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(text, paint(color)));
    }
    Line::from(spans)
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
/// columns area (see `columns_inner`), just wide enough for a single
/// arrow glyph.
pub(super) const SCROLL_ARROW_WIDTH: u16 = 1;

/// The columns area is whatever's left below the fixed dashboard strip
/// callers (keyboard navigation, mouse hit-testing) need this same
/// rectangle to stay in sync with what's actually rendered.
pub fn columns_area(frame_area: Rect, overview: &Overview) -> Rect {
    // +1 for the same gap `draw_overview` puts between the top strip and
    // the columns, Resources-to-Events and top-strip-to-columns are now
    // both a single blank row, not one bigger than the other.
    let top_h = top_area_height(overview) + 1;
    Rect { x: frame_area.x, y: frame_area.y + top_h, width: frame_area.width, height: frame_area.height.saturating_sub(top_h) }
}

/// The columns area minus the scroll-arrow gutters. Drawing and hit-testing both
/// use it so arrows never overlap the boxes.
pub(super) fn columns_inner(area: Rect) -> Rect {
    let shrink = SCROLL_ARROW_WIDTH * 2;
    Rect { x: area.x + SCROLL_ARROW_WIDTH, y: area.y, width: area.width.saturating_sub(shrink), height: area.height }
}

pub fn visible_columns(width: u16, total_columns: usize) -> usize {
    // Each column takes `COLUMN_WIDTH` plus a 1-cell gap before the next
    // one (see `column_layout`), so `n` columns actually need
    // `n * (COLUMN_WIDTH + 1) - 1` cells, not `n * COLUMN_WIDTH`.
    let cols = ((width + 1) / (COLUMN_WIDTH + 1)).max(1) as usize;
    cols.min(total_columns.max(1))
}

/// How many item cards fit vertically in one column. All columns share the height.
pub fn visible_items_per_column(columns_area_height: u16, item_height: u16) -> usize {
    (columns_area_height.saturating_sub(2) / item_height).max(1) as usize
}

/// The shared column-rect layout, `draw_columns` and `column_hit` must
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
/// renders into, one place so its own draw pass, the grid column count,
/// and the visible-row count can't drift apart.
pub(super) fn column_detail_area(frame_area: Rect) -> Rect {
    centered_rect(85, 80, frame_area)
}

/// How many item cards fit per row in a column-detail popup.
pub fn column_detail_cols(frame_area: Rect, items: &[(&str, usize)]) -> usize {
    let inner = Block::default().borders(Borders::ALL).inner(column_detail_area(frame_area));
    ((inner.width + 1) / (detail_card(items).0 + 1)).max(1) as usize
}

/// How many grid rows of item cards fit vertically in a column-detail
/// popup at once.
pub fn column_detail_visible_rows(frame_area: Rect, items: &[(&str, usize)]) -> usize {
    let inner = Block::default().borders(Borders::ALL).inner(column_detail_area(frame_area));
    (inner.height / detail_card(items).1).max(1) as usize
}

/// Movement for a column-detail popup's item grid: `move_selection` with one
/// section, clamping at the edges.
pub fn move_column_detail_selection(items_len: usize, cols: usize, selected: usize, dir: Direction) -> usize {
    move_selection(&[items_len], cols, (0, selected), dir).1
}

/// Adjusts a scroll offset so `target` lies inside the `visible` window. Used for
/// the Overview's horizontal and per-column vertical scroll.
pub fn scroll_columns_to_show(col_scroll: usize, cols_visible: usize, target_col: usize) -> usize {
    if target_col < col_scroll {
        target_col
    } else if target_col >= col_scroll + cols_visible {
        target_col + 1 - cols_visible
    } else {
        col_scroll
    }
}

/// Which column header or item, or the Resources/Events box, is under a terminal
/// position. It uses the same layout as drawing. `active_col` and `item_scroll`
/// must match the last `draw_columns` call.
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

/// Draws the columns plus a "◀" / "▶" arrow in the gutters when scrolling that
/// way would reveal another column.
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

    let arrow_style = if dimmed { dim_style() } else { Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD) };
    if col_scroll > 0 {
        let left = Rect { x: area.x, y: area.y, width: SCROLL_ARROW_WIDTH, height: 1 };
        frame.render_widget(Paragraph::new(Span::styled("◀", arrow_style)), left);
    }
    if col_scroll + cols_visible < total {
        let right = Rect { x: (area.x + area.width).saturating_sub(SCROLL_ARROW_WIDTH), y: area.y, width: SCROLL_ARROW_WIDTH, height: 1 };
        frame.render_widget(Paragraph::new(Span::styled("▶", arrow_style)), right);
    }
}

/// One column: a rounded box titled with the category, its border highlighted when
/// the header is selected, listing the kinds as item cards. `item_scroll` applies
/// only to the selected column.
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
    let highlight = Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD);

    let (border_style, title_style) = if dimmed {
        (dim_style(), dim_style())
    } else if header_selected {
        (highlight, highlight)
    } else {
        (Style::default(), Style::default().add_modifier(Modifier::BOLD))
    };

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
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
        draw_column_item(frame, rows[slot], label, *count, None, title, selected, dimmed, icons);
    }
}

/// Labels here are fixed kind names except discovered CRD groups (raw API groups),
/// which only appear under "Custom Resources" and get the generic CRD icon.
pub(super) fn resolve_icon_kind(label: &str, column_title: &str) -> Option<ResourceKind> {
    ResourceKind::from_label(label).or_else(|| (column_title == "Custom Resources").then_some(ResourceKind::CustomResourceList))
}

/// One item card: the kind's icon, its name and live count, e.g. `<image> Pods  17`.
/// Selecting it turns the whole border into the highlight colour.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_column_item(frame: &mut Frame, area: Rect, label: &str, count: usize, health: Option<Health>, column_title: &str, selected: bool, dimmed: bool, icons: &mut IconCache) {
    let highlight = Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD);
    let (border_style, text_style, count_style) = if dimmed {
        let muted = dim_style();
        (muted, muted, muted)
    } else if selected {
        (highlight, highlight, Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD))
    } else {
        (Style::default(), Style::default().add_modifier(Modifier::BOLD), Style::default().fg(theme().namespace))
    };

    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    // Roomy cards (the opened-up view) get a bigger icon, a gap after it and
    // one cell of padding on the right.
    let roomy = inner.height >= 4;
    let icon_w = if roomy { 7 } else { 3 }.min(inner.width);
    let parts = Layout::horizontal([Constraint::Length(icon_w), Constraint::Min(0), Constraint::Length(u16::from(roomy))]).split(inner);
    let split = [Rect { width: icon_w.saturating_sub(u16::from(roomy) * 2), x: parts[0].x + u16::from(roomy), ..parts[0] }, parts[1]];

    // A vendored image where the terminal can render one, else a small emoji glyph.
    // Skipped while dimmed, since an emoji can't be muted with ANSI styling.
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
    // The opened-up view adds a bar and what it is made of.
    if roomy {
        lines.push(Line::raw(""));
    }
    if let Some(health) = health {
        lines.push(health_bar(health, count, split[1].width as usize, dimmed));
        lines.push(health_legend(health, count, split[1].width as usize, dimmed));
    }
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
        // group name, same reasoning as `resolve_icon_kind`'s fallback.
        _ => "🧩",
    }
}

/// One category column opened into a bigger grid of the same cards, for categories
/// with many kinds (Custom Resources). Scrolls by grid row.
pub(super) fn draw_column_detail_popup(frame: &mut Frame, title: &str, items: &[(&str, usize)], health: &std::collections::HashMap<&'static str, Health>, selected: usize, row_scroll: usize, icons: &mut IconCache) {
    let area = column_detail_area(frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .title(title.to_string());
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if items.is_empty() {
        frame.render_widget(Paragraph::new("Nothing here.").alignment(Alignment::Center), inner);
        return;
    }

    let (card_w, item_h) = detail_card(items);
    let cols = ((inner.width + 1) / (card_w + 1)).max(1) as usize;
    let total_rows = items.len().div_ceil(cols);
    let visible_rows = (inner.height / item_h).max(1) as usize;
    let row_scroll = row_scroll.min(total_rows.saturating_sub(visible_rows));
    let rows_shown = visible_rows.min(total_rows.saturating_sub(row_scroll));

    let row_constraints: Vec<Constraint> = (0..rows_shown).map(|_| Constraint::Length(item_h)).collect();
    let row_areas = Layout::vertical(row_constraints).split(inner);

    for (slot, row_area) in row_areas.iter().enumerate() {
        let row_idx = row_scroll + slot;
        let start = row_idx * cols;
        let row_items = &items[start..(start + cols).min(items.len())];
        let col_constraints: Vec<Constraint> = row_items.iter().map(|_| Constraint::Length(card_w)).collect();
        let col_areas = Layout::horizontal(col_constraints).spacing(1).split(*row_area);
        for (i, (item_area, (label, count))) in col_areas.iter().zip(row_items.iter()).enumerate() {
            let idx = start + i;
            draw_column_item(frame, *item_area, label, *count, Some(health.get(label).copied().unwrap_or_default()).filter(|_| shows_health(label) && *count > 0), title, idx == selected, false, icons);
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
mod health_tests {
    use super::*;

    #[test]
    fn the_bar_fills_its_width() {
        assert_eq!(health_bar(Health { good: 15, warn: 1, bad: 1 }, 17, 20, false).width(), 20);
    }

    #[test]
    fn a_bar_of_everything_accounted_for_has_no_grey() {
        let line = health_bar(Health { good: 14, warn: 1, bad: 2 }, 17, 40, false);
        assert_eq!(line.spans[3].content.chars().count(), 0);
        assert_eq!(line.width(), 40);
    }

    #[test]
    fn a_broken_object_always_shows_even_among_many() {
        let line = health_bar(Health { good: 199, warn: 0, bad: 1 }, 200, 20, false);
        assert_eq!(line.spans[2].content.chars().count(), 1, "one red cell");
    }

    #[test]
    fn the_legend_lists_only_states_that_exist() {
        let text: String = health_legend(Health { good: 14, warn: 0, bad: 3 }, 17, 30, false).spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "● 14 ok  ● 3 error");
        assert_eq!(health_legend(Health::default(), 0, 30, false).spans[0].content, "none");
    }

    #[test]
    fn a_big_cluster_still_fits_the_card() {
        let big = Health { good: 41_250, warn: 1_800, bad: 950 };
        for width in [31, 20, 12] {
            assert!(health_legend(big, 44_000, width, false).width() <= width.max(12), "{width}");
        }
        assert_eq!(compact(1_234), "1.2k");
        assert_eq!(compact(15_000), "15k");
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
        assert!(column_detail_cols(tiny, &[("Pods", 1)]) >= 1);
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
