//! Pickers for contexts, namespaces, slots and themes.

use super::*;

/// The size the context and namespace pickers and their chip hit-testing share.
fn picker_area(frame: Rect) -> Rect {
    centered_rect(94, 88, frame)
}

/// Contexts matching `filter` on name or cluster, best first. With no filter, the
/// current one comes first and the rest keep the kubeconfig's order.
pub fn context_matches<'a>(contexts: &'a [crate::k8s::ContextInfo], filter: &str) -> Vec<&'a crate::k8s::ContextInfo> {
    let mut scored: Vec<(i64, &crate::k8s::ContextInfo)> = contexts
        .iter()
        .filter_map(|c| {
            let score = crate::util::fuzzy::score(filter, &c.name).max(crate::util::fuzzy::score(filter, &c.cluster).map(|s| s / 2));
            score.map(|s| (if filter.is_empty() { i64::from(c.is_current) } else { s }, c))
        })
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, c)| c).collect()
}

/// The grey text beside a context: its cluster when named differently, and the
/// namespace it defaults to.
fn context_detail(c: &crate::k8s::ContextInfo) -> String {
    let mut parts = Vec::new();
    if c.cluster != c.name && !c.cluster.is_empty() {
        parts.push(c.cluster.clone());
    }
    if let Some(ns) = &c.namespace {
        parts.push(format!("ns {ns}"));
    }
    parts.join("   ")
}

/// Rows above the list: the filter line and a rule under it.
const CONTEXT_TOP: u16 = 2;

/// The picker's box and the rows its list takes, sized to the contexts and centred.
fn context_layout(full: Rect, items: &[&crate::k8s::ContextInfo], has_error: bool) -> (Rect, Rect) {
    // The dot, the longest name and the longest detail, as the table lays them out.
    let name_w = items.iter().map(|c| cell_width(&c.name)).max().unwrap_or(0);
    let detail_w = items.iter().map(|c| cell_width(&context_detail(c))).max().unwrap_or(0);
    let widest = (1 + 2 + name_w + if detail_w > 0 { 2 + detail_w } else { 0 }) as u16;
    // An error and its fix need room for a sentence and a command.
    let least = if has_error { 84 } else { 60 };
    let width = (widest + 4).clamp(least, 120).min(full.width.saturating_sub(4).max(20));
    // Borders, the filter and rule, a blank row under the list, and the error with
    // what to do about it.
    let chrome = 3 + CONTEXT_TOP + 3 * u16::from(has_error);
    let list_h = (items.len() as u16).clamp(1, (full.height * 2 / 3).saturating_sub(chrome).max(1));
    let height = (list_h + chrome).min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 3, width, height };
    let list = Rect { x: area.x + 2, y: area.y + 1 + CONTEXT_TOP, width: width.saturating_sub(4), height: list_h };
    (area, list)
}

