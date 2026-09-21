//! The shared colour palette and small styling helpers every screen uses.

use super::*;

/// The colour everything in a dimmed background layer is muted to: darker than
/// `DarkGray`, plus `DIM`, so a screen behind a popup reads as out of focus.
pub(super) fn dim_style() -> Style {
    Style::default().fg(theme().dim).add_modifier(Modifier::DIM)
}

/// The table palette, after k9s: pale-teal rows, a lighter blue header,
/// a solid pale-blue selection bar with dark text, and a slate border.
/// Every list/table goes through these so they read as one theme.
pub(super) fn theme_row(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(theme().row) }
}

pub(super) fn theme_header(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(theme().header) }
}

/// The selected row, after k9s: a solid pale-blue bar with dark bold text,
/// laid over the row's own colours (so a selected failing pod is still
/// findable by the path, not by its tint).
pub(super) fn selection_style(tone: crate::k8s::describe::Tone, dimmed: bool) -> Style {
    if dimmed {
        return dim_style();
    }
    // The bar wears the state of the row it is on, like k9s: red on a broken
    // pod, orange on a pending one, grey on a finished one.
    let bg = match tone {
        crate::k8s::describe::Tone::Plain | crate::k8s::describe::Tone::Good => theme().select_bg,
        other => tone_color(other),
    };
    Style::default().bg(bg).fg(crate::theme::on(bg)).add_modifier(Modifier::BOLD)
}

/// Gives marked rows (Space) their own fill, under the cells like the selection bar.
/// `marked` says, row by row, which are marked and may be empty (no marks).
pub(super) fn mark_rows<'a>(rows: impl Iterator<Item = Row<'a>>, marked: &[bool], dimmed: bool) -> Vec<Row<'a>> {
    let mark = if dimmed { dim_style() } else { Style::default().bg(theme().marked_bg) };
    rows.enumerate().map(|(i, row)| if marked.get(i).copied().unwrap_or(false) { row.style(mark) } else { row }).collect()
}

/// The key a marked row is remembered by: `namespace/name` (`-` when cluster-scoped).
pub fn mark_key(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}

use std::sync::RwLock;

/// The line style for a config name (`ui.border`); unknown names get the default.
pub fn border_set_named(name: &str) -> ratatui::symbols::border::Set<'static> {
    use ratatui::symbols::border;
    match name {
        "thick" => border::THICK,
        "double" => border::DOUBLE,
        _ => border::ROUNDED,
    }
}

static BORDER: RwLock<Option<ratatui::symbols::border::Set<'static>>> = RwLock::new(None);

/// Picks the box line style once, at startup, from the config.
pub fn configure_border(name: &str) {
    if let Ok(mut border) = BORDER.write() {
        *border = Some(border_set_named(name));
    }
}

/// The line style every box is drawn with.
pub fn border_set() -> ratatui::symbols::border::Set<'static> {
    BORDER.read().ok().and_then(|b| *b).unwrap_or_else(|| border_set_named(""))
}

static LIST_FOCUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the list has the keys while a panel is open beside it (its border lights up).
pub fn set_list_focused(focused: bool) {
    LIST_FOCUSED.store(focused, std::sync::atomic::Ordering::Relaxed);
}

/// The border of a list: bright while it has the keys next to an open panel.
pub(super) fn list_border(dimmed: bool) -> Style {
    if !dimmed && LIST_FOCUSED.load(std::sync::atomic::Ordering::Relaxed) { Style::default().fg(theme().accent).add_modifier(Modifier::BOLD) } else { theme_border(dimmed) }
}

pub(super) fn theme_border(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(theme().border) }
}

/// k9s-style table title: the kind as a filled pill, the count in orange.
pub(super) fn table_title(label: &str, count: usize, window: &Window, dimmed: bool) -> Line<'static> {
    if dimmed {
        return Line::styled(format!(" {label} ({count}) "), dim_style());
    }
    let mut spans = vec![
        Span::styled(format!(" {label} "), Style::default().bg(theme().pill_bg).fg(theme().text_strong).add_modifier(Modifier::BOLD)),
        Span::styled(format!("({count})"), Style::default().fg(theme().warm).add_modifier(Modifier::BOLD)),
    ];
    // `‹ ›` when columns are scrolled out of view on that side.
    if window.can_left || window.can_right {
        let hint = Style::default().fg(theme().warm);
        spans.push(Span::styled(format!("  {}{}", if window.can_left { "‹" } else { " " }, if window.can_right { "›" } else { "" }), hint));
    }
    Line::from(spans)
}

/// ` search: text▏ ` for the right end of a top border (`▏` is the cursor)
/// while a search is being typed or applied; nothing when there isn't one.
/// Every searchable screen shows it the same way.
static TITLE_RESERVE: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

/// Cells at the right of a list's top border taken by the sorting, faults and wide
/// badges, which the search text keeps clear of.
pub fn set_title_reserve(cells: u16) {
    TITLE_RESERVE.store(cells, std::sync::atomic::Ordering::Relaxed);
}

