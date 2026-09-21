//! The shared colour palette and small styling helpers every screen uses.

use super::*;

/// The color everything in a dimmed "background" layer is muted down
/// to — deliberately darker than plain ANSI `DarkGray` (which most
/// terminals render as a fairly legible mid-gray) plus the `DIM`
/// modifier on top, so a screen sitting behind a popup reads as
/// unmistakably out of focus rather than just "a bit gray."
pub(super) fn dim_style() -> Style {
    Style::default().fg(Color::Rgb(40, 40, 40)).add_modifier(Modifier::DIM)
}

/// The table palette, after k9s: pale-teal rows, a lighter blue header,
/// a solid pale-blue selection bar with dark text, and a slate border.
/// Every list/table goes through these so they read as one theme.
pub(super) const ROW_FG: Color = Color::Rgb(143, 191, 208);
pub(super) const HEADER_FG: Color = Color::Rgb(137, 180, 250);
pub(super) const SELECT_BG: Color = Color::Rgb(148, 191, 206);
pub(super) const BORDER_FG: Color = Color::Rgb(96, 125, 139);

pub(super) fn theme_row(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(ROW_FG) }
}

pub(super) fn theme_header(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(HEADER_FG) }
}

/// The selected row, after k9s: a solid pale-blue bar with dark bold text,
/// laid over the row's own colours (so a selected failing pod is still
/// findable by the breadcrumb, not by its tint).
pub(super) fn selection_style(tone: crate::describe::Tone, dimmed: bool) -> Style {
    if dimmed {
        return dim_style();
    }
    // The bar wears the state of the row it is on, like k9s: red on a broken
    // pod, orange on a pending one, grey on a finished one.
    let bg = match tone {
        crate::describe::Tone::Plain | crate::describe::Tone::Good => SELECT_BG,
        other => tone_color(other),
    };
    Style::default().bg(bg).fg(Color::Black).add_modifier(Modifier::BOLD)
}

/// Rows the user marked (Space) get their own fill, under the cells like the selection bar.
const MARKED_ROW_BG: Color = Color::Rgb(84, 72, 24);

/// Gives marked rows their own fill; `marked` says, row by row, which are
/// marked and may be empty (no marks).
pub(super) fn mark_rows<'a>(rows: impl Iterator<Item = Row<'a>>, marked: &[bool], dimmed: bool) -> Vec<Row<'a>> {
    let mark = if dimmed { dim_style() } else { Style::default().bg(MARKED_ROW_BG) };
    rows.enumerate().map(|(i, row)| if marked.get(i).copied().unwrap_or(false) { row.style(mark) } else { row }).collect()
}

/// The key a marked row is remembered by: `namespace/name` (`-` when cluster-scoped).
pub fn mark_key(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}

pub(super) fn theme_border(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(BORDER_FG) }
}

/// k9s-style table title: the kind as a filled pill, the count in orange.
pub(super) fn table_title(label: &str, count: usize, search: Search, window: &Window, dimmed: bool) -> Line<'static> {
    if dimmed {
        return Line::styled(format!(" {label} ({count}) "), dim_style());
    }
    let mut spans = vec![
        Span::styled(format!(" {label} "), Style::default().bg(Color::Rgb(50, 56, 72)).fg(Color::Rgb(226, 232, 240)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("({count})"), Style::default().fg(Color::Rgb(240, 160, 110)).add_modifier(Modifier::BOLD)),
    ];
    spans.extend(search_span(search.text, search.editing, false));
    // `‹ ›` when columns are scrolled out of view on that side.
    if window.can_left || window.can_right {
        let hint = Style::default().fg(Color::Rgb(240, 160, 110));
        spans.push(Span::styled(format!("  {}{}", if window.can_left { "‹" } else { " " }, if window.can_right { "›" } else { "" }), hint));
    }
    Line::from(spans)
}

/// `  search: text▏` for a title while a search is being typed (`▏` is the
/// cursor) or applied; nothing when there isn't one. Every searchable
/// screen shows it the same way.
pub(super) fn search_span(text: &str, editing: bool, dimmed: bool) -> Option<Span<'static>> {
    if text.is_empty() && !editing {
        return None;
    }
    let style = if dimmed { dim_style() } else { Style::default().fg(Color::Yellow) };
    Some(Span::styled(format!("  search: {text}{}", if editing { "▏" } else { "" }), style))
}

/// Shared namespace/name coloring — namespace in the app's cyan accent,
/// name in plain bold, `/` muted — the same "kind vs value" split the
/// breadcrumb uses, reused everywhere a `namespace/name` pair shows up
/// (this status line, the Containers/Logs popup titles) so it's one
/// defined color pairing rather than a different pick per screen.
pub(super) fn namespace_name_spans(namespace: &str, name: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(namespace.to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled("/", Style::default().fg(Color::DarkGray)),
        Span::styled(name.to_string(), Style::default().add_modifier(Modifier::BOLD)),
    ]
}

