//! Pickers for contexts, namespaces, slots and themes.

use super::*;

/// Every picker in this file (context, namespace) and its chip hit-testing share this size, so
/// they all have to agree on it.
fn picker_area(frame: Rect) -> Rect {
    centered_rect(94, 88, frame)
}

/// The `:ctx` / `C` context browser, same full-size table as the
/// Events browser (and the same geometry, so `event_row_at` hit-tests
/// its rows too). Typing filters immediately, no `/` needed.
pub(in crate::ui) fn draw_context_popup(
    frame: &mut Frame,
    items: &[(String, String, bool)],
    total: usize,
    filter: &str,
    editing: bool,
    state: &mut TableState,
    error: Option<&str>,
    sort: SortState,
) {
    let area = picker_area(frame.area());
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

    let mut title = pill_title(&format!("Contexts ({}/{total})", items.len()), false, Style::default());
    if let Some(err) = error {
        title.push_span(Span::styled(format!("  -  {err}"), Style::default().fg(theme().bad)));
    }

    let block = with_search(Block::default().borders(Borders::ALL).border_set(border_set()).title(title), filter, editing, false)
        .title_bottom(hint_strip(&[("type", "filter"), ("↑↓", "move"), ("enter", "connect"), ("esc", "back")]).right_aligned());
    let table = Table::new(mark_rows(rows, &[], false), window.constraints.clone())
        .column_spacing(COLUMN_GAP)
        .style(theme_row(false))
        .header(header)
        .block(block)
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

fn chips_origin(area: Rect) -> u16 {
    area.x + 1 + CHIP_LABEL.chars().count() as u16
}

/// Which number chip (1-9) a click on the namespace picker's bottom border
/// lands on.
pub fn slot_chip_at(frame_area: Rect, column: u16, row: u16) -> Option<usize> {
    let area = picker_area(frame_area);
    if row != area.y + area.height.saturating_sub(1) || column < chips_origin(area) {
        return None;
    }
    let offset = column - chips_origin(area);
    let (chip, within) = (usize::from(offset / 4), offset % 4);
    (chip < 9 && within < 3).then_some(chip + 1)
}

pub(in crate::ui) fn draw_namespace_picker(
    frame: &mut Frame,
    items: &[(String, Option<usize>)],
    total: usize,
    filter: &str,
    editing: bool,
    state: &mut TableState,
    sort: SortState,
) {
    let area = picker_area(frame.area());
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

    let title = pill_title(&format!("Choose the namespace to filter by ({}/{total})", items.len()), false, Style::default());

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
pub(in crate::ui) fn draw_slots_popup(frame: &mut Frame, namespace: &str, slots: &[Option<String>], selected: usize) {
    let bar = centered_box(frame.area(), 2 + 1 + 9 + 1 + 1);
    frame.render_widget(Clear, bar);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .title(pill_title(&format!("Choose the key for '{namespace}'"), false, Style::default()));
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
            None => "-".to_string(),
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

/// The theme list: each theme with a strip of its colours. The screen behind
/// is drawn in the theme being previewed, so the whole interface is the sample.
pub(in crate::ui) fn draw_theme_picker(frame: &mut Frame, entries: &[crate::app::mode::ThemeEntry], state: &mut TableState, saved: &str) {
    let area = centered_rect(64, 86, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(pill_title(&format!("Themes ({})", entries.len()), false, theme_border(false)))
        .title_bottom(hint_strip(&[("enter", "keeps"), ("esc", "cancels")]).right_aligned());
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

#[cfg(test)]
mod chip_tests {
    use super::*;

    #[test]
    fn a_click_on_a_chip_gives_its_number() {
        let frame = Rect { x: 0, y: 0, width: 120, height: 40 };
        let area = picker_area(frame);
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
        let area = picker_area(frame);
        let bottom = area.y + area.height - 1;
        let origin = chips_origin(area);
        assert_eq!(slot_chip_at(frame, origin + 3, bottom), None, "the gap between chips");
        assert_eq!(slot_chip_at(frame, origin - 1, bottom), None, "the label");
        assert_eq!(slot_chip_at(frame, origin + 4 * 9, bottom), None, "past 9");
        assert_eq!(slot_chip_at(frame, origin, bottom - 1), None, "not the border row");
    }
}
