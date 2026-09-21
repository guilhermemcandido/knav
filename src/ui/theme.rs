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

/// The selected row's bar: a background fill only, applied *underneath*
/// the row's cells (`Row::style`), so each cell keeps its own colours — the
/// container dots, status colours and the yellow search match all stay
/// visible on the selected row instead of being painted over.
const SELECTED_ROW_BG: Color = Color::Rgb(58, 74, 96);

/// Rows the user marked (Space) get their own fill, under the cells like the selection bar.
const MARKED_ROW_BG: Color = Color::Rgb(84, 72, 24);

/// `marked` says, row by row, which are marked; it may be empty (no marks).
pub(super) fn select_rows<'a>(rows: impl Iterator<Item = Row<'a>>, selected: Option<usize>, marked: &[bool], dimmed: bool) -> Vec<Row<'a>> {
    let bar = if dimmed { dim_style() } else { Style::default().bg(SELECTED_ROW_BG).add_modifier(Modifier::BOLD) };
    let mark = if dimmed { dim_style() } else { Style::default().bg(MARKED_ROW_BG) };
    rows.enumerate()
        .map(|(i, row)| match () {
            _ if Some(i) == selected => row.style(bar),
            _ if marked.get(i).copied().unwrap_or(false) => row.style(mark),
            _ => row,
        })
        .collect()
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
        if sort.choosing {
            // Columns 1-9 are keys 1-9; the tenth is key 0.
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
    use crate::describe::Tone;
    if dimmed {
        return dim_style();
    }
    match tone {
        Tone::Plain => Style::default().fg(ROW_FG),
        Tone::Good => Style::default().fg(Color::Green),
        Tone::Warn => Style::default().fg(Color::Yellow),
        Tone::Bad => Style::default().fg(Color::Red),
        Tone::Muted => Style::default().fg(Color::DarkGray),
    }
}

#[cfg(test)]
mod row_style_tests {
    use super::*;

    fn bg(row: &Row) -> Option<Color> {
        // `Row` keeps its style private; the debug form shows it.
        let text = format!("{row:?}");
        [SELECTED_ROW_BG, MARKED_ROW_BG].into_iter().find(|c| text.contains(&format!("{c:?}")))
    }

    fn rows() -> Vec<Row<'static>> {
        select_rows((0..3).map(|_| Row::new(["x"])), Some(0), &[false, true, false], false)
    }

    #[test]
    fn the_cursor_row_gets_the_selection_fill_and_marked_rows_their_own() {
        let rows = rows();
        assert_eq!(bg(&rows[0]), Some(SELECTED_ROW_BG));
        assert_eq!(bg(&rows[1]), Some(MARKED_ROW_BG));
        assert_eq!(bg(&rows[2]), None);
    }

    #[test]
    fn the_cursor_wins_over_a_mark_on_the_same_row() {
        let rows = select_rows(std::iter::once(Row::new(["x"])), Some(0), &[true], false);
        assert_eq!(bg(&rows[0]), Some(SELECTED_ROW_BG));
    }

    #[test]
    fn mark_keys_join_namespace_and_name() {
        assert_eq!(mark_key("kube-system", "coredns"), "kube-system/coredns");
        assert_eq!(mark_key("-", "node-1"), "-/node-1");
    }
}
