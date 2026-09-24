//! The settings screen and key capture.

use super::*;

/// Everything `draw_settings` reads but doesn't own, gathered so the
/// function takes one argument instead of a fistful.
pub(in crate::ui) struct SettingsView<'a> {
    pub tab: SettingsTab,
    pub rows: &'a [SettingView],
    pub layout: &'a [LayoutRow],
    pub error: Option<&'a str>,
    pub capture: Option<&'a CaptureView>,
}

/// The settings screen: one row per setting under its section, the value in
/// bold when the config file sets it, and a swatch for colours.
pub(in crate::ui) fn draw_settings(frame: &mut Frame, view: SettingsView, state: &mut TableState) {
    let SettingsView { tab, rows, layout, error, capture } = view;
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let bottom = match error {
        Some(e) => Line::styled(format!(" {e} "), Style::default().fg(theme().bad)),
        None if tab == SettingsTab::Overview => hint_strip(&[("J/K", "move"), ("space", "show/hide"), ("r", "reset"), ("tab", "next"), ("esc", "close")]).right_aligned(),
        None => hint_strip(&[("←→", "change"), ("enter", "edit"), ("r", "reset"), ("tab", "next"), ("esc", "close")]).right_aligned(),
    };
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(theme_border(false)).title(settings_tabs(tab).centered()).title_bottom(bottom);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // The selected setting's explanation takes the last two lines.
    let help = if tab == SettingsTab::Overview {
        "Give each category its place from the left: press a number to put the selected one there, or K and J to nudge it. Space hides or shows it."
    } else {
        state.selected().and_then(|i| rows.get(i)).map(|r| r.help).unwrap_or("")
    };
    let split = Layout::vertical([Constraint::Min(1), Constraint::Length(if inner.height > 6 { 3 } else { 0 })]).split(inner);
    let (inner, help_area) = (split[0], split[1]);
    if help_area.height > 0 {
        let text = Paragraph::new(help).style(Style::default().fg(theme().muted)).wrap(Wrap { trim: true }).block(Block::default().borders(Borders::TOP).border_style(theme_border(false)));
        frame.render_widget(text, help_area);
    }
    if tab == SettingsTab::Overview {
        draw_layout_rows(frame, inner, layout, state);
        if let Some(capture) = capture {
            draw_key_capture(frame, capture);
        }
        return;
    }
    let mut last_section = "";
    let table_rows: Vec<Row> = rows
        .iter()
        .map(|row| {
            let section = if row.section == last_section { "" } else { row.section };
            last_section = row.section;
            let mut value = Vec::new();
            if let Some(color) = row.swatch {
                value.push(Span::styled("● ", Style::default().fg(color)));
            }
            let style = if row.editing {
                Style::default().fg(theme().highlight).add_modifier(Modifier::BOLD)
            } else if row.customised {
                Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme().desc)
            };
            value.push(Span::styled(if row.editing { format!("{}▏", row.value) } else { row.value.clone() }, style));
            let note = if row.restart { "restart to apply" } else if row.customised { "custom" } else { "" };
            Row::new(vec![
                Cell::from(Span::styled(section, Style::default().fg(theme().heading).add_modifier(Modifier::BOLD))),
                Cell::from(row.label.clone()),
                Cell::from(Line::from(value)),
                Cell::from(Span::styled(note, Style::default().fg(theme().muted))),
            ])
        })
        .collect();
    let table = Table::new(table_rows, [Constraint::Length(14), Constraint::Length(34), Constraint::Length(24), Constraint::Min(10)])
        .column_spacing(2)
        .style(theme_row(false))
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, false));
    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(rows.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, inner, state);
    if let Some(capture) = capture {
        draw_key_capture(frame, capture);
    }
}

/// The tab names for the top border, the current one lit.
fn settings_tabs(current: SettingsTab) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, tab) in SettingsTab::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("│", Style::default().fg(theme().muted)));
        }
        let style = if *tab == current { Style::default().fg(theme().accent).add_modifier(Modifier::BOLD | Modifier::UNDERLINED) } else { Style::default().fg(theme().muted) };
        spans.push(Span::styled(format!(" {} ", tab.label()), style));
    }
    Line::from(spans)
}

/// Which tab a click on the top border lands on.
pub fn settings_tab_at(frame_area: Rect, column: u16, row: u16) -> Option<SettingsTab> {
    let area = body_area(frame_area, true);
    if row != area.y {
        return None;
    }
    let width: usize = SettingsTab::ALL.iter().map(|t| t.label().chars().count() + 2).sum::<usize>() + SettingsTab::ALL.len() - 1;
    let mut x = usize::from(area.x) + usize::from(area.width).saturating_sub(width) / 2;
    for (i, tab) in SettingsTab::ALL.iter().enumerate() {
        if i > 0 {
            x += 1;
        }
        let end = x + tab.label().chars().count() + 2;
        if (x..end).contains(&usize::from(column)) {
            return Some(*tab);
        }
        x = end;
    }
    None
}