/// The context picker, at startup and on `C`: a filter line, then each context with
/// a green dot on the current one. `leave` is what Esc does (`back` or `quit`).
pub fn draw_context_picker(frame: &mut Frame, view: ContextView) {
    let ContextView { items, total, filter, state, error, leave } = view;
    let full = frame.area();
    let (area, list) = context_layout(full, items, error.is_some());
    frame.render_widget(Clear, area);
    let title = if items.len() == total { format!("Contexts ({total})") } else { format!("Contexts ({}/{total})", items.len()) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(pill_title(&title, false, theme_border(false)))
        .title_bottom(hint_strip(&[("↑↓", "move"), ("enter", "connect"), ("esc", leave)]).right_aligned());
    frame.render_widget(block, area);

    let muted = Style::default().fg(theme().muted);
    let prompt = if filter.is_empty() {
        Line::from(vec![Span::styled("> ", Style::default().fg(theme().accent)), Span::styled("▏", Style::default().fg(theme().highlight)), Span::styled("type to filter", muted)])
    } else {
        Line::from(vec![Span::styled("> ", Style::default().fg(theme().accent)), Span::styled(filter.to_string(), Style::default().fg(theme().highlight).add_modifier(Modifier::BOLD)), Span::styled("▏", Style::default().fg(theme().highlight))])
    };
    frame.render_widget(Paragraph::new(prompt), Rect { y: area.y + 1, height: 1, ..list });
    frame.render_widget(Paragraph::new(Line::styled("─".repeat(usize::from(list.width)), Style::default().fg(theme().panel_bg))), Rect { y: area.y + 2, height: 1, ..list });

    if items.is_empty() {
        frame.render_widget(Paragraph::new(Line::styled("No context matches", muted)), list);
    } else {
        let detail_w = items.iter().map(|c| cell_width(&context_detail(c))).max().unwrap_or(0) as u16;
        let name_room = usize::from(list.width.saturating_sub(3 + if detail_w > 0 { detail_w + 2 } else { 0 }));
        // Long names scroll sideways with the arrows.
        let shift = popup_hscroll().min(items.iter().map(|c| cell_width(&c.name)).max().unwrap_or(0).saturating_sub(name_room));
        set_popup_hscroll(shift);
        let rows = items.iter().map(|c| {
            let dot = if c.is_current { Span::styled("●", Style::default().fg(theme().ok)) } else { Span::raw(" ") };
            Row::new(vec![
                Cell::from(Line::from(dot)),
                Cell::from(highlight_fuzzy(&truncate(&c.name.chars().skip(shift).collect::<String>(), name_room), filter, Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD))),
                Cell::from(Line::styled(context_detail(c), muted).right_aligned()),
            ])
        });
        let table = Table::new(rows, [Constraint::Length(1), Constraint::Fill(1), Constraint::Length(detail_w)])
            .column_spacing(2)
            .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, false));
        if let Some(selected) = state.selected() {
            state.select(Some(selected.min(items.len() - 1)));
        }
        frame.render_stateful_widget(table, list, state);
    }
    // The error in red, and on the next line what to do about it.
    if let Some(err) = error {
        let width = usize::from(list.width);
        let mut lines = err.lines();
        let reason = Line::styled(truncate(lines.next().unwrap_or(""), width), Style::default().fg(theme().bad).add_modifier(Modifier::BOLD));
        let fix = Line::styled(truncate(lines.next().unwrap_or(""), width), Style::default().fg(theme().text_soft));
        frame.render_widget(Paragraph::new(vec![reason, fix]), Rect { y: list.y + list.height + 1, height: 2, ..list });
    }
}

/// The context under a click on row `row`, if any.
pub fn context_row_at(frame_area: Rect, items: &[&crate::k8s::ContextInfo], has_error: bool, offset: usize, row: u16) -> Option<usize> {
    let (_, list) = context_layout(frame_area, items, has_error);
    if row < list.y || row >= list.y + list.height {
        return None;
    }
    let index = offset + usize::from(row - list.y);
    (index < items.len()).then_some(index)
}

