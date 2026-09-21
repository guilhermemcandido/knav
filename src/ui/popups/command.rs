//! The `:` command line and its suggestions.

use super::*;

/// Rows each suggestion takes: room for a 6x3 icon beside its name.
const SUGGESTION_HEIGHT: u16 = 3;

const SUGGESTION_ICON: Rect = Rect { x: 0, y: 0, width: 6, height: SUGGESTION_HEIGHT };

/// The `:` command line, k9s-style: one full-width box with the input on top and the live
/// autocomplete listed under it, each match with its icon. The best match's
/// remaining letters show dimmed after the cursor.
pub(in crate::ui) fn draw_command_line(frame: &mut Frame, bar: Rect, input: &str, suggestions: &[SuggestionView], selected: usize, icons: &mut IconCache) {
    let width = bar.width;
    // As many rows as the screen has room for, scrolled to keep the selection in view.
    let below = frame.area().bottom().saturating_sub(bar.y);
    let shown = suggestions.len().min(usize::from(below.saturating_sub(4) / SUGGESTION_HEIGHT));
    let height = if shown == 0 { bar.height } else { shown as u16 * SUGGESTION_HEIGHT + 4 };
    let area = Rect { x: bar.x, y: bar.y, width, height };
    frame.render_widget(Clear, area);
    let line_style = Style::default().fg(theme().command);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(line_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // "namespaces (ns)": complete against the name, not the alias note.
    let ghost = suggestions
        .get(selected)
        .and_then(|s| s.label.split(" (").next())
        .and_then(|name| name.strip_prefix(input))
        .unwrap_or("");
    let line = Line::from(vec![
        Span::styled("> ", line_style),
        Span::styled(input.to_string(), Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD)),
        Span::styled("▏", line_style),
        Span::styled(ghost.to_string(), Style::default().fg(theme().muted)),
    ]);
    frame.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
    if shown == 0 {
        return;
    }

    // A divider joined to the box's sides, then the matches.
    let set = border_set();
    let (left, right) = match set.horizontal_top {
        "━" => ("┣", "┫"),
        "═" => ("╠", "╣"),
        _ => ("├", "┤"),
    };
    let divider = format!("{left}{}{right}", set.horizontal_top.repeat(usize::from(width.saturating_sub(2))));
    frame.render_widget(Paragraph::new(Span::styled(divider, line_style)), Rect { x: area.x, y: inner.y + 1, width, height: 1 });
    let start = (selected + 1).saturating_sub(shown).min(suggestions.len() - shown);
    let rows_top = inner.y + 2;
    for (n, suggestion) in suggestions.iter().enumerate().skip(start).take(shown) {
        let row = Rect { x: inner.x, y: rows_top + (n - start) as u16 * SUGGESTION_HEIGHT, width: inner.width, height: SUGGESTION_HEIGHT };
        let chosen = n == selected;
        let style = if chosen { Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme().row) };
        frame.render_widget(Block::default().style(style), row);
        let icon_area = Rect { x: row.x + 1, y: row.y, ..SUGGESTION_ICON };
        // All the same size, a little inside the square so they don't crowd the row.
        let fill = crate::config::tunables::tunables().suggestion_icon_percent as f32 / 100.0;
        let square = icons.centered_square(icon_area);
        match suggestion.icon {
            SuggestionIcon::Kind(kind) => icons.draw_kind(frame, square, kind, fill),
            SuggestionIcon::Named(name) => icons.draw_named(frame, square, name, fill),
        }
        let text = Rect { x: icon_area.right() + 1, y: row.y + 1, width: row.right().saturating_sub(icon_area.right() + 1), height: 1 };
        frame.render_widget(Paragraph::new(Span::styled(suggestion.label.clone(), style)), text);
    }
}
