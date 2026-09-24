//! Draws whichever extension dashboard is active: one bordered, scrollable
//! page, the same shape for all of them. What it shows is entirely the
//! extension's own doing (see `extensions::dashboards`) — this only knows
//! how to frame and scroll a title and some lines, never which extension.

use super::*;

pub(super) fn draw_dashboard(frame: &mut Frame, area: Rect, title: &str, content: &[Line<'static>], scroll: usize, dimmed: bool) {
    let style = if dimmed { dim_style() } else { Style::default() };
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(style).title(Line::styled(format!(" {title} "), if dimmed { style } else { Style::default().add_modifier(Modifier::BOLD) }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let max_scroll = content.len().saturating_sub(inner.height as usize);
    let scroll = scroll.min(max_scroll);
    frame.render_widget(Paragraph::new(content.to_vec()).scroll((scroll as u16, 0)), inner);
}