/// The Permissions menu: this cluster's mode (your role or read-only), and the
/// contexts that are always read-only.
pub(in crate::ui) fn draw_permissions(frame: &mut Frame, view: PermissionsView) {
    let full = frame.area();
    let muted = Style::default().fg(theme().muted);
    let strong = Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD);
    let selected = selection_style(crate::k8s::describe::Tone::Plain, false);
    let pick = |i: usize| i == view.cursor && view.input.is_none();
    // A row: the text, a grey note on the right, highlighted under the cursor.
    let row = |text: Vec<Span<'static>>, note: String, on: bool, width: usize| {
        let used: usize = text.iter().map(|s| cell_width(&s.content)).sum::<usize>() + cell_width(&note);
        let mut spans = text;
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
        spans.push(Span::styled(note, muted));
        let line = Line::from(spans);
        if on { line.style(selected) } else { line }
    };

    let width = 74u16.min(full.width.saturating_sub(4));
    let inner_w = usize::from(width.saturating_sub(4));
    let mut lines: Vec<Line> = Vec::new();
    // The tabs, the active one filled.
    let tab = |label: &str, active: bool| Span::styled(format!(" {label} "), if active { Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD) } else { muted });
    lines.push(Line::from(vec![tab("This cluster", view.tab == 0), Span::raw("  "), tab("Read-only contexts", view.tab == 1), Span::raw("  "), tab("Highlighted", view.tab == 2)]));
    lines.push(Line::raw(""));
    let hints: &[(&str, &str)] = if view.tab == 0 {
        lines.push(Line::from(vec![Span::styled("Context  ", muted), Span::styled(view.context.to_string(), strong)]));
        lines.push(Line::raw(""));
        let dot = |on: bool| if on { Span::styled("● ", Style::default().fg(theme().ok)) } else { Span::styled("○ ", muted) };
        let role = if view.role.is_empty() { "Your role".to_string() } else { format!("Your role ({})", view.role) };
        lines.push(row(vec![dot(!view.read_only), Span::styled(role, strong)], "what your permissions allow".into(), pick(0), inner_w));
        lines.push(row(vec![dot(view.read_only), Span::styled("Read-only".to_string(), strong)], "no deletes, edits, scaling or shells".into(), pick(1), inner_w));
        &[("↑↓", "move"), ("enter", "choose"), ("tab", "switch"), ("esc", "close")]
    } else {
        // The read-only tab leads with a switch for every context; Highlighted says what it does.
        let first = if view.tab == 1 {
            let onoff = |on: bool| Span::styled(if on { "on" } else { "off" }, Style::default().fg(if on { theme().warn } else { theme().muted }).add_modifier(Modifier::BOLD));
            lines.push(row(vec![Span::styled("All contexts  ".to_string(), strong), onoff(view.everywhere)], "every cluster is read-only".into(), pick(0), inner_w));
            1
        } else {
            lines.push(Line::styled("Their header turns red, so you notice where you are.", muted));
            lines.push(Line::raw(""));
            0
        };
        for (i, pattern) in view.contexts.iter().enumerate() {
            let text = match view.input {
                Some((Some(at), typed)) if at == i => vec![Span::styled("> ", Style::default().fg(theme().accent)), Span::styled(format!("{typed}▏"), Style::default().fg(theme().highlight))],
                _ => vec![Span::raw("  "), Span::raw(pattern.clone())],
            };
            let note = if pattern.contains('*') { "pattern" } else if pattern == view.context { "this cluster" } else { "" };
            lines.push(row(text, note.into(), pick(i + first), inner_w));
        }
        let add = match view.input {
            Some((None, typed)) => vec![Span::styled("> ", Style::default().fg(theme().accent)), Span::styled(format!("{typed}▏"), Style::default().fg(theme().highlight))],
            _ => vec![Span::styled("+ Add a context or pattern".to_string(), Style::default().fg(theme().accent))],
        };
        lines.push(row(add, String::new(), pick(view.contexts.len() + first), inner_w));
        lines.push(Line::raw(""));
        lines.push(Line::styled("* matches anything: prod* or *payments*", muted));
        if view.input.is_some() { &[("enter", "save"), ("esc", "cancel")] } else { &[("enter", "edit"), ("a", "add"), ("d", "delete"), ("tab", "switch"), ("esc", "close")] }
    };
    if let Some(error) = view.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(truncate(error, inner_w), Style::default().fg(theme().bad)));
    }

    // Borders, and a blank row above and below the content.
    let height = (lines.len() as u16 + 4).min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 3, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(pill_title("Permissions", false, theme_border(false)))
        .title_bottom(hint_strip(hints).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: inner.height.saturating_sub(1) });
}

/// The chip strip on the namespace picker's bottom border: a label, then keys 1 to 9,
/// three cells wide and one apart.
const CHIP_LABEL: &str = " assign to key: ";

fn chips_origin(area: Rect) -> u16 {
    area.x + 1 + CHIP_LABEL.chars().count() as u16
}

/// The number chip a click on the namespace picker's bottom border lands on.
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
    let window = layout_popup(&HEADERS, items.iter().map(|(name, _)| vec![cell_width(name), 1]), area.width.saturating_sub(2), None);
    let header = header_row(&HEADERS, sort, false, &window);
    let rows = items.iter().map(|(name, key)| {
        Row::new(window.slice(vec![
            Cell::from(highlight_fuzzy(name, filter, Style::default().add_modifier(Modifier::BOLD))),
            Cell::from(key.map(|k| k.to_string()).unwrap_or_default()).style(Style::default().fg(theme().warm)),
        ]))
    });

    let title = pill_title(&format!("Choose the namespace to filter by ({}/{total})", items.len()), false, Style::default());

    // Each key's chip is lit when the highlighted namespace has it, orange when another does.
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

/// The key picker: `0` (always all) and keys 1 to 9 with what each holds. Enter puts
/// the namespace on the highlighted key.
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

/// The theme picker's box: wide enough for the list and a preview beside it.
fn theme_picker_area(frame_area: Rect) -> Rect {
    centered_rect(90, 86, frame_area)
}

/// The list's width inside the picker; the preview takes the rest.
const THEME_LIST_WIDTH: u16 = 46;

