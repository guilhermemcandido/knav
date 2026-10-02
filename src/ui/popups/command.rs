//! The `:` command line and its suggestions.

use super::*;

/// Rows each suggestion takes with icons: room for a 6x3 icon beside its name.
const SUGGESTION_HEIGHT: u16 = 3;

const SUGGESTION_ICON: Rect = Rect { x: 0, y: 0, width: 6, height: SUGGESTION_HEIGHT };

/// The `:` command line: the input on top and the live suggestions under it, each with
/// its icon. The best match's remaining letters show dimmed after the cursor.
pub(in crate::ui) fn draw_command_line(frame: &mut Frame, bar: Rect, input: &str, suggestions: &[SuggestionView], selected: usize, icons: &mut IconCache) {
    let width = bar.width;
    // Without icons each suggestion is a single row.
    let row_h = if icons.enabled() { SUGGESTION_HEIGHT } else { 1 };
    // As many rows as the screen has room for, scrolled to keep the selection in view.
    let below = frame.area().bottom().saturating_sub(bar.y);
    let shown = suggestions.len().min(usize::from(below.saturating_sub(4) / row_h));
    let height = if shown == 0 { bar.height } else { shown as u16 * row_h + 4 };
    let area = Rect { x: bar.x, y: bar.y, width, height };
    frame.render_widget(Clear, area);
    let line_style = Style::default().fg(theme().command);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(line_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Complete against the name, not the alias note in "namespaces (ns)".
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
        let row = Rect { x: inner.x, y: rows_top + (n - start) as u16 * row_h, width: inner.width, height: row_h };
        let chosen = n == selected;
        let style = if chosen {
            Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD)
        } else if suggestion.heading {
            Style::default().fg(theme().muted)
        } else {
            Style::default().fg(theme().row)
        };
        frame.render_widget(Block::default().style(style), row);
        let text_x = if icons.enabled() {
            let icon_area = Rect { x: row.x + 1, y: row.y, ..SUGGESTION_ICON };
            // Icons a little inside their square, so they don't crowd the row.
            let fill = crate::config::tunables::tunables().suggestion_icon_percent as f32 / 100.0;
            let square = icons.centered_square(icon_area);
            match suggestion.icon {
                SuggestionIcon::Kind(kind) => icons.draw_kind(frame, square, kind, fill),
                SuggestionIcon::Named(name) => icons.draw_named(frame, square, name, fill),
            }
            icon_area.right() + 1
        } else {
            row.x + 1
        };
        let text = Rect { x: text_x, y: row.y + row_h / 2, width: row.right().saturating_sub(text_x), height: 1 };
        let mut spans = Vec::new();
        if let Some(last) = suggestion.branch {
            let branch = if chosen { style } else { Style::default().fg(theme().muted) };
            spans.push(Span::styled(if last { "└─ " } else { "├─ " }, branch));
        }
        spans.push(Span::styled(suggestion.label.clone(), style));
        frame.render_widget(Paragraph::new(Line::from(spans)), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(show_icons: bool) -> Vec<String> {
        let mut icons = IconCache::halfblocks();
        icons.set_enabled(show_icons);
        let suggestions: Vec<SuggestionView> = ["pods", "deployments", "nodes"].iter().map(|l| SuggestionView { label: l.to_string(), icon: SuggestionIcon::Named("home"), branch: None, heading: false }).collect();
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 20)).unwrap();
        terminal.draw(|frame| draw_command_line(frame, Rect { x: 0, y: 0, width: 40, height: 3 }, "", &suggestions, 0, &mut icons)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..20).map(|y| (0..40).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn without_icons_each_suggestion_is_one_row() {
        let rows = rows(false);
        let text = |i: usize| rows[i].trim_matches(|c| c == '│' || c == ' ').to_string();
        assert_eq!([text(3), text(4), text(5)], ["pods", "deployments", "nodes"], "{rows:#?}");
        assert!(rows[6].starts_with('╰'), "the box closes right after the last one: {rows:#?}");
    }

    #[test]
    fn with_icons_each_suggestion_takes_three_rows() {
        let rows = rows(true);
        assert!(rows[4].contains("pods") && rows[7].contains("deployments"), "{rows:#?}");
    }
}
