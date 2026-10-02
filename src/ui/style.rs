//! Shared styling helpers every screen uses.

use super::*;

/// What a screen behind a popup is muted to, so it reads as out of focus.
pub(super) fn dim_style() -> Style {
    Style::default().fg(theme().dim).add_modifier(Modifier::DIM)
}

/// The plain row style every table uses.
pub(super) fn theme_row(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(theme().row) }
}

pub(super) fn theme_header(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(theme().header) }
}

/// The selected row: a solid bar with dark bold text over the row's own colours.
pub(super) fn selection_style(tone: crate::k8s::describe::Tone, dimmed: bool) -> Style {
    if dimmed {
        return dim_style();
    }
    // The bar wears the row's state: red on a broken pod, orange on a pending one,
    // grey on a finished one.
    let bg = match tone {
        crate::k8s::describe::Tone::Plain | crate::k8s::describe::Tone::Good => theme().select_bg,
        other => tone_color(other),
    };
    Style::default().bg(bg).fg(crate::theme::on(bg)).add_modifier(Modifier::BOLD)
}

/// Gives marked rows (Space) their own fill. `marked` says row by row which are marked.
pub(super) fn mark_rows<'a>(rows: impl Iterator<Item = Row<'a>>, marked: &[bool], dimmed: bool) -> Vec<Row<'a>> {
    let mark = if dimmed { dim_style() } else { Style::default().bg(theme().marked_bg) };
    rows.enumerate().map(|(i, row)| if marked.get(i).copied().unwrap_or(false) { row.style(mark) } else { row }).collect()
}

/// The key a marked row is remembered by: `namespace/name`, `-` when cluster-scoped.
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

/// Sets the box line style from the config.
pub fn configure_border(name: &str) {
    if let Ok(mut border) = BORDER.write() {
        *border = Some(border_set_named(name));
    }
}

/// The line style every box is drawn with.
pub fn border_set() -> ratatui::symbols::border::Set<'static> {
    BORDER.read().ok().and_then(|b| *b).unwrap_or_else(|| border_set_named(""))
}

/// How the main list draws this frame.
#[derive(Clone, Copy, Default)]
pub(super) struct ListLook {
    pub dimmed: bool,
    /// Has the keys while a panel is open beside it, so its border lights up.
    pub focused: bool,
    /// The sidebar has the keys, so the selection is marked quietly.
    pub unfocused: bool,
    /// Cells at the right of the top border kept for the badges.
    pub title_reserve: u16,
}

impl ListLook {
    pub fn dimmed(dimmed: bool) -> Self {
        ListLook { dimmed, ..Default::default() }
    }

    pub(super) fn border(self) -> Style {
        if !self.dimmed && self.focused { Style::default().fg(theme().accent).add_modifier(Modifier::BOLD) } else { theme_border(self.dimmed) }
    }

    pub(super) fn selection(self, tone: crate::k8s::describe::Tone) -> Style {
        if self.unfocused && !self.dimmed { Style::default().bg(theme().pill_bg).fg(theme().text_strong) } else { selection_style(tone, self.dimmed) }
    }
}

pub(super) fn theme_border(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(theme().border) }
}

/// A table title: the kind as a pill, the count in orange.
pub(super) fn table_title(label: &str, count: usize, window: &Window, dimmed: bool) -> Line<'static> {
    if dimmed {
        return Line::styled(format!(" {label} ({count}) "), dim_style());
    }
    let mut spans = vec![
        Span::styled(format!(" {label} "), Style::default().bg(theme().pill_bg).fg(theme().text_strong).add_modifier(Modifier::BOLD)),
        Span::styled(format!("({count})"), Style::default().fg(theme().warm).add_modifier(Modifier::BOLD)),
    ];
    // `‹ ›` when columns are scrolled out of view on that side, right after the count.
    if window.can_left || window.can_right {
        let hint = Style::default().fg(theme().warm);
        let arrows = match (window.can_left, window.can_right) {
            (true, true) => " ‹ ›",
            (true, false) => " ‹",
            _ => " ›",
        };
        spans.push(Span::styled(arrows, hint));
    }
    Line::from(spans)
}

/// ` search: text▏ ` for the right of a top border while a search is typed or applied.
fn search_title(text: &str, editing: bool, dimmed: bool, reserve: u16) -> Option<Line<'static>> {
    if text.is_empty() && !editing {
        return None;
    }
    let style = if dimmed { dim_style() } else { Style::default().fg(theme().highlight) };
    let reserve = usize::from(reserve);
    // The badges' room is filled with the border line, not blanks.
    let filler = border_set().horizontal_top.repeat(reserve);
    Some(Line::from(vec![Span::styled(format!(" search: {text}{} ", if editing { "▏" } else { "" }), style), Span::styled(filler, theme_border(dimmed))]).right_aligned())
}

/// `block` with the search, if any, on the right of its top border.
pub(super) fn with_search<'a>(block: Block<'a>, text: &str, editing: bool, dimmed: bool) -> Block<'a> {
    with_search_beside(block, text, editing, dimmed, 0)
}

