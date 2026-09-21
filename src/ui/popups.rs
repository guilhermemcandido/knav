//! Modal popups: search/command/context/notice bars, node detail, events, containers, resources.

use super::*;

/// The command line's calm steel blue (its border and prompt).

/// Rows each suggestion takes: room for a 6x3 icon beside its name.
const SUGGESTION_HEIGHT: u16 = 3;
const SUGGESTION_ICON: Rect = Rect { x: 0, y: 0, width: 6, height: SUGGESTION_HEIGHT };

/// The `:` command line, k9s-style: a bar under the header with live autocomplete
/// hanging off it, each match with its icon. The best match's remaining letters
/// show dimmed after the cursor.
pub(super) fn draw_command_line(frame: &mut Frame, bar: Rect, input: &str, suggestions: &[SuggestionView], selected: usize, icons: &mut IconCache) {
    frame.render_widget(Clear, bar);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(Style::default().fg(theme().command));
    let inner = block.inner(bar);
    frame.render_widget(block, bar);

    // "namespaces (ns)": complete against the name, not the alias note.
    let ghost = suggestions
        .get(selected)
        .and_then(|s| s.label.split(" (").next())
        .and_then(|name| name.strip_prefix(input))
        .unwrap_or("");
    let line = Line::from(vec![
        Span::styled("> ", Style::default().fg(theme().command)),
        Span::styled(input.to_string(), Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD)),
        Span::styled("▏", Style::default().fg(theme().command)),
        Span::styled(ghost.to_string(), Style::default().fg(theme().muted)),
    ]);
    frame.render_widget(Paragraph::new(line), inner);

    if suggestions.is_empty() {
        return;
    }
    // As many rows as the screen has room for, scrolled to keep the selection in view.
    let below = frame.area().bottom().saturating_sub(bar.bottom());
    let shown = suggestions.len().min(usize::from(below.saturating_sub(2) / SUGGESTION_HEIGHT));
    if shown == 0 {
        return;
    }
    let start = (selected + 1).saturating_sub(shown).min(suggestions.len() - shown);
    let width = (suggestions.iter().map(|s| s.label.chars().count()).max().unwrap_or(0) as u16 + SUGGESTION_ICON.width + 8).max(40).min(bar.width);
    let list = Rect { x: bar.x, y: bar.bottom(), width, height: shown as u16 * SUGGESTION_HEIGHT + 2 };
    frame.render_widget(Clear, list);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(Style::default().fg(theme().command));
    let inner = block.inner(list);
    frame.render_widget(block, list);
    for (n, suggestion) in suggestions.iter().enumerate().skip(start).take(shown) {
        let row = Rect { x: inner.x, y: inner.y + (n - start) as u16 * SUGGESTION_HEIGHT, width: inner.width, height: SUGGESTION_HEIGHT };
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

/// The `:ctx` / `C` context browser, same full-size table as the
/// Events browser (and the same geometry, so `event_row_at` hit-tests
/// its rows too). `/` live-filters by name.
pub(super) fn draw_context_popup(
    frame: &mut Frame,
    items: &[(String, String, bool)],
    total: usize,
    filter: &str,
    editing: bool,
    state: &mut TableState,
    error: Option<&str>,
    sort: SortState,
) {
    let area = centered_rect(94, 88, frame.area());
    frame.render_widget(Clear, area);

    const HEADERS: [&str; 3] = ["CONTEXT", "CLUSTER", "STATUS"];
    let window = layout_table(
        &HEADERS,
        items.iter().map(|(name, cluster, _)| vec![cell_width(name), cell_width(cluster), cell_width("current")]),
        area.width.saturating_sub(2),
        None,
        &mut 0,
    );
    let header = header_row(&HEADERS, sort, false, &window);
    let rows = items.iter().map(|(name, cluster, current)| {
        Row::new(window.slice(vec![
            Cell::from(highlight_fuzzy(name, filter, Style::default().add_modifier(Modifier::BOLD))),
            Cell::from(highlight_fuzzy(cluster, filter, Style::default())),
            Cell::from(if *current { "current" } else { "" }).style(Style::default().fg(theme().ok)),
        ]))
    });

    let mut title = colored_slash_title(&format!("Contexts ({}/{total})", items.len()));
    if let Some(err) = error {
        title.push_span(Span::styled(format!("  —  {err}"), Style::default().fg(theme().bad)));
    }

    let table = Table::new(mark_rows(rows, &[], false), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(false))
        .header(header)
        .block(with_search(Block::default().borders(Borders::ALL).border_set(border_set()).title(title), filter, editing, false))
        .highlight_symbol("")
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, false));

    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(items.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, state);
}

/// The chip strip along the namespace picker's bottom border: a label, then
/// `1`..`9`, each three cells wide, one cell apart.
const CHIP_LABEL: &str = " assign to key: ";

fn chips_origin(picker_area: Rect) -> u16 {
    picker_area.x + 1 + CHIP_LABEL.chars().count() as u16
}

