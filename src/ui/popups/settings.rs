//! The settings screen and key capture.

use super::*;

/// Everything `draw_settings` reads but doesn't own, gathered so the
/// function takes one argument instead of a fistful.
pub(in crate::ui) struct SettingsView<'a> {
    pub tab: SettingsTab,
    pub rows: &'a [SettingView],
    pub layout: &'a [LayoutRow],
    pub extensions: &'a [ExtensionRow],
    pub error: Option<&'a str>,
    pub capture: Option<&'a CaptureView>,
}

/// The settings screen: one row per setting under its section, the value in
/// bold when the config file sets it, and a swatch for colours.
pub(in crate::ui) fn draw_settings(frame: &mut Frame, view: SettingsView, state: &mut TableState) {
    let SettingsView { tab, rows, layout, extensions, error, capture } = view;
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let bottom = match error {
        Some(e) => Line::styled(format!(" {e} "), Style::default().fg(theme().bad)),
        None if tab == SettingsTab::Overview => hint_strip(&[("J/K", "move"), ("space", "show/hide"), ("r", "reset"), ("tab", "next"), ("esc", "close")]).right_aligned(),
        None if tab == SettingsTab::Extensions => hint_strip(&[("space/enter", "on/off"), ("tab", "next"), ("esc", "close")]).right_aligned(),
        None => hint_strip(&[("←→", "change"), ("enter", "edit"), ("r", "reset"), ("tab", "next"), ("esc", "close")]).right_aligned(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(settings_tabs(tab).centered())
        .title_bottom(bottom);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // The selected setting's explanation takes the last two lines.
    let help = if tab == SettingsTab::Overview {
        "Give each category its place from the left: press a number to put the selected one there, or K and J to nudge it. Space hides or shows it."
    } else if tab == SettingsTab::Extensions {
        "Space or Enter toggles the selected one, off by default. ACTIVE is found/total: how many of its resource kinds this cluster has installed."
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
    if tab == SettingsTab::Extensions {
        draw_extension_rows(frame, inner, extensions, state);
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
    let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).split(area);
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
    let preview = Paragraph::new(vec![Line::raw(""), Line::from(vec![Span::styled("Left to right:  ", Style::default().fg(theme().muted)), Span::styled(shown.join("  ›  "), Style::default().fg(theme().accent))])]).wrap(Wrap { trim: true });
    frame.render_widget(preview, parts[1]);
}

/// One extension's row: status, name, the "active" column (whether it's
/// actually found anything on this cluster, separate from on/off), and its
/// static description — an error takes over the description spot instead.
fn extension_row(row: &ExtensionRow) -> Row<'static> {
    let (status, status_style) = match (&row.error, row.enabled) {
        (Some(_), _) => ("error".to_string(), Style::default().fg(theme().bad)),
        (None, true) => ("on".to_string(), Style::default().fg(theme().ok).add_modifier(Modifier::BOLD)),
        (None, false) => ("off".to_string(), Style::default().fg(theme().muted)),
    };
    let name_style = if row.error.is_some() {
        Style::default().fg(theme().muted)
    } else if row.enabled {
        Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme().desc)
    };
    // Its own column, not folded into the description: on/off asks "did you turn
    // this on", active asks "is there anything for it to do on THIS cluster" —
    // two different questions, so a glance at one doesn't answer the other.
    let (active, active_style) = match row.found {
        Some((0, total)) => (format!("0/{total}"), Style::default().fg(theme().muted)),
        Some((found, total)) => (format!("{found}/{total}"), Style::default().fg(theme().ok)),
        None => ("—".to_string(), Style::default().fg(theme().muted)),
    };
    let detail = row.error.clone().unwrap_or_else(|| row.description.clone());
    Row::new(vec![
        Cell::from(Span::styled(status, status_style)),
        Cell::from(Span::styled(row.name.clone(), name_style)),
        Cell::from(Span::styled(active, active_style)),
        Cell::from(Span::styled(detail, Style::default().fg(theme().muted))),
    ])
}

fn section_heading(text: &str) -> Row<'static> {
    Row::new(vec![Cell::new(Span::styled(text.to_string(), Style::default().fg(theme().heading).add_modifier(Modifier::BOLD))).column_span(4)])
}

fn placeholder_row(text: &str) -> Row<'static> {
    Row::new(vec![Cell::new(Span::styled(text.to_string(), Style::default().fg(theme().muted).add_modifier(Modifier::ITALIC))).column_span(4)])
}

fn blank_row() -> Row<'static> {
    Row::new(vec![Cell::from("")])
}

/// The render position (a row index into what `draw_extension_rows` actually
/// builds, heading and spacer rows included) of logical extension index `i`.
/// The layout is always "Bundled" heading, bundled rows, a blank spacer,
/// "External" heading, external rows (or a `<none>` placeholder when there
/// aren't any) — the same fixed shape regardless of what's loaded, so there's
/// no branch here to keep in sync with the one in `extension_at_render`.
fn extension_render_index(i: usize, bundled_count: usize) -> usize {
    if i < bundled_count { i + 1 } else { i + 3 }
}

/// The reverse of `extension_render_index`: which logical extension (if any)
/// sits at render position `r` — `None` on a heading row, the spacer row, the
/// placeholder row, or past the end.
fn extension_at_render(r: usize, bundled_count: usize, total: usize) -> Option<usize> {
    if r == 0 {
        return None; // "Bundled" heading
    }
    if r <= bundled_count {
        return Some(r - 1);
    }
    if r <= bundled_count + 2 {
        return None; // blank spacer, then "External" heading
    }
    let logical = bundled_count + (r - bundled_count - 3);
    (logical < total).then_some(logical)
}

