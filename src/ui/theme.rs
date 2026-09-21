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

pub(super) fn theme_highlight(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().bg(SELECT_BG).fg(Color::Black).add_modifier(Modifier::BOLD) }
}

pub(super) fn theme_border(dimmed: bool) -> Style {
    if dimmed { dim_style() } else { Style::default().fg(BORDER_FG) }
}

/// k9s-style table title: the kind as a filled pill, the count in orange.
pub(super) fn table_title(label: &str, count: usize, search: Search, dimmed: bool) -> Line<'static> {
    if dimmed {
        return Line::styled(format!(" {label} ({count}) "), dim_style());
    }
    let mut spans = vec![
        Span::styled(format!(" {label} "), Style::default().bg(Color::Rgb(50, 56, 72)).fg(Color::Rgb(226, 232, 240)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("({count})"), Style::default().fg(Color::Rgb(240, 160, 110)).add_modifier(Modifier::BOLD)),
    ];
    spans.extend(search_span(search.text, search.editing, false));
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

/// A table header where every column carries its number, `(1)NAME`, and
/// the sorted column an arrow, `(2)AGE ▲` (ascending) / `▼` (descending).
/// The numbers light up while the sort key is being chosen (`s`).
pub(super) fn header_row(names: &[&str], sort: SortState, dimmed: bool) -> Row<'static> {
    let text = theme_header(dimmed);
    let number = if dimmed {
        dim_style()
    } else if sort.choosing {
        Style::default().fg(Color::Rgb(240, 160, 110)).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(96, 125, 139))
    };
    let cells = names.iter().enumerate().map(|(i, name)| {
        let mut spans = vec![Span::styled(format!("({})", i + 1), number), Span::styled((*name).to_string(), text)];
        if sort.column == Some(i) {
            spans.push(Span::styled(if sort.descending { " ▼" } else { " ▲" }, text));
        }
        Cell::from(Line::from(spans))
    });
    Row::new(cells)
}