/// Which number chip (1-9) a click on the namespace picker's bottom border
/// lands on.
pub fn slot_chip_at(frame_area: Rect, column: u16, row: u16) -> Option<usize> {
    let area = centered_rect(94, 88, frame_area);
    if row != area.y + area.height.saturating_sub(1) || column < chips_origin(area) {
        return None;
    }
    let offset = column - chips_origin(area);
    let (chip, within) = (usize::from(offset / 4), offset % 4);
    (chip < 9 && within < 3).then_some(chip + 1)
}

pub(super) fn draw_namespace_picker(
    frame: &mut Frame,
    items: &[(String, Option<usize>)],
    total: usize,
    filter: &str,
    editing: bool,
    state: &mut TableState,
    sort: SortState,
) {
    let area = centered_rect(94, 88, frame.area());
    frame.render_widget(Clear, area);

    const HEADERS: [&str; 2] = ["NAMESPACE", "KEY"];
    let window = layout_table(&HEADERS, items.iter().map(|(name, _)| vec![cell_width(name), 1]), area.width.saturating_sub(2), None, &mut 0);
    let header = header_row(&HEADERS, sort, false, &window);
    let rows = items.iter().map(|(name, key)| {
        Row::new(window.slice(vec![
            Cell::from(highlight_fuzzy(name, filter, Style::default().add_modifier(Modifier::BOLD))),
            Cell::from(key.map(|k| k.to_string()).unwrap_or_default()).style(Style::default().fg(theme().warm)),
        ]))
    });

    let title = colored_slash_title(&format!("Choose the namespace to filter by ({}/{total})", items.len()));

    // The number chips: each key, lit when the highlighted namespace has it,
    // orange when another namespace does.
    let selected_key = state.selected().and_then(|i| items.get(i)).and_then(|(_, key)| *key);
    let mut chips = vec![Span::styled(CHIP_LABEL, Style::default().fg(theme().muted))];
    for key in 1..=9usize {
        let taken = items.iter().any(|(_, k)| *k == Some(key));
        let style = if selected_key == Some(key) {
            Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD)
        } else if taken {
            Style::default().fg(theme().warm).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme().text_soft)
        };
        chips.push(Span::styled(format!(" {key} "), style));
        chips.push(Span::raw(" "));
    }

    let table = Table::new(mark_rows(rows, &[], false), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(false))
        .header(header)
        .block(with_search(Block::default().borders(Borders::ALL).border_set(border_set()).title(title).title_bottom(Line::from(chips)), filter, editing, false))
        .highlight_symbol("")
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, false));

    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(items.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, state);
}

/// The key picker: `0` (always "all", not assignable) and keys 1-9 with
/// what each holds; the highlighted key is where Enter puts the namespace.
pub(super) fn draw_slots_popup(frame: &mut Frame, namespace: &str, slots: &[Option<String>], selected: usize) {
    let bar = centered_box(frame.area(), 2 + 1 + 9 + 1 + 1);
    frame.render_widget(Clear, bar);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .title(format!(" Choose the key for '{namespace}' "));
    let inner = block.inner(bar);
    frame.render_widget(block, bar);

    let rows = Layout::vertical([Constraint::Length(1)].repeat(inner.height.max(1) as usize)).split(inner);
    let key_style = Style::default().fg(theme().warm);
    let fixed = Style::default().fg(theme().muted);

    if let Some(row) = rows.first() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled("<0> ", fixed), Span::styled("all", fixed)])),
            *row,
        );
    }
    for (i, slot) in slots.iter().enumerate() {
        let Some(row) = rows.get(i + 1) else { break };
        let text = match slot.as_deref() {
            Some(ns) => ns.to_string(),
            None => "—".to_string(),
        };
        let line = if i == selected {
            let style = Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD);
            Line::styled(format!("{:<width$}", format!("<{}> {text}", i + 1), width = row.width as usize), style)
        } else {
            Line::from(vec![Span::styled(format!("<{}> ", i + 1), key_style), Span::raw(text)])
        };
        frame.render_widget(Paragraph::new(line), *row);
    }
    if let Some(row) = rows.get(slots.len() + 2) {
        frame.render_widget(
            Paragraph::new(Line::styled("1-9 assign  d clear", Style::default().fg(theme().muted))),
            *row,
        );
    }
}

/// A small centered message box, green-bordered for success, red for
/// an error. Sized to the text so a one-liner doesn't get a huge box.
pub(super) fn draw_notice_popup(frame: &mut Frame, text: &str, error: bool) {
    let full = frame.area();
    let width = (full.width * 3 / 5).max(30).min(full.width);
    let inner_w = width.saturating_sub(2).max(1) as usize;
    let lines: usize = text.lines().map(|l| l.chars().count().div_ceil(inner_w).max(1)).sum::<usize>().max(1);
    let height = (lines as u16 + 2).min(full.height);
    let area = Rect {
        x: full.x + full.width.saturating_sub(width) / 2,
        y: full.y + full.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, area);
    let color = if error { theme().bad } else { theme().ok };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(Style::default().fg(color))
        .title(if error { "Failed" } else { "Done" });
    frame.render_widget(Paragraph::new(text.to_string()).wrap(Wrap { trim: false }).block(block), area);
}

