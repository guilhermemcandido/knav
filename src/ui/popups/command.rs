//! The `:` command line and its suggestions.

use super::*;

/// Rows each suggestion takes with icons: one, so the text sits level with its icon (an
/// even number of lines can't centre a line of text against it).
const SUGGESTION_HEIGHT: u16 = 1;

/// The icon's cells: as wide as a square one row tall needs, plus a gap.
/// The most rows the menu shows at once; the rest scroll.
const MAX_SHOWN: usize = 16;
const SUGGESTION_ICON: Rect = Rect { x: 0, y: 0, width: 2, height: SUGGESTION_HEIGHT };

/// The least room a description is worth drawing in, in cells.
const MIN_DESCRIPTION: usize = 12;

/// The colour of a `key:value` filter word, or `None` for ordinary text. A filter with nothing
/// after the colon yet is dimmed: it is still being typed.
fn chip_color(word: &str) -> Option<Color> {
    let (key, value) = word.split_once(':')?;
    let color = match key.to_lowercase().as_str() {
        "group" | "g" => theme().warm,
        "ext" | "e" => theme().accent,
        "is" => theme().heading,
        _ => return None,
    };
    Some(if value.is_empty() { theme().muted } else { color })
}

/// The typed input, with each filter word drawn as a coloured chip.
fn input_spans(input: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for token in input.split_inclusive(' ') {
        let word = token.trim_end_matches(' ');
        match chip_color(word) {
            Some(bg) => {
                spans.push(Span::styled(format!(" {word} "), Style::default().bg(bg).fg(crate::theme::on(bg)).add_modifier(Modifier::BOLD)));
                spans.push(Span::raw(token[word.len()..].to_string()));
            }
            None => spans.push(Span::styled(token.to_string(), Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD))),
        }
    }
    spans
}