fn search_title(text: &str, editing: bool, dimmed: bool) -> Option<Line<'static>> {
    if text.is_empty() && !editing {
        return None;
    }
    let style = if dimmed { dim_style() } else { Style::default().fg(theme().highlight) };
    let reserve = usize::from(TITLE_RESERVE.load(std::sync::atomic::Ordering::Relaxed));
    // The room kept for the badges is filled with the border's own line, not blanks.
    let filler = border_set().horizontal_top.repeat(reserve);
    Some(Line::from(vec![Span::styled(format!(" search: {text}{} ", if editing { "▏" } else { "" }), style), Span::styled(filler, theme_border(dimmed))]).right_aligned())
}

/// `block` with the search, if any, on the right of its top border.
pub(super) fn with_search<'a>(block: Block<'a>, text: &str, editing: bool, dimmed: bool) -> Block<'a> {
    match search_title(text, editing, dimmed) {
        Some(line) => block.title_top(line),
        None => block,
    }
}

/// Shared namespace/name colouring: namespace in the cyan accent, name bold, `/`
/// muted. Used wherever a `namespace/name` pair shows up.
pub(super) fn namespace_name_spans(namespace: &str, name: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(namespace.to_string(), Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD)),
        Span::styled("/", Style::default().fg(theme().muted)),
        Span::styled(name.to_string(), Style::default().add_modifier(Modifier::BOLD)),
    ]
}

/// Colours a `/`-joined title like the path: namespace in the cyan accent, a
/// container name in its own accent, the rest bold, joined by muted `/`. A title
/// without `/` is plain bold.
pub(super) fn colored_slash_title(title: &str) -> Line<'static> {
    let parts: Vec<&str> = title.split('/').collect();
    if parts.len() < 2 {
        return Line::styled(title.to_string(), Style::default().add_modifier(Modifier::BOLD));
    }
    let sep = Style::default().fg(theme().muted);
    let plain = Style::default().add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(parts[0].to_string(), Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD))];
    for (i, part) in parts[1..].iter().enumerate() {
        spans.push(Span::styled("/", sep));
        let is_last = i == parts.len() - 2;
        let style = if is_last && parts.len() > 2 { Style::default().fg(theme().container).add_modifier(Modifier::BOLD) } else { plain };
        spans.push(Span::styled((*part).to_string(), style));
    }
    Line::from(spans)
}

/// The look of a matched search character: yellow fill, dark bold text,
/// underlined so it still shows on the selected row's own fill.
pub(super) fn match_style() -> Style {
    Style::default().bg(theme().highlight).fg(crate::theme::on(theme().highlight)).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// `text` with the characters the fuzzy filter `pattern` matched
/// highlighted; plain `base` when there's no pattern or it doesn't match
/// this cell. Consecutive matched characters share one span.
pub(super) fn highlight_fuzzy(text: &str, pattern: &str, base: Style) -> Line<'static> {
    let Some(positions) = (!pattern.is_empty()).then(|| crate::startup::fuzzy::positions(pattern, text)).flatten() else {
        return Line::styled(text.to_string(), base);
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_matched = false;
    for (i, ch) in text.chars().enumerate() {
        let matched = positions.binary_search(&i).is_ok();
        if matched != run_matched && !run.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut run), if run_matched { match_style() } else { base }));
        }
        run_matched = matched;
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_matched { match_style() } else { base }));
    }
    Line::from(spans)
}

#[cfg(test)]
mod highlight_tests {
    use super::*;

    fn texts(line: &Line) -> Vec<(String, bool)> {
        line.spans.iter().map(|s| (s.content.to_string(), s.style.bg == Some(theme().highlight))).collect()
    }

    #[test]
    fn matched_characters_are_split_into_highlighted_runs() {
        let line = highlight_fuzzy("kube-system", "ksy", Style::default());
        assert_eq!(texts(&line), [("k".into(), true), ("ube-".into(), false), ("sy".into(), true), ("stem".into(), false)]);
    }

    #[test]
    fn no_pattern_or_no_match_is_plain() {
        assert_eq!(texts(&highlight_fuzzy("default", "", Style::default())), [("default".into(), false)]);
        assert_eq!(texts(&highlight_fuzzy("default", "zzz", Style::default())), [("default".into(), false)]);
    }
}

/// A table header. In sort mode (`s`) every column carries its number,
/// `(1)NAME`; the sorted column always carries an arrow, `AGE ▲`
/// (ascending) or `AGE ▼` (descending).
pub(super) fn header_row(names: &[&str], sort: SortState, dimmed: bool, window: &Window) -> Row<'static> {
    let text = theme_header(dimmed);
    let number = if dimmed { dim_style() } else { Style::default().fg(theme().warm).add_modifier(Modifier::BOLD) };
    let cells = window.range().map(|i| {
        let name = names[i];
        let mut spans = Vec::new();
        if sort.choosing && i < 10 {
            // Columns 1-9 are keys 1-9; the tenth is key 0; later ones have none.
            spans.push(Span::styled(format!("({})", (i + 1) % 10), number));
        }
        spans.push(Span::styled(name.to_string(), text));
        if sort.column == Some(i) {
            spans.push(Span::styled(if sort.descending { " ▼" } else { " ▲" }, text));
        }
        Cell::from(Line::from(spans))
    });
    Row::new(cells)
}