/// A small centred box with a title and body lines, for the question popups.
fn small_popup(frame: &mut Frame, title: &str, color: Color, body: Vec<Line<'static>>) {
    let full = frame.area();
    let width = (full.width * 3 / 5).max(30).min(full.width);
    let height = (body.len() as u16 + 2).min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 2, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(Style::default().fg(color)).title(title.to_string());
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }).block(block), area);
}

pub(super) fn draw_confirm_popup(frame: &mut Frame, text: &str) {
    let key = Style::default().fg(theme().highlight).add_modifier(Modifier::BOLD);
    let body = vec![
        Line::from(text.to_string()),
        Line::from(vec![Span::styled("y", key), Span::raw(" yes   "), Span::styled("n", key), Span::raw(" no")]),
    ];
    small_popup(frame, "Confirm", theme().highlight, body);
}

/// The port-forward dialog, laid out like k9s's: labelled fields, a warning
/// when the port is a guess, and OK / Cancel.
pub(super) fn draw_port_forward_popup(frame: &mut Frame, title: &str, form: &crate::ops::portforward::PortForm) {
    use crate::ops::portforward::Field;
    let full = frame.area();
    let width = (full.width * 3 / 5).clamp(44, full.width.max(1)).min(full.width);
    let height = 11u16.min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 3, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(Line::styled("<PortForward>", Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)).centered());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label = Style::default().fg(theme().label);
    let value = Style::default().fg(theme().text_strong);
    let hint = Style::default().fg(theme().muted);
    let field = |name: &str, text: &str, placeholder: &str, focused: bool| {
        let shown = if text.is_empty() && !focused { Span::styled(placeholder.to_string(), hint) } else { Span::styled(format!("{text}{}", if focused { "▏" } else { "" }), value) };
        Line::from(vec![Span::styled(format!(" {name:<16}"), label), shown])
    };
    let button = |name: &str, focused: bool| {
        let style = if focused { Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD) } else { Style::default().fg(value.fg.unwrap_or(theme().text_strong)) };
        Span::styled(format!(" {name} "), style)
    };
    let mut lines = vec![
        Line::styled(title.to_string(), value.add_modifier(Modifier::BOLD)).centered(),
        Line::raw(""),
        field("Container Port:", &form.container, "Enter the container port", form.focus == Field::Container),
        field("Local Port:", &form.local, "Enter a local port", form.focus == Field::Local),
        field("Address:", &form.address, "localhost", form.focus == Field::Address),
        Line::raw(""),
    ];
    let note = match (&form.error, form.warning()) {
        (Some(e), _) => Some(Line::styled(format!(" {e}"), Style::default().fg(theme().bad))),
        (None, Some(w)) => Some(Line::styled(format!(" ⚠ {w}"), Style::default().fg(theme().warn))),
        _ => None,
    };
    lines.push(note.unwrap_or_else(|| Line::raw("")));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![button("OK", form.focus == Field::Ok), Span::raw("   "), button("Cancel", form.focus == Field::Cancel)]).centered());
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The theme list: each theme with a strip of its colours. The screen behind
/// is drawn in the theme being previewed, so the whole interface is the sample.
pub(super) fn draw_theme_picker(frame: &mut Frame, entries: &[crate::app::mode::ThemeEntry], state: &mut TableState, saved: &str) {
    let area = centered_rect(64, 86, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(Line::styled(format!(" Themes ({}) ", entries.len()), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)))
        .title_bottom(Line::styled(" enter keeps  ·  esc cancels ", Style::default().fg(theme().muted)).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // The first two colours are the theme's background and text, which the live
    // preview already shows; the swatch is the accents. The selected row is
    // shaded through the row style so its dots keep their own colours.
    let selected = state.selected();
    let rows: Vec<Row> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let swatch: Vec<Span> = entry.swatch.iter().skip(2).map(|color| Span::styled("● ", Style::default().fg(*color))).collect();
            let mark = if entry.name == saved { "✔ in use" } else { "" };
            let row = Row::new(vec![
                Cell::from(Span::styled(entry.name.clone(), Style::default().add_modifier(Modifier::BOLD))),
                Cell::from(Line::from(swatch)),
                Cell::from(Span::styled(mark, Style::default().fg(theme().ok))),
            ]);
            if selected == Some(i) { row.style(selection_style(crate::k8s::describe::Tone::Plain, false)) } else { row }
        })
        .collect();
    let table = Table::new(rows, [Constraint::Length(20), Constraint::Length(16), Constraint::Min(8)])
        .column_spacing(2)
        .style(theme_row(false));
    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(entries.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, inner, state);
}