/// The `:` command line: the input on top and the live suggestions under it, each with
/// its icon. The best match's remaining letters show dimmed after the cursor.
pub(in crate::ui) fn draw_command_line(frame: &mut Frame, bar: Rect, input: &str, suggestions: &[SuggestionView], selected: usize, icons: &mut IconCache) {
    let width = bar.width;
    // Without icons each suggestion is a single row.
    let row_h = if icons.enabled() { SUGGESTION_HEIGHT } else { 1 };
    // As many rows as the screen has room for, scrolled to keep the selection in view.
    // Never to the screen's last rows, which hold the hints, and not a wall of rows either.
    let below = frame.area().bottom().saturating_sub(bar.y).saturating_sub(2);
    let shown = suggestions.len().min(usize::from(below.saturating_sub(4) / row_h)).min(MAX_SHOWN);
    let height = if shown == 0 { bar.height } else { shown as u16 * row_h + 4 };
    let area = Rect { x: bar.x, y: bar.y, width, height };
    frame.render_widget(Clear, area);
    let line_style = Style::default().fg(theme().command);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(line_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The rest of the highlighted name after the word being typed, whatever its case, and none
    // while that word is a filter or a space was just typed.
    let typed = input.rsplit(' ').next().filter(|word| !word.is_empty() && !word.contains(':')).map(str::to_lowercase);
    let ghost = typed
        .and_then(|typed| suggestions.get(selected).and_then(|s| if s.label.to_lowercase().starts_with(&typed) { s.label.get(typed.len()..).map(str::to_string) } else { None }))
        .unwrap_or_default();
    let mut spans = vec![Span::styled("> ", line_style)];
    spans.extend(input_spans(input));
    spans.push(Span::styled("▏", line_style));
    spans.push(Span::styled(ghost.to_string(), Style::default().fg(theme().muted)));
    let line = Line::from(spans);
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
        // A kind sits in from its API group's header, so the groups read as a tree.
        let indent = u16::from(suggestion.depth) * 3;
        // A group's header is a coloured band, so where one group ends and the next starts is plain.
        let header_color = suggestion.tags.iter().find_map(|t| match t.1 {
            TagRole::GroupRow => Some(theme().warm),
            TagRole::Native => Some(theme().heading),
            _ => None,
        });
        let style = if chosen {
            Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD)
        } else if suggestion.header {
            Style::default().bg(theme().pill_bg).fg(header_color.unwrap_or(theme().row)).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme().row)
        };
        frame.render_widget(Block::default().style(style), row);
        // A tree connector leads from the header to each of its kinds.
        if suggestion.depth > 0 {
            let connector = if suggestion.last { "└─" } else { "├─" };
            let connector_style = if chosen { style.remove_modifier(Modifier::BOLD) } else { Style::default().fg(theme().muted) };
            frame.render_widget(Paragraph::new(Span::styled(connector, connector_style)), Rect { x: row.x + 1, y: row.y, width: 2, height: 1 });
        }
        let text_x = if icons.enabled() {
            let icon_area = Rect { x: row.x + 1 + indent, y: row.y, ..SUGGESTION_ICON };
            // Icons a little inside their square, so they don't crowd the row.
            let fill = crate::config::tunables::tunables().suggestion_icon_percent as f32 / 100.0;
            let square = icons.centered_square(icon_area);
            match suggestion.icon {
                SuggestionIcon::Kind(kind) => icons.draw_kind(frame, square, kind, fill),
                SuggestionIcon::Named(name) => icons.draw_named(frame, square, name, fill),
            }
            icon_area.right() + 1
        } else {
            row.x + 1 + indent
        };
        let text = Rect { x: text_x, y: row.y + row_h / 2, width: row.right().saturating_sub(text_x), height: 1 };
        frame.render_widget(Paragraph::new(Span::styled(suggestion.label.clone(), style)), text);
        // What the row belongs to at the right end (its kind, its extension), each role in
        // its own colour. The CRD's description takes only the room left and is left out
        // when that is short, so no row ever changes size; tags that don't fit are dropped.
        let color = |role: TagRole| match role {
            TagRole::GroupRow => theme().warm,
            TagRole::Native => theme().heading,
            TagRole::Knav => theme().label,
            TagRole::Extension => theme().accent,
            TagRole::Kind => theme().text_soft,
            TagRole::Description => theme().muted,
        };
        let mut room = usize::from(text.width.saturating_sub(1)).saturating_sub(cell_width(&suggestion.label) + 3);
        let mut shown: Vec<(String, TagRole)> = Vec::new();
        for tag in suggestion.tags.iter().rev().filter(|t| t.1 != TagRole::Description) {
            let need = cell_width(&tag.0) + if shown.is_empty() { 0 } else { 2 };
            if need > room {
                break;
            }
            room -= need;
            shown.push(tag.clone());
        }
        shown.reverse();
        if let Some((description, role)) = suggestion.tags.iter().find(|t| t.1 == TagRole::Description) {
            let room = room.saturating_sub(if shown.is_empty() { 0 } else { 2 });
            if room >= MIN_DESCRIPTION {
                shown.insert(0, (truncate(description, room), *role));
            }
        }
        let mut spans = Vec::new();
        for (i, (label, role)) in shown.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            let mut tag_style = Style::default().fg(color(*role));
            if chosen {
                // On the selection's own colours, since the roles' colours can match it.
                tag_style = Style::default().fg(crate::theme::on(theme().select_bg)).bg(theme().select_bg);
                if *role == TagRole::Description {
                    tag_style = tag_style.add_modifier(Modifier::DIM);
                }
            } else if suggestion.header {
                tag_style = tag_style.bg(theme().pill_bg);
            }
            spans.push(Span::styled(label.clone(), tag_style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)).right_aligned(), Rect { width: text.width.saturating_sub(1), ..text });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(show_icons: bool) -> Vec<String> {
        let mut icons = IconCache::halfblocks();
        icons.set_enabled(show_icons);
        let suggestions: Vec<SuggestionView> = ["pods", "deployments", "nodes"].iter().map(|l| SuggestionView { label: l.to_string(), icon: SuggestionIcon::Named("home"), tags: Vec::new(), depth: 0, header: false, last: false }).collect();
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
    fn with_icons_each_suggestion_stays_one_row_so_text_meets_its_icon() {
        let rows = rows(true);
        assert!(rows[3].contains("pods") && rows[4].contains("deployments"), "{rows:#?}");
    }

    /// The rows of a menu of one group header and one kind, drawn `width` cells wide.
    fn tree(width: u16, description: &str) -> Vec<String> {
        let mut icons = IconCache::halfblocks();
        icons.set_enabled(false);
        let suggestions = vec![
            SuggestionView { label: "kagent.dev".into(), icon: SuggestionIcon::Named("home"), tags: vec![("API group".into(), TagRole::GroupRow)], depth: 0, header: true, last: false },
            SuggestionView {
                label: "agents".into(),
                icon: SuggestionIcon::Named("home"),
                tags: vec![(description.into(), TagRole::Description), ("Agent".into(), TagRole::Kind)],
                depth: 1,
                header: false,
                last: true,
            },
        ];
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 12)).unwrap();
        terminal.draw(|frame| draw_command_line(frame, Rect { x: 0, y: 0, width, height: 3 }, "ag", &suggestions, 1, &mut icons)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..12).map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn a_kind_sits_in_from_its_group_and_a_description_takes_only_the_room_left() {
        let long = "An Agent declares an LLM-backed agent that can call tools and talk to other agents";
        let wide = tree(70, long);
        assert!(wide[3].contains("kagent.dev") && wide[3].contains("API group"), "{wide:#?}");
        // One row each: the box is as tall with a description as without.
        assert!(wide[4].contains("└─ agents") && wide[4].contains("Agent"), "{wide:#?}");
        assert!(wide[4].contains('…') && wide[5].starts_with('╰'), "{wide:#?}");
        assert_eq!(tree(70, "").len(), wide.len());
        assert_eq!(tree(70, "").iter().position(|r| r.starts_with('╰')), wide.iter().position(|r| r.starts_with('╰')));
    }

    #[test]
    fn a_narrow_menu_drops_the_description_and_keeps_the_kind() {
        let rows = tree(34, "An Agent declares an LLM-backed agent");
        assert!(rows[4].contains("agents") && rows[4].contains("Agent"), "{rows:#?}");
        assert!(!rows[4].contains("declares"), "{rows:#?}");
        assert!(rows[5].starts_with('╰'), "{rows:#?}");
    }
}