/// The colour a cell's tone gets (see `describe::Tone`); plain cells keep
/// the row colour, and everything goes dim behind a popup.
pub(super) fn tone_style(tone: crate::k8s::describe::Tone, dimmed: bool) -> Style {
    if dimmed {
        return dim_style();
    }
    Style::default().fg(tone_color(tone))
}

/// The state colours, after k9s: healthy stays the row teal, in-progress
/// is orange, broken is a soft red, finished is grey.
pub(super) fn tone_color(tone: crate::k8s::describe::Tone) -> Color {
    use crate::k8s::describe::Tone;
    match tone {
        Tone::Plain => theme().row,
        Tone::Good => theme().ok,
        Tone::Warn => theme().warn,
        Tone::Bad => theme().bad,
        Tone::Muted => theme().muted,
    }
}

/// The style of a row's ordinary cells: the row teal, or the colour of the
/// row's state, so a failing pod reads as red from end to end.
pub(super) fn row_tone_style(tone: crate::k8s::describe::Tone, dimmed: bool) -> Style {
    tone_style(tone, dimmed)
}

#[cfg(test)]
mod row_style_tests {
    use super::*;

    fn marked_bg(row: &Row) -> bool {
        // `Row` keeps its style private; the debug form shows it.
        let bg = theme().marked_bg;
        format!("{row:?}").contains(&format!("{bg:?}"))
    }

    #[test]
    fn marked_rows_get_their_own_fill() {
        let rows = mark_rows((0..3).map(|_| Row::new(["x"])), &[false, true, false], false);
        assert_eq!(rows.iter().map(marked_bg).collect::<Vec<_>>(), [false, true, false]);
    }

    #[test]
    fn no_marks_leaves_rows_alone() {
        let rows = mark_rows((0..2).map(|_| Row::new(["x"])), &[], false);
        assert!(rows.iter().all(|r| !marked_bg(r)));
    }

    #[test]
    fn the_selection_is_a_pale_bar_with_dark_bold_text() {
        let style = selection_style(crate::k8s::describe::Tone::Plain, false);
        assert_eq!((style.bg, style.fg), (Some(theme().select_bg), Some(crate::theme::on(theme().select_bg))));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_bar_takes_the_colour_of_the_rows_state() {
        use crate::k8s::describe::Tone;
        assert_eq!(selection_style(Tone::Bad, false).bg, Some(theme().bad));
        assert_eq!(selection_style(Tone::Warn, false).bg, Some(theme().warn));
        assert_eq!(selection_style(Tone::Muted, false).bg, Some(theme().muted));
        assert_eq!(selection_style(Tone::Good, false).bg, Some(theme().select_bg));
        assert_eq!(selection_style(Tone::Bad, false).fg, Some(crate::theme::on(theme().bad)), "text stays readable on the state colour");
    }

    #[test]
    fn mark_keys_join_namespace_and_name() {
        assert_eq!(mark_key("kube-system", "coredns"), "kube-system/coredns");
        assert_eq!(mark_key("-", "node-1"), "-/node-1");
    }
}

#[cfg(test)]
mod border_tests {
    use super::*;

    #[test]
    fn names_pick_styles_and_unknown_ones_get_the_default() {
        assert_eq!(border_set_named("thick"), ratatui::symbols::border::THICK);
        assert_eq!(border_set_named("double"), ratatui::symbols::border::DOUBLE);
        assert_eq!(border_set_named("nonsense"), ratatui::symbols::border::ROUNDED);
        assert_eq!(border_set_named("thick").horizontal_top, "━");
        assert_eq!(border_set_named("double").horizontal_top, "═");
    }
}

/// A popup title as a pill set in from the corner, like the `sorting` and `wide` badges. The gap
/// before it is drawn with the border line (`border`), so the box's top edge stays unbroken.
pub(super) fn pill_title(title: &str, dimmed: bool, border: Style) -> Line<'static> {
    let gap = Span::styled(border_set().horizontal_top.repeat(2), border);
    if dimmed {
        return Line::from(vec![gap, Span::styled(format!(" {title} "), dim_style())]);
    }
    let pill = Style::default().bg(theme().pill_bg);
    let mut spans = vec![gap, Span::styled(" ", pill)];
    spans.extend(colored_slash_title(title).spans.into_iter().map(|s| Span::styled(s.content, s.style.bg(theme().pill_bg))));
    spans.push(Span::styled(" ", pill));
    Line::from(spans)
}