/// The relations diagram in a full-size frame.
pub(super) fn draw_relations(frame: &mut Frame, title: &str, graph: &crate::k8s::relations::Graph, selected: usize) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(Line::styled(format!(" Related to {title} "), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)).centered())
        .title_bottom(Line::styled(" ←↑↓→ move   enter open   space follow   backspace back   esc close ", Style::default().fg(theme().muted)).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if graph.nodes.len() <= 1 {
        frame.render_widget(Paragraph::new("Nothing else is related to this object.").style(Style::default().fg(theme().muted)).alignment(Alignment::Center), inner);
        return;
    }
    super::graph::draw_graph(frame, inner, graph, selected);
}

/// The settings screen: one row per setting under its section, the value in
/// bold when the config file sets it, and a swatch for colours.
pub(super) fn draw_settings(frame: &mut Frame, tab: SettingsTab, rows: &[SettingView], layout: &[LayoutRow], state: &mut TableState, error: Option<&str>, capture: Option<&CaptureView>) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let bottom = match error {
        Some(e) => Line::styled(format!(" {e} "), Style::default().fg(theme().bad)),
        None if tab == SettingsTab::Overview => Line::styled(" J/K move   space show/hide   r reset   tab next   esc close ", Style::default().fg(theme().muted)).right_aligned(),
        None => Line::styled(" ←→ change   enter edit   r reset   tab next   esc close ", Style::default().fg(theme().muted)).right_aligned(),
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
        .title(Line::styled(format!(" {} ", capture.label), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)).centered());
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

/// Which theme row a click lands on.
pub fn theme_row_at(frame_area: Rect, len: usize, offset: usize, row: u16) -> Option<usize> {
    let area = centered_rect(64, 86, frame_area);
    let top = area.y + 1; // top border
    let bottom = area.y + area.height.saturating_sub(1);
    if row < top || row >= bottom {
        return None;
    }
    let index = offset + usize::from(row - top);
    (index < len).then_some(index)
}

/// Where an embedded shell's screen goes: the body of the page, inside its border.
pub fn shell_inner(frame_area: Rect) -> Rect {
    let area = body_area(frame_area, true);
    Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) }
}

fn shell_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// An emulated terminal screen over the whole body, cell by cell.
pub(super) fn draw_shell_popup(frame: &mut Frame, title: &str, screen: &vt100::Screen, exited: bool) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let bottom = if exited { " the shell ended; press any key " } else { " ctrl-] closes " };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(Line::styled(format!(" Shell {title} "), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)).centered())
        .title_bottom(Line::styled(bottom, Style::default().fg(if exited { theme().warn } else { theme().muted })).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let buffer = frame.buffer_mut();
    for row in 0..inner.height {
        for column in 0..inner.width {
            let Some(cell) = screen.cell(row, column) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut style = Style::default().fg(shell_color(cell.fgcolor())).bg(shell_color(cell.bgcolor()));
            for (on, modifier) in [(cell.bold(), Modifier::BOLD), (cell.italic(), Modifier::ITALIC), (cell.underline(), Modifier::UNDERLINED), (cell.inverse(), Modifier::REVERSED)] {
                if on {
                    style = style.add_modifier(modifier);
                }
            }
            let contents = cell.contents();
            buffer[(inner.x + column, inner.y + row)].set_symbol(if contents.is_empty() { " " } else { &contents }).set_style(style);
        }
    }
    if !exited && !screen.hide_cursor() {
        let (row, column) = screen.cursor_position();
        if row < inner.height && column < inner.width {
            frame.set_cursor_position((inner.x + column, inner.y + row));
        }
    }
}

/// One YAML line coloured by role: keys blue, the rest plain, list dashes muted.
fn yaml_line(line: &str) -> Line<'static> {
    let indent = line.len() - line.trim_start().len();
    let (lead, rest) = line.split_at(indent);
    let (dash, rest) = match rest.strip_prefix("- ") {
        Some(after) => ("- ", after),
        None => ("", rest),
    };
    let key_style = Style::default().fg(theme().key);
    let plain = Style::default().fg(theme().text_soft);
    let mut spans = vec![Span::raw(lead.to_string()), Span::styled(dash.to_string(), Style::default().fg(theme().muted))];
    match rest.split_once(": ").or_else(|| rest.strip_suffix(':').map(|k| (k, ""))) {
        Some((key, value)) if !key.contains(' ') || key.starts_with('"') => {
            spans.push(Span::styled(key.to_string(), key_style));
            spans.push(Span::styled(":", Style::default().fg(theme().muted)));
            if !value.is_empty() {
                spans.push(Span::styled(format!(" {value}"), plain));
            }
        }
        _ => spans.push(Span::styled(rest.to_string(), plain)),
    }
    Line::from(spans)
}