/// The Overview layout editor: each category with its place from the left,
/// and a line showing the result.
fn draw_layout_rows(frame: &mut Frame, area: Rect, layout: &[LayoutRow], state: &mut TableState) {
    let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(4)]).split(area);
    let rows: Vec<Row> = layout
        .iter()
        .map(|row| {
            let style = if row.hidden { Style::default().fg(theme().muted) } else { Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD) };
            Row::new(vec![
                Cell::from(Span::styled(format!(" {} ", row.number), Style::default().fg(theme().key).add_modifier(Modifier::BOLD))),
                Cell::from(Span::styled(row.name.clone(), style)),
                Cell::from(Span::styled(if row.hidden { "hidden" } else { "" }, Style::default().fg(theme().muted))),
            ])
        })
        .collect();
    let table = Table::new(rows, [Constraint::Length(5), Constraint::Length(24), Constraint::Min(8)]).column_spacing(2).style(theme_row(false)).row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, false));
    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(layout.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, parts[0], state);
    let shown: Vec<&str> = layout.iter().filter(|r| !r.hidden).map(|r| r.name.as_str()).collect();
    let label_area = Rect { height: 1, ..parts[1] };
    frame.render_widget(Paragraph::new(Span::styled("Left to right:", Style::default().fg(theme().muted))), label_area);
    let boxes_area = Rect { y: parts[1].y + 1, height: parts[1].height.saturating_sub(1), ..parts[1] };
    draw_category_boxes(frame, boxes_area, &shown);
}

/// The layout preview as actual boxes, one per visible category, left to
/// right in the order they'll show on the Overview — closer to what you're
/// really changing than a plain `A › B › C` line of text.
fn draw_category_boxes(frame: &mut Frame, area: Rect, names: &[&str]) {
    if area.height < 3 {
        return;
    }
    let mut x = area.x;
    for (i, name) in names.iter().enumerate() {
        let width = name.chars().count() as u16 + 4; // borders + a space of padding either side
        if x + width > area.x + area.width {
            break; // no room for another box; the rest just don't show
        }
        let box_area = Rect { x, y: area.y, width, height: 3 };
        let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(Style::default().fg(theme().accent));
        let inner = block.inner(box_area);
        frame.render_widget(block, box_area);
        frame.render_widget(Paragraph::new(Span::styled(*name, Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD))).alignment(Alignment::Center), inner);
        x += width;
        if i + 1 < names.len() && x + 1 < area.x + area.width {
            let arrow_area = Rect { x, y: area.y + 1, width: 1, height: 1 };
            frame.render_widget(Paragraph::new(Span::styled("›", Style::default().fg(theme().muted))), arrow_area);
        }
        x += 1;
    }
}

/// The popup for changing an action's keys: what to do with them, then the
/// key itself, pressed and confirmed.
fn draw_key_capture(frame: &mut Frame, capture: &CaptureView) {
    let muted = Style::default().fg(theme().muted);
    let strong = Style::default().fg(theme().text_strong);
    let key_style = Style::default().fg(theme().key).add_modifier(Modifier::BOLD);
    let choice = |key: &str, what: &str| Line::from(vec![Span::styled(format!("{key:<10}"), key_style), Span::styled(what.to_string(), muted)]);
    let mut lines: Vec<Line> = Vec::new();
    match &capture.stage {
        CaptureStage::Menu => {
            lines.push(Line::styled("Keys now", strong));
            for (i, key) in capture.keys.iter().enumerate() {
                lines.push(Line::from(vec![Span::styled(format!("  {}  ", i + 1), muted), Span::styled(key.clone(), key_style)]));
            }
            lines.push(Line::raw(""));
            lines.push(choice("a", "add another key"));
            lines.push(choice("r", "replace them all with one key"));
            lines.push(choice("1-9", "remove that key"));
            lines.push(choice("d", "go back to the default keys"));
            lines.push(choice("esc", "close"));
        }
        CaptureStage::Waiting => {
            lines.push(Line::styled("Press the key you want to use.", strong));
            lines.push(Line::styled("Any key works, Enter and Esc included. Nothing is saved until you confirm.", muted));
        }
        CaptureStage::Confirm { key, replace } => {
            lines.push(Line::from(vec![Span::styled("You pressed  ", strong), Span::styled(format!("<{key}>"), key_style)]));
            lines.push(Line::raw(""));
            lines.push(choice("enter", if *replace { "use it instead of the current keys" } else { "add it to the current keys" }));
            lines.push(choice("backspace", "pick a different key"));
            lines.push(choice("esc", "cancel"));
        }
    }
    if let Some(problem) = &capture.problem {
        lines.push(Line::raw(""));
        lines.push(Line::styled(problem.clone(), Style::default().fg(theme().bad)));
    }
    let height = (lines.len() as u16 + 4).min(frame.area().height);
    let area = centered_rect(60, 100, frame.area());
    let area = Rect { y: frame.area().y + frame.area().height.saturating_sub(height) / 2, height, ..area };
    // Everything behind recedes and the popup gets a bright border.
    let full = frame.area();
    frame.buffer_mut().set_style(full, Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(Style::default().fg(theme().accent))
        .title(pill_title(&capture.label, false, Style::default().fg(theme().accent)));
    let inner = block.inner(area).inner(ratatui::layout::Margin { horizontal: 2, vertical: 1 });
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Which settings row a click lands on.
pub fn settings_row_at(frame_area: Rect, len: usize, offset: usize, row: u16) -> Option<usize> {
    let area = body_area(frame_area, true);
    let top = area.y + 1;
    let bottom = area.y + area.height.saturating_sub(1);
    if row < top || row >= bottom {
        return None;
    }
    let index = offset + usize::from(row - top);
    (index < len).then_some(index)
}