/// `with_search`, keeping `reserve` cells at the right free for badges.
pub(super) fn with_search_beside<'a>(block: Block<'a>, text: &str, editing: bool, dimmed: bool, reserve: u16) -> Block<'a> {
    match search_title(text, editing, dimmed, reserve) {
        Some(line) => block.title_top(line),
        None => block,
    }
}

/// A `namespace/name` pair: namespace in the accent colour, name bold, `/` muted.
pub(super) fn namespace_name_spans(namespace: &str, name: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(namespace.to_string(), Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD)),
        Span::styled("/", Style::default().fg(theme().muted)),
        Span::styled(name.to_string(), Style::default().add_modifier(Modifier::BOLD)),
    ]
}

/// A `/`-joined title coloured like the path: namespace and container in their
/// accents, the rest bold. A title without `/` is plain bold.
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

/// A matched search character: yellow fill, dark bold, underlined so it shows on the
/// selected row too.
pub(super) fn match_style() -> Style {
    Style::default().bg(theme().highlight).fg(crate::theme::on(theme().highlight)).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// `text` with the characters `pattern` fuzzy-matched highlighted, or plain `base`
/// when nothing matches.
pub(super) fn highlight_fuzzy(text: &str, pattern: &str, base: Style) -> Line<'static> {
    let Some(positions) = (!pattern.is_empty()).then(|| crate::util::fuzzy::positions(pattern, text)).flatten() else {
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

/// A table header. In sort mode every column shows its number, `(1)NAME`; the sorted
/// column shows ▲ or ▼.
pub(super) fn header_row(names: &[&str], sort: SortState, dimmed: bool, window: &Window) -> Row<'static> {
    let text = theme_header(dimmed);
    let number = if dimmed { dim_style() } else { Style::default().fg(theme().warm).add_modifier(Modifier::BOLD) };
    let cells = window.range().map(|i| {
        let name = names[i];
        let mut spans = Vec::new();
        if sort.choosing && i < 10 {
            // Columns 0 to 9 are keys 0 to 9; later ones are reached with the cursor.
            spans.push(Span::styled(format!("({i})"), number));
        }
        let under_cursor = sort.cursor == Some(i) && !dimmed;
        spans.push(Span::styled(name.to_string(), if under_cursor { text.bg(theme().pill_bg).add_modifier(Modifier::UNDERLINED) } else { text }));
        if sort.column == Some(i) {
            spans.push(Span::styled(if sort.descending { " ▼" } else { " ▲" }, text));
        }
        Cell::from(Line::from(spans))
    });
    Row::new(cells)
}

/// A cell's tone as a style. Plain cells keep the row colour; everything dims behind a popup.
pub(super) fn tone_style(tone: crate::k8s::describe::Tone, dimmed: bool) -> Style {
    if dimmed {
        return dim_style();
    }
    Style::default().fg(tone_color(tone))
}

/// The state colours: healthy keeps the row colour, in progress orange, broken red,
/// finished grey.
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

/// A row's ordinary cells: the row colour, or its state's, so a failing pod is red
/// end to end.
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

/// A title as a pill, each part in its own text colour whatever the border's.
fn pill_spans(title: &str, dimmed: bool) -> Vec<Span<'static>> {
    if dimmed {
        return vec![Span::styled(format!(" {title} "), dim_style())];
    }
    let pill = Style::default().bg(theme().pill_bg);
    let mut spans = vec![Span::styled(" ", pill)];
    spans.extend(colored_slash_title(title).spans.into_iter().map(|s| {
        let fg = s.style.fg.unwrap_or(theme().text_strong);
        Span::styled(s.content, s.style.fg(fg).bg(theme().pill_bg).add_modifier(Modifier::BOLD))
    }));
    spans.push(Span::styled(" ", pill));
    spans
}

/// A popup title as a pill set in from the corner. The gap before it is drawn with
/// the border line, so the top edge stays unbroken.
pub(super) fn pill_title(title: &str, dimmed: bool, border: Style) -> Line<'static> {
    let mut spans = vec![Span::styled(border_set().horizontal_top.repeat(2), border)];
    spans.extend(pill_spans(title, dimmed));
    Line::from(spans)
}

/// The same pill in the middle of the top edge, for windows not about one object.
pub(super) fn pill_title_centered(title: &str, dimmed: bool) -> Line<'static> {
    Line::from(pill_spans(title, dimmed)).centered()
}

/// A box's own key hints in its bottom border, like `<o> open list`. Movement keys
/// are left out, since they work everywhere.
pub(super) fn hint_strip(hints: &[(&str, &str)]) -> Line<'static> {
    let key_style = Style::default().fg(theme().key).add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(theme().desc);
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, what)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(format!("<{key}>"), key_style));
        spans.push(Span::raw(" "));
        spans.push(Span::styled((*what).to_string(), desc_style));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}