/// A manifest as scrollable text over the whole body of the screen.
pub(super) fn draw_yaml_popup(frame: &mut Frame, title: &str, text: &str, scroll: usize) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(Line::styled(format!(" {title} "), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)).centered());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines: Vec<Line> = text.lines().skip(scroll).take(usize::from(inner.height)).map(yaml_line).collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_prompt_popup(frame: &mut Frame, title: &str, value: &str, hint: &str) {
    let mut body = vec![Line::from(vec![Span::raw("> "), Span::styled(format!("{value}▏"), Style::default().fg(theme().highlight))])];
    if !hint.is_empty() {
        body.push(Line::styled(hint.to_string(), Style::default().fg(theme().muted)));
    }
    small_popup(frame, title, theme().namespace, body);
}

/// Node drill-down: the node's gauges (`draw_gauge`) above its pods, using the
/// same pod table and container dots as the Pods list.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_node_detail_popup(
    frame: &mut Frame,
    name: &str,
    cpu_usage: Option<i64>,
    cpu_capacity: i64,
    memory_usage: Option<i64>,
    memory_capacity: i64,
    pod_capacity: i64,
    info: Option<&crate::k8s::NodeDetailInfo>,
    pods: &[PodRow],
    state: &mut TableState,
    sort: SortState,
    search: Search,
    dimmed: bool,
) {
    let area = centered_rect(94, 92, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(border_style)
        .title(format!("Node: {name}"));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let info_h = info.map(node_info_height).unwrap_or(0);
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Length(info_h), Constraint::Min(0)]).split(inner);

    match (cpu_usage, memory_usage) {
        (Some(cpu), Some(mem)) => {
            let lines = Layout::vertical([Constraint::Length(1); 3]).split(chunks[0]);
            draw_meter(frame, lines[0], "CPU", cpu as f64, cpu_capacity as f64, |v| format!("{:.2} cores", v / 1000.0), dimmed);
            draw_meter(frame, lines[1], "Memory", mem as f64, memory_capacity as f64, format_bytes, dimmed);
            draw_meter(frame, lines[2], "Pods", pods.len() as f64, pod_capacity as f64, |v| format!("{v:.0}"), dimmed);
        }
        _ => {
            let text = Paragraph::new(Line::styled("metrics unavailable", Style::default().fg(theme().muted).add_modifier(Modifier::BOLD)))
                .alignment(Alignment::Center);
            frame.render_widget(text, chunks[0]);
        }
    }

    if let Some(info) = info {
        draw_node_info_panel(frame, chunks[1], info, dimmed);
    }

    draw_table(frame, chunks[2], pods, state, if dimmed { Search::default() } else { search }, if dimmed { SortState::default() } else { sort }, &mut 0, &HashSet::new(), false, dimmed);
}

/// How tall the node-info panel is: three summary lines, a blank, the conditions
/// header and rows, then a blank and a taints line if any. Sizing and drawing share it.
pub(super) fn node_info_height(info: &crate::k8s::NodeDetailInfo) -> u16 {
    let base = 3 + 1 + 1 + info.conditions.len() as u16;
    if info.taints.is_empty() { base } else { base + 1 + info.taints.len() as u16 }
}

/// Node summary: schedulability, roles, version, addresses, host details, the full
/// condition list (healthy ones too) and any taints.
pub(super) fn draw_node_info_panel(frame: &mut Frame, area: Rect, info: &crate::k8s::NodeDetailInfo, dimmed: bool) {
    let label = Style::default().fg(theme().muted);
    let value = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let field = |l: &'static str, v: String| vec![Span::styled(format!("{l}: "), label), Span::styled(v, value)];

    let schedulable_text = if info.schedulable { "Schedulable".to_string() } else { "Cordoned".to_string() };
    let schedulable_style = if dimmed {
        dim_style()
    } else if info.schedulable {
        Style::default().fg(theme().ok)
    } else {
        Style::default().fg(theme().highlight)
    };

    let mut line1 = field("Roles", info.roles.clone());
    line1.push(Span::raw("   "));
    line1.extend(vec![Span::styled("Status: ", label), Span::styled(schedulable_text, schedulable_style)]);
    line1.push(Span::raw("   "));
    line1.extend(field("Kubelet", info.kubelet_version.clone()));

    let mut line2 = field("Internal IP", info.internal_ip.clone());
    line2.push(Span::raw("   "));
    line2.extend(field("External IP", info.external_ip.clone()));

    let mut line3 = field("OS", info.os_image.clone());
    line3.push(Span::raw("   "));
    line3.extend(field("Kernel", info.kernel_version.clone()));
    line3.push(Span::raw("   "));
    line3.extend(field("Runtime", info.container_runtime.clone()));

    let mut lines = vec![Line::from(line1), Line::from(line2), Line::from(line3), Line::raw("")];

    lines.push(Line::styled("CONDITIONS", value));
    for c in &info.conditions {
        let is_healthy = (c.type_ == "Ready") == (c.status == "True");
        let color = if dimmed {
            theme().dim
        } else if is_healthy {
            theme().ok
        } else {
            theme().bad
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:<20}", c.type_), if dimmed { dim_style() } else { Style::default() }),
            Span::styled(format!("{:<8}", c.status), Style::default().fg(color)),
            Span::styled(c.reason.clone(), label),
        ]));
    }

    if !info.taints.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled("Taints: ", label), Span::styled(info.taints.join(", "), value)]));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// The full Events browser: every event, uncapped, filterable by severity
