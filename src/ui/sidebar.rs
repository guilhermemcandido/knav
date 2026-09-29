//! The resource sidebar (`m`): every category and kind on the left, like an IDE's explorer.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRow {
    pub label: String,
    /// A collapsible category heading rather than a kind.
    pub heading: bool,
    pub collapsed: bool,
    pub count: Option<usize>,
    /// This is the list on screen.
    pub current: bool,
}

pub struct Sidebar {
    pub rows: Vec<SidebarRow>,
    /// The row the cursor is on.
    pub selected: usize,
    /// The keys are moving in it.
    pub focused: bool,
}


/// Narrower terminals have no room for it beside a list.
pub const SIDEBAR_MIN_WIDTH: u16 = 90;
const SIDEBAR_WIDTH: u16 = 28;

/// The sidebar's width at `full_width` columns; 0 when it is off.
fn sidebar_width(full_width: u16, chrome: &Chrome) -> u16 {
    if full_width >= SIDEBAR_MIN_WIDTH && chrome.sidebar.is_some() { SIDEBAR_WIDTH } else { 0 }
}

/// The first row shown, keeping the cursor in view.
pub fn sidebar_scroll(selected: usize, len: usize, height: usize) -> usize {
    selected.saturating_sub(height.saturating_sub(1) / 2).min(len.saturating_sub(height))
}

/// The row under a screen position, given the sidebar sits in `area`.
pub fn sidebar_row_at(area: Rect, selected: usize, len: usize, column: u16, row: u16) -> Option<usize> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    if column < inner.x || column >= inner.x + inner.width || row < inner.y || row >= inner.y + inner.height {
        return None;
    }
    let index = sidebar_scroll(selected, len, usize::from(inner.height)) + usize::from(row - inner.y);
    (index < len).then_some(index)
}

pub fn sidebar_area(frame_area: Rect, shortcuts_line: bool, chrome: &Chrome) -> Rect {
    let body = body_area(frame_area, shortcuts_line);
    Rect { width: sidebar_width(frame_area.width, chrome).min(body.width), ..body }
}

pub fn beside_sidebar(frame_area: Rect, shortcuts_line: bool, chrome: &Chrome) -> Rect {
    let body = body_area(frame_area, shortcuts_line);
    let taken = sidebar_width(frame_area.width, chrome).min(body.width);
    Rect { x: body.x + taken, width: body.width - taken, ..body }
}

pub(super) fn draw_sidebar(frame: &mut Frame, area: Rect, dimmed: bool, chrome: &Chrome) {
    if area.width == 0 {
        return;
    }
    let Some(sidebar) = chrome.sidebar.as_ref() else { return };
    let border = if dimmed {
        dim_style()
    } else if sidebar.focused {
        Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)
    } else {
        theme_border(false)
    };
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border).title(pill_title("Browse", dimmed, border));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = usize::from(inner.height);
    let first = sidebar_scroll(sidebar.selected, sidebar.rows.len(), height);
    let width = usize::from(inner.width);
    let lines: Vec<Line> = sidebar
        .rows
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, row)| {
            let cursor = sidebar.focused && i == sidebar.selected && !dimmed;
            let base = if dimmed {
                dim_style()
            } else if cursor {
                Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD)
            } else if row.current {
                Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)
            } else if row.heading {
                Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD)
            } else if row.count == Some(0) {
                // Empty kinds are quiet, so the ones with objects stand out.
                Style::default().fg(theme().muted)
            } else {
                Style::default().fg(theme().row)
            };
            let marker = if row.heading { if row.collapsed { "▸ " } else { "▾ " } } else { "  " };
            let left = format!("{marker}{}", row.label);
            let count = row.count.map(|c| c.to_string()).unwrap_or_default();
            let room = width.saturating_sub(cell_width(&count) + 1);
            let left = truncate(&left, room);
            let pad = width.saturating_sub(cell_width(&left) + cell_width(&count));
            let count_style = if cursor || dimmed { base } else { Style::default().fg(theme().muted) };
            Line::from(vec![Span::styled(left, base), Span::styled(" ".repeat(pad), base), Span::styled(count, count_style)])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_stays_in_view_and_clicks_land_on_the_row_drawn_there() {
        assert_eq!(sidebar_scroll(0, 40, 10), 0);
        let first = sidebar_scroll(30, 40, 10);
        assert!(first <= 30 && 30 < first + 10);
        assert_eq!(sidebar_scroll(39, 40, 10), 30, "never scrolls past the end");
        let area = Rect { x: 0, y: 3, width: 28, height: 12 };
        let scroll = sidebar_scroll(30, 40, 10);
        assert_eq!(sidebar_row_at(area, 30, 40, 2, 4), Some(scroll));
        assert_eq!(sidebar_row_at(area, 30, 40, 0, 4), None, "the border");
    }

    #[test]
    fn the_sidebar_takes_room_only_when_the_chrome_has_one() {
        let full = Rect { x: 0, y: 0, width: 120, height: 40 };
        let none = Chrome::default();
        let with = Chrome { sidebar: Some(Sidebar { rows: Vec::new(), selected: 0, focused: false }), ..Chrome::default() };
        assert_eq!(beside_sidebar(full, true, &none).x, 0);
        assert_eq!(beside_sidebar(full, true, &with).x, SIDEBAR_WIDTH);
        let narrow = Rect { width: SIDEBAR_MIN_WIDTH - 1, ..full };
        assert_eq!(beside_sidebar(narrow, true, &with).x, 0, "too narrow for it");
    }
}
