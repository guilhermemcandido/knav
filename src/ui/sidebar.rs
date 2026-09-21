//! The resource sidebar (`m`): every category and kind on the left, like an IDE's explorer.

use super::*;

/// One line of the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRow {
    pub label: String,
    /// A category heading (collapsible) rather than a kind.
    pub heading: bool,
    /// Whether a heading is folded.
    pub collapsed: bool,
    pub count: Option<usize>,
    /// The list on screen is this one.
    pub current: bool,
}

pub struct Sidebar {
    pub rows: Vec<SidebarRow>,
    /// The row the cursor is on.
    pub selected: usize,
    /// The keys are moving in it.
    pub focused: bool,
}

static SIDEBAR: std::sync::RwLock<Option<Sidebar>> = std::sync::RwLock::new(None);

/// Sets (or clears) the sidebar drawn at the left of the body.
pub fn set_sidebar(sidebar: Option<Sidebar>) {
    if let Ok(mut slot) = SIDEBAR.write() {
        *slot = sidebar;
    }
}

/// Narrower terminals have no room for it beside a list.
pub const SIDEBAR_MIN_WIDTH: u16 = 90;
const SIDEBAR_WIDTH: u16 = 28;

/// How wide the sidebar is at `full_width` columns; 0 when it is off.
pub fn sidebar_width(full_width: u16) -> u16 {
    if full_width >= SIDEBAR_MIN_WIDTH && SIDEBAR.read().is_ok_and(|s| s.is_some()) { SIDEBAR_WIDTH } else { 0 }
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

/// The rectangle the sidebar takes out of the body.
pub fn sidebar_area(frame_area: Rect, shortcuts_line: bool) -> Rect {
    let body = body_area(frame_area, shortcuts_line);
    Rect { width: sidebar_width(frame_area.width).min(body.width), ..body }
}

/// What is left of the body once the sidebar has taken its share.
pub fn beside_sidebar(frame_area: Rect, shortcuts_line: bool) -> Rect {
    let body = body_area(frame_area, shortcuts_line);
    let taken = sidebar_width(frame_area.width).min(body.width);
    Rect { x: body.x + taken, width: body.width - taken, ..body }
}

pub(super) fn draw_sidebar(frame: &mut Frame, area: Rect, dimmed: bool) {
    if area.width == 0 {
        return;
    }
    let Ok(sidebar) = SIDEBAR.read() else { return };
    let Some(sidebar) = sidebar.as_ref() else { return };
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
                // Nothing in it: quiet, so what has objects stands out.
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
}