/// (`Normal` and `Warning` are all Kubernetes defines).
pub(super) fn draw_events_popup(
    frame: &mut Frame,
    events: &[EventEntry],
    filter: EventFilter,
    search: &str,
    editing: bool,
    state: &mut TableState,
    sort: SortState,
    dimmed: bool,
) {
    let area = centered_rect(94, 88, frame.area());
    frame.render_widget(Clear, area);

    let filtered = crate::k8s::filter_events(events, filter, search, sort.spec());

    const HEADERS: [&str; 6] = ["TYPE", "REASON", "OBJECT", "KIND", "MESSAGE", "AGE"];
    // MESSAGE is free text: it takes whatever room the other columns leave.
    let window = layout_table(
        &HEADERS,
        filtered.iter().map(|e| vec![cell_width("Warning"), cell_width(&e.reason), cell_width(&e.object), cell_width(&e.kind), cell_width(&e.message), cell_width(&e.age)]),
        area.width.saturating_sub(2),
        Some(4),
        &mut 0,
    );
    let header = header_row(&HEADERS, sort, dimmed, &window);
    let cell_style = theme_row(dimmed);
    let rows = filtered.iter().map(|e| {
        let color = if dimmed {
            theme().dim
        } else {
            match (e.severity, e.kind.as_str()) {
                (crate::k8s::EventSeverity::Warning, "Node") => theme().bad,
                (crate::k8s::EventSeverity::Warning, _) => theme().warn,
                (crate::k8s::EventSeverity::Normal, _) => theme().ok,
            }
        };
        let type_text = match e.severity {
            crate::k8s::EventSeverity::Normal => "Normal",
            crate::k8s::EventSeverity::Warning => "Warning",
        };
        Row::new(window.slice(vec![
            Cell::from(type_text).style(Style::default().fg(color)),
            Cell::from(Line::from(highlight_matches(&e.reason, search, cell_style))),
            Cell::from(Line::from(highlight_matches(&e.object, search, cell_style))),
            Cell::from(Line::from(highlight_matches(&e.kind, search, cell_style))),
            Cell::from(Line::from(highlight_matches(&e.message, search, cell_style))),
            Cell::from(e.age.clone()).style(cell_style),
        ]))
    });

    // `Events (3/11)  (a) all  (w) warnings  (n) normal`, the active
    // severity highlighted, and `/text` while a search is applied.
    let active = if dimmed { dim_style() } else { Style::default().fg(theme().warm).add_modifier(Modifier::BOLD) };
    let idle = if dimmed { dim_style() } else { Style::default().fg(theme().muted) };
    let key_style = |this: EventFilter| if filter == this { active } else { idle };
    let count_style = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let title_spans = vec![
        Span::styled(format!("Events ({}/{})", filtered.len(), events.len()), count_style),
        Span::styled("  (a) all", key_style(EventFilter::All)),
        Span::styled("  (w) warnings", key_style(EventFilter::Warnings)),
        Span::styled("  (n) normal", key_style(EventFilter::Normal)),
    ];
    let title = Line::from(title_spans);

    let border_style = theme_border(dimmed);
    let table = Table::new(mark_rows(rows, &[], dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(with_search(Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title(title), search, editing, dimmed))
        .highlight_symbol("")
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, dimmed));

    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(filtered.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, state);
}

/// Which row of the Events table is under a terminal position, using the layout
/// of `draw_events_popup`. `offset` is the table's scroll offset, valid only after
/// that state has been rendered once.
pub fn event_row_at(frame_area: Rect, filtered_len: usize, offset: usize, row: u16) -> Option<usize> {
    let area = centered_rect(94, 88, frame_area);
    let top = area.y + 2; // top border + header row
    let bottom = area.y + area.height.saturating_sub(1); // bottom border
    if row < top || row >= bottom {
        return None;
    }
    let index = offset + usize::from(row - top);
    (index < filtered_len).then_some(index)
}

/// One event's full detail, a plain wrapped-text popup rather than a
/// table row, since the point is showing the *un*truncated message a
/// narrow MESSAGE column would otherwise clip.
pub(super) fn draw_event_detail_popup(frame: &mut Frame, entry: &EventEntry) {
    let area = centered_rect(70, 50, frame.area());
    frame.render_widget(Clear, area);

    let color = match (entry.severity, entry.kind.as_str()) {
        (crate::k8s::EventSeverity::Warning, "Node") => theme().bad,
        (crate::k8s::EventSeverity::Warning, _) => theme().warn,
        (crate::k8s::EventSeverity::Normal, _) => theme().ok,
    };
    let type_text = match entry.severity {
        crate::k8s::EventSeverity::Normal => "Normal",
        crate::k8s::EventSeverity::Warning => "Warning",
    };
    let label = Style::default().fg(theme().muted);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let lines = vec![
        Line::from(vec![Span::styled("Type:   ", label), Span::styled(type_text, Style::default().fg(color).add_modifier(Modifier::BOLD))]),
        Line::from(vec![Span::styled("Reason: ", label), Span::styled(entry.reason.clone(), bold)]),
        Line::from(vec![Span::styled("Object: ", label), Span::raw(format!("{} ({})", entry.object, entry.kind))]),
        Line::from(vec![Span::styled("Age:    ", label), Span::raw(entry.age.clone())]),
        Line::raw(""),
        Line::styled("Message:", bold),
        Line::raw(entry.message.clone()),
    ];

    let block = Block::default().borders(Borders::ALL).border_set(border_set()).title("Event detail");
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), area);
}