/// The theme list with a swatch per theme, and beside it a sample screen drawn in the
/// theme being previewed.
pub(in crate::ui) fn draw_theme_picker(frame: &mut Frame, entries: &[crate::theme::ThemeEntry], state: &mut TableState, saved: &str) {
    let area = theme_picker_area(frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(pill_title(&format!("Themes ({})", entries.len()), false, theme_border(false)))
        .title_bottom(hint_strip(&[("enter", "keep"), ("esc", "cancel")]).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let list = Rect { width: THEME_LIST_WIDTH.min(inner.width), ..inner };
    // The swatch skips the background and text colours, which the preview already shows.
    // The selected row is shaded through the row style so its dots keep their colours.
    let selected = state.selected();
    let rows: Vec<Row> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let swatch: Vec<Span> = entry.swatch.iter().skip(2).map(|color| Span::styled("● ", Style::default().fg(*color))).collect();
            let mark = if entry.name == saved { "✔" } else { "" };
            let row = Row::new(vec![
                Cell::from(Span::styled(entry.name.clone(), Style::default().add_modifier(Modifier::BOLD))),
                Cell::from(Line::from(swatch)),
                Cell::from(Span::styled(mark, Style::default().fg(theme().ok))),
            ]);
            if selected == Some(i) { row.style(selection_style(crate::k8s::describe::Tone::Plain, false)) } else { row }
        })
        .collect();
    let table = Table::new(rows, [Constraint::Length(20), Constraint::Length(16), Constraint::Min(2)])
        .column_spacing(2)
        .style(theme_row(false));
    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(entries.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, list, state);
    let preview = Rect { x: list.right() + 2, width: inner.right().saturating_sub(list.right() + 3), ..inner };
    if preview.width >= 30 {
        draw_theme_preview(frame, preview);
    }
}

/// A small sample of knav in the current theme: a list with every row state, the
/// breadcrumbs, a label and a diff.
fn draw_theme_preview(frame: &mut Frame, area: Rect) {
    use crate::k8s::describe::Tone;
    let border = theme_border(false);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border).title(pill_title("Pods (4)", false, border));
    let inner = block.inner(area);
    frame.render_widget(Block::default().style(Style::default().bg(theme().background)), area);
    frame.render_widget(block, Rect { height: area.height.min(8), ..area });
    let header = Style::default().fg(theme().header).add_modifier(Modifier::BOLD);
    let width = usize::from(inner.width);
    let row = |ns: &str, name: &str, status: &str, style: Style| Line::styled(format!("{:<width$}", format!("{ns:<10}{name:<14}{status}")), style);
    let lines = vec![
        Line::styled(format!("{:<10}{:<14}{}", "NAMESPACE", "NAME", "STATUS"), header),
        row("shop", "web-7d9f", "Running", selection_style(Tone::Plain, false)),
        row("shop", "worker-2", "Running", Style::default().fg(theme().row)),
        row("shop", "migrate-1", "Pending", Style::default().fg(theme().warn)),
        row("shop", "cache-0", "CrashLoopBackOff", Style::default().fg(theme().bad)),
        row("staging", "cleanup-9", "Completed", Style::default().fg(theme().muted)),
    ];
    frame.render_widget(Paragraph::new(lines), Rect { height: inner.height.min(6), ..inner });

    // Below the box: the breadcrumbs, a label pill and a diff, each on its own row.
    let pill = Style::default().bg(theme().pill_bg);
    let sep = Span::styled(" › ", Style::default().fg(theme().muted));
    let current = Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg));
    let below = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled(" Pods ", pill.fg(theme().namespace).add_modifier(Modifier::BOLD)),
            sep.clone(),
            Span::styled(" shop/web-7d9f ", current.add_modifier(Modifier::BOLD)),
            sep,
            Span::styled(" ● nginx Running ", pill.fg(theme().ok)),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Labels  ", Style::default().fg(theme().muted)),
            Span::styled(" app", pill.fg(theme().namespace)),
            Span::styled("=", pill.fg(theme().muted)),
            Span::styled("web ", pill.fg(theme().text_strong)),
        ]),
        Line::raw(""),
        Line::styled("- image: nginx:1.25", Style::default().fg(theme().bad)),
        Line::styled("+ image: nginx:1.26", Style::default().fg(theme().ok)),
        Line::styled("  replicas: 3", Style::default().fg(theme().muted)),
    ];
    let top = area.y + area.height.min(8);
    frame.render_widget(Paragraph::new(below), Rect { y: top, height: area.bottom().saturating_sub(top), ..area });
}

/// Which theme row a click lands on.
pub fn theme_row_at(frame_area: Rect, len: usize, offset: usize, row: u16) -> Option<usize> {
    let area = theme_picker_area(frame_area);
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