/// The Extensions tab: bundled ones under their own heading, then anything
/// added from `~/.config/knav/extensions/` under an "External" heading of its
/// own — the two are a different kind of thing (shipped with knav vs.
/// someone's own manifest), worth keeping visually apart rather than one flat
/// list. The External heading always shows, even with nothing under it yet,
/// so it reads as "supported, empty" rather than "doesn't exist" — same
/// reasoning as showing an enabled extension's category with 0 found rather
/// than hiding it. A bad manifest shows its error instead of the on/off
/// toggle doing anything.
fn draw_extension_rows(frame: &mut Frame, area: Rect, extensions: &[ExtensionRow], state: &mut TableState) {
    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(extensions.len().saturating_sub(1))));
    }
    // `Registry::load` always lists bundled ones first, so finding where they
    // stop is the whole split, not a real sort.
    let bundled_count = extensions.iter().take_while(|e| e.bundled).count();

    let mut rows: Vec<Row> = vec![section_heading("Bundled")];
    rows.extend(extensions[..bundled_count].iter().map(extension_row));
    rows.push(blank_row());
    rows.push(section_heading("External"));
    if bundled_count < extensions.len() {
        rows.extend(extensions[bundled_count..].iter().map(extension_row));
    } else {
        rows.push(placeholder_row("<none> — add one with `knav ext add <repo>`"));
    }

    // The heading/placeholder rows aren't selectable, so the selection needs
    // remapping from logical (an index into `extensions`, what the handler
    // indexes directly) to render position (a row in `rows` above, heading
    // rows included) — the same remap `extension_row_at` uses for mouse
    // clicks, or clicking a row and landing on the one above or below it
    // would disagree. The offset lives in render terms the whole time
    // instead: nothing outside this tab reads it, so there's no logical
    // meaning it needs to hold, and the widget already knows how to shift it
    // to keep the selection in view.
    let logical = state.selected().unwrap_or(0);
    let mut render_state = TableState::default().with_selected(Some(extension_render_index(logical, bundled_count))).with_offset(state.offset());

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled("EXTENSION", Style::default().fg(theme().muted))),
        Cell::from(Span::styled("ACTIVE", Style::default().fg(theme().muted))),
        Cell::from(""),
    ]);
    let table = Table::new(rows, [Constraint::Length(6), Constraint::Length(16), Constraint::Length(7), Constraint::Min(10)])
        .column_spacing(2)
        .style(theme_row(false))
        .header(header)
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, false));
    frame.render_stateful_widget(table, area, &mut render_state);
    state.select(Some(logical));
    *state.offset_mut() = render_state.offset();
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

/// Which extension a click on the Extensions tab lands on: `settings_row_at`'s
/// plain offset math, then translated through the same heading-aware remap
/// `draw_extension_rows` uses, so a click can't land on the wrong row just
/// because a "Bundled"/"External" heading sits somewhere above it on screen.
/// `bundled_count`/`total` describe the loaded extensions the same way
/// `draw_extension_rows` derives them, just handed in rather than a typed
/// slice, so this works whichever of the two extension list types the caller
/// has on hand. `offset` is already in render terms (see `draw_extension_rows`),
/// the same value `state.offset()` holds for this tab.
pub fn extension_row_at(frame_area: Rect, bundled_count: usize, total: usize, offset: usize, row: u16) -> Option<usize> {
    let area = body_area(frame_area, true);
    let top = area.y + 2; // top border + column header
    let bottom = area.y + area.height.saturating_sub(1);
    if row < top || row >= bottom {
        return None;
    }
    let render_row = offset + usize::from(row - top);
    extension_at_render(render_row, bundled_count, total)
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

#[cfg(test)]
mod extension_row_tests {
    use super::*;

    #[test]
    fn every_logical_index_round_trips_through_its_render_position() {
        // 3 bundled, 2 external: headings at render 0 ("Bundled") and render 5 ("External"),
        // with a blank spacer at render 4.
        let (bundled_count, total) = (3, 5);
        for logical in 0..total {
            let r = extension_render_index(logical, bundled_count);
            assert_eq!(extension_at_render(r, bundled_count, total), Some(logical), "logical {logical} -> render {r} didn't round-trip");
        }
    }

    #[test]
    fn heading_and_spacer_rows_belong_to_nobody() {
        let (bundled_count, total) = (3, 5);
        assert_eq!(extension_at_render(0, bundled_count, total), None, "the \"Bundled\" heading");
        assert_eq!(extension_at_render(4, bundled_count, total), None, "the blank spacer");
        assert_eq!(extension_at_render(5, bundled_count, total), None, "the \"External\" heading");
        assert_eq!(extension_at_render(99, bundled_count, total), None, "past the end");
    }

    #[test]
    fn with_nothing_external_the_placeholder_row_belongs_to_nobody() {
        let (bundled_count, total) = (3, 3);
        assert_eq!(extension_at_render(0, bundled_count, total), None, "the \"Bundled\" heading");
        assert_eq!(extension_at_render(1, bundled_count, total), Some(0));
        assert_eq!(extension_at_render(3, bundled_count, total), Some(2));
        assert_eq!(extension_at_render(4, bundled_count, total), None, "the blank spacer");
        assert_eq!(extension_at_render(5, bundled_count, total), None, "the \"External\" heading");
        assert_eq!(extension_at_render(6, bundled_count, total), None, "the <none> placeholder row");
    }
}