/// Colors a `/`-joined title (`namespace/name`, or `namespace/pod/
/// container` for Logs) the same way as the breadcrumb: the outermost
/// segment (namespace) in the app's cyan accent, the innermost (a
/// container name, when there is one) in a distinct accent of its own,
/// everything else plain bold — joined by muted `/`s instead of one
/// flat-colored string. Falls back to plain bold for a title with no
/// `/` at all (a bare node name, say).
pub(super) fn colored_slash_title(title: &str) -> Line<'static> {
    let parts: Vec<&str> = title.split('/').collect();
    if parts.len() < 2 {
        return Line::styled(title.to_string(), Style::default().add_modifier(Modifier::BOLD));
    }
    let sep = Style::default().fg(Color::DarkGray);
    let plain = Style::default().add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(parts[0].to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))];
    for (i, part) in parts[1..].iter().enumerate() {
        spans.push(Span::styled("/", sep));
        let is_last = i == parts.len() - 2;
        let style = if is_last && parts.len() > 2 { Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD) } else { plain };
        spans.push(Span::styled((*part).to_string(), style));
    }
    Line::from(spans)
}

/// The look of a matched search character: yellow fill, dark bold text,
/// underlined so it still shows on the selected row's own fill.
pub(super) fn match_style() -> Style {
    Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// `text` with the characters the fuzzy filter `pattern` matched
/// highlighted; plain `base` when there's no pattern or it doesn't match
/// this cell. Consecutive matched characters share one span.
pub(super) fn highlight_fuzzy(text: &str, pattern: &str, base: Style) -> Line<'static> {
    let Some(positions) = (!pattern.is_empty()).then(|| crate::fuzzy::positions(pattern, text)).flatten() else {
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
        line.spans.iter().map(|s| (s.content.to_string(), s.style.bg == Some(Color::Yellow))).collect()
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
    let number = if dimmed { dim_style() } else { Style::default().fg(Color::Rgb(240, 160, 110)).add_modifier(Modifier::BOLD) };
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
pub(super) fn tone_style(tone: crate::describe::Tone, dimmed: bool) -> Style {
    if dimmed {
        return dim_style();
    }
    Style::default().fg(tone_color(tone))
}

/// The state colours, after k9s: healthy stays the row teal, in-progress
/// is orange, broken is a soft red, finished is grey.
pub const OK_FG: Color = Color::Rgb(126, 201, 140);
pub const WARN_FG: Color = Color::Rgb(255, 167, 64);
pub const BAD_FG: Color = Color::Rgb(217, 96, 106);
pub const MUTED_FG: Color = Color::Rgb(122, 128, 148);

pub(super) fn tone_color(tone: crate::describe::Tone) -> Color {
    use crate::describe::Tone;
    match tone {
        Tone::Plain => ROW_FG,
        Tone::Good => OK_FG,
        Tone::Warn => WARN_FG,
        Tone::Bad => BAD_FG,
        Tone::Muted => MUTED_FG,
    }
}

/// The style of a row's ordinary cells: the row teal, or the colour of the
/// row's state, so a failing pod reads as red from end to end.
pub(super) fn row_tone_style(tone: crate::describe::Tone, dimmed: bool) -> Style {
    tone_style(tone, dimmed)
}

#[cfg(test)]
mod row_style_tests {
    use super::*;

    fn marked_bg(row: &Row) -> bool {
        // `Row` keeps its style private; the debug form shows it.
        format!("{row:?}").contains(&format!("{MARKED_ROW_BG:?}"))
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
        let style = selection_style(crate::describe::Tone::Plain, false);
        assert_eq!((style.bg, style.fg), (Some(SELECT_BG), Some(Color::Black)));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_bar_takes_the_colour_of_the_rows_state() {
        use crate::describe::Tone;
        assert_eq!(selection_style(Tone::Bad, false).bg, Some(BAD_FG));
        assert_eq!(selection_style(Tone::Warn, false).bg, Some(WARN_FG));
        assert_eq!(selection_style(Tone::Muted, false).bg, Some(MUTED_FG));
        assert_eq!(selection_style(Tone::Good, false).bg, Some(SELECT_BG));
        assert_eq!(selection_style(Tone::Bad, false).fg, Some(Color::Black), "text stays dark and readable");
    }

    #[test]
    fn mark_keys_join_namespace_and_name() {
        assert_eq!(mark_key("kube-system", "coredns"), "kube-system/coredns");
        assert_eq!(mark_key("-", "node-1"), "-/node-1");
    }
}