/// One cluster-wide meter as a real `Gauge`. The popup has room for the used,
/// capacity and percentage text in the gauge's own label.
pub(super) fn draw_gauge_box(frame: &mut Frame, area: Rect, label: &str, used: f64, capacity: f64, format_value: impl Fn(f64) -> String, dimmed: bool) {
    let ratio = if capacity > 0.0 { (used / capacity).clamp(0.0, 1.0) } else { 0.0 };
    let border_style = if dimmed { dim_style() } else { Style::default() };
    let title_style = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let gauge_style = if dimmed { dim_style() } else { Style::default().fg(usage_color(ratio, dimmed)) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(border_style)
        .title(Line::styled(format!(" {label} "), title_style));
    let label_text = format!("{} / {} ({:.0}%)", format_value(used), format_value(capacity), ratio * 100.0);
    let gauge = Gauge::default().block(block).gauge_style(gauge_style).ratio(ratio).label(label_text);
    frame.render_widget(gauge, area);
}

/// The Resources panel opened up: cluster-wide CPU/Memory/Pods as gauges. It
/// doesn't repeat the per-node breakdown the Nodes list owns.
pub(super) fn draw_resources_detail_popup(frame: &mut Frame, overview: &Overview, dimmed: bool) {
    let area = centered_rect(60, 30, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let outer = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title("Resources");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if !overview.metrics_available {
        let text = vec![
            Line::styled("metrics unavailable", Style::default().fg(theme().muted).add_modifier(Modifier::BOLD)),
            Line::styled("install metrics-server to see CPU/Memory usage", Style::default().fg(theme().muted)),
        ];
        frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
        return;
    }

    let pod_usage = workloads_pod_count(overview);
    let chunks = Layout::vertical([Constraint::Length(3); 3]).split(inner);
    draw_gauge_box(
        frame,
        chunks[0],
        "CPU",
        overview.cpu_usage_millicores as f64,
        overview.cpu_capacity_millicores as f64,
        |v| format!("{:.2} cores", v / 1000.0),
        dimmed,
    );
    draw_gauge_box(
        frame,
        chunks[1],
        "Memory",
        overview.memory_usage_bytes as f64,
        overview.memory_capacity_bytes as f64,
        format_bytes,
        dimmed,
    );
    draw_gauge_box(frame, chunks[2], "Pods", pod_usage as f64, overview.pod_capacity as f64, |v| format!("{v:.0}"), dimmed);
}

pub(super) fn draw_containers_popup(frame: &mut Frame, title: &str, containers: &[ContainerInfo], state: &mut TableState, sort: SortState, dimmed: bool) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);

    let muted = dim_style();
    const HEADERS: [&str; 4] = ["", "NAME", "STATE", "RESTARTS"];
    let state_text_of = |c: &ContainerInfo| {
        c.reason.clone().unwrap_or_else(|| match c.status {
            ContainerStatusKind::Running => "Running".into(),
            ContainerStatusKind::Waiting => "Waiting".into(),
            ContainerStatusKind::Terminated => "Terminated".into(),
            ContainerStatusKind::Unknown => "Unknown".into(),
        })
    };
    let window = layout_table(
        &HEADERS,
        containers.iter().map(|c| vec![1, cell_width(&c.name), cell_width(&state_text_of(c)), c.restarts.to_string().len()]),
        area.width.saturating_sub(2),
        None,
        &mut 0,
    );
    let header = header_row(&HEADERS, sort, dimmed, &window);
    let cell_style = theme_row(dimmed);
    let rows = containers.iter().map(|c| {
        let (glyph, color) = container_dot(c);
        let dot_style = if dimmed { muted } else { Style::default().fg(color) };
        Row::new(window.slice(vec![
            Cell::from(Span::styled(glyph, dot_style)),
            Cell::from(c.name.clone()).style(cell_style),
            Cell::from(state_text_of(c)).style(dot_style),
            Cell::from(c.restarts.to_string()).style(cell_style),
        ]))
    });

    let border_style = theme_border(dimmed);
    let table = Table::new(mark_rows(rows, &[], dimmed), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(dimmed))
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_set(border_set())
                .border_style(border_style)
                .title(if dimmed { Line::styled(title.to_string(), muted) } else { colored_slash_title(title) }),
        )
        .highlight_symbol("")
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, dimmed));

    frame.render_stateful_widget(table, area, state);
}

#[cfg(test)]
mod chip_tests {
    use super::*;

    #[test]
    fn a_click_on_a_chip_gives_its_number() {
        let frame = Rect { x: 0, y: 0, width: 120, height: 40 };
        let area = centered_rect(94, 88, frame);
        let bottom = area.y + area.height - 1;
        let origin = chips_origin(area);
        assert_eq!(slot_chip_at(frame, origin, bottom), Some(1));
        assert_eq!(slot_chip_at(frame, origin + 2, bottom), Some(1));
        assert_eq!(slot_chip_at(frame, origin + 4, bottom), Some(2));
        assert_eq!(slot_chip_at(frame, origin + 4 * 8, bottom), Some(9));
    }

    #[test]
    fn the_gaps_the_label_and_other_rows_are_not_chips() {
        let frame = Rect { x: 0, y: 0, width: 120, height: 40 };
        let area = centered_rect(94, 88, frame);
        let bottom = area.y + area.height - 1;
        let origin = chips_origin(area);
        assert_eq!(slot_chip_at(frame, origin + 3, bottom), None, "the gap between chips");
        assert_eq!(slot_chip_at(frame, origin - 1, bottom), None, "the label");
        assert_eq!(slot_chip_at(frame, origin + 4 * 9, bottom), None, "past 9");
        assert_eq!(slot_chip_at(frame, origin, bottom - 1), None, "not the border row");
    }
}

#[cfg(test)]
mod events_popup_tests {
    use super::*;

    fn entry(severity: crate::k8s::EventSeverity) -> EventEntry {
        EventEntry {
            message: "m".into(),
            reason: "r".into(),
            object: "o".into(),
            kind: "Pod".into(),
            age: "1m".into(),
            age_secs: 60,
            severity,
        }
    }

    #[test]
    fn workloads_pod_count_reads_the_pods_tile_from_the_catalog() {
        let overview = Overview {
            events: vec![],
            cpu_usage_millicores: 0,
            cpu_capacity_millicores: 0,
            memory_usage_bytes: 0,
            memory_capacity_bytes: 0,
            pod_capacity: 0,
            metrics_available: false,
            catalog: vec![("Workloads", vec![("Pods", 17), ("Deployments", 4)])],
            health: Default::default(),
        };
        assert_eq!(workloads_pod_count(&overview), 17);
    }

    #[test]
    fn event_row_at_resolves_the_first_row_and_respects_offset() {
        let frame_area = Rect { x: 0, y: 0, width: 100, height: 40 };
        let area = centered_rect(94, 88, frame_area);
        let top = area.y + 2;
        assert_eq!(event_row_at(frame_area, 5, 0, top), Some(0));
        assert_eq!(event_row_at(frame_area, 5, 2, top), Some(2));
        assert_eq!(event_row_at(frame_area, 5, 0, top - 1), None); // header row, not a data row
    }

    #[test]
    fn event_row_at_is_none_past_the_filtered_list_or_the_table() {
        let frame_area = Rect { x: 0, y: 0, width: 100, height: 40 };
        let area = centered_rect(94, 88, frame_area);
        let top = area.y + 2;
        assert_eq!(event_row_at(frame_area, 1, 0, top + 1), None); // only 1 row exists
        let bottom = area.y + area.height - 1;
        assert_eq!(event_row_at(frame_area, 100, 0, bottom), None); // bottom border row
    }

    #[test]
    fn event_filter_matches_the_right_severities() {
        let normal = entry(crate::k8s::EventSeverity::Normal);
        let warning = entry(crate::k8s::EventSeverity::Warning);
        assert!(EventFilter::All.matches(&normal) && EventFilter::All.matches(&warning));
        assert!(EventFilter::Warnings.matches(&warning) && !EventFilter::Warnings.matches(&normal));
        assert!(EventFilter::Normal.matches(&normal) && !EventFilter::Normal.matches(&warning));
    }

    #[test]
    fn events_search_matches_reason_object_kind_and_message_case_insensitively() {
        let mut crash = entry(crate::k8s::EventSeverity::Warning);
        crash.reason = "BackOff".into();
        crash.message = "Back-off restarting failed container".into();
        crash.object = "web-1".into();
        let ok = entry(crate::k8s::EventSeverity::Normal);
        let events = vec![crash, ok];
        let found = |filter, search: &str| crate::k8s::filter_events(&events, filter, search, None).len();
        assert_eq!(found(EventFilter::All, ""), 2);
        assert_eq!(found(EventFilter::All, "backoff"), 1); // reason
        assert_eq!(found(EventFilter::All, "WEB-1"), 1); // object
        assert_eq!(found(EventFilter::All, "restarting"), 1); // message
        assert_eq!(found(EventFilter::All, "pod"), 2); // kind, on both
        assert_eq!(found(EventFilter::All, "zzz"), 0);
        assert_eq!(found(EventFilter::Normal, "backoff"), 0); // severity filter still applies
    }
}
