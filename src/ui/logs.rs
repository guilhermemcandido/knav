//! The logs popup: filtering, level colouring and match highlighting.

use super::*;

/// Log lines matching the filter, a plain case-insensitive substring, in reading order.
fn shown<'a>(lines: &'a [String], filter: &str, order: LogOrder) -> Vec<&'a str> {
    let needle = filter.to_lowercase();
    let mut out: Vec<&str> = lines.iter().map(String::as_str).filter(|l| needle.is_empty() || contains_ci(l, &needle)).collect();
    if order == LogOrder::NewestFirst {
        out.reverse();
    }
    out
}

/// The lines the logs view shows, as text, for copying.
pub fn logs_text(lines: &[String], filter: &str, order: LogOrder) -> String {
    shown(lines, filter, order).join("\n")
}

/// Case-insensitive `contains` for a lowercased `needle`, without copying ASCII lines.
fn contains_ci(hay: &str, needle: &str) -> bool {
    if hay.is_ascii() && needle.is_ascii() {
        let (h, n) = (hay.as_bytes(), needle.as_bytes());
        return n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n));
    }
    hay.to_lowercase().contains(needle)
}

/// One log line as the rows it takes at `width` cells, coloured and hard-wrapped.
fn rows_of(raw: &str, format: TimestampFormat, filter: &str, width: usize) -> Vec<Line<'static>> {
    let line = colorize_log_line(raw, format, filter);
    let width = width.max(1);
    if line.width() <= width {
        return vec![line];
    }
    let mut rows: Vec<Line<'static>> = Vec::new();
    let (mut spans, mut used): (Vec<Span<'static>>, usize) = (Vec::new(), 0);
    for span in line.spans {
        let (mut piece, mut piece_w) = (String::new(), 0);
        for ch in span.content.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + piece_w + w > width {
                if !piece.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut piece), span.style));
                }
                rows.push(Line::from(std::mem::take(&mut spans)));
                (used, piece_w) = (0, 0);
            }
            piece.push(ch);
            piece_w += w;
        }
        if !piece.is_empty() {
            spans.push(Span::styled(piece, span.style));
        }
        used += piece_w;
    }
    rows.push(Line::from(spans));
    rows
}

fn text_area(frame_area: Rect) -> Rect {
    let area = body_area(frame_area, true);
    Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) }
}

/// How many of the last lines of `ordered` fit on `height` rows.
fn lines_fitting_at_end(ordered: &[&str], format: TimestampFormat, filter: &str, width: usize, height: usize) -> usize {
    let (mut rows, mut count) = (0, 0);
    for raw in ordered.iter().rev() {
        rows += rows_of(raw, format, filter, width).len();
        if rows > height && count > 0 {
            break;
        }
        count += 1;
    }
    count
}

pub(super) fn draw_logs_popup(frame: &mut Frame, view: LogsView) {
    let LogsView { title, lines, scroll, follow, timestamp_format, order, filter, filter_editing } = view;
    // The whole body width, so selecting text with the mouse never takes in what is behind.
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let ordered = shown(lines, filter, order);

    // Just the live state; the keys are in `?`. The arrow points the way the log reads.
    let direction = if order == LogOrder::OldestFirst { "↓" } else { "↑" };
    let follow_status = if follow { "following" } else { "scrolled" };
    let count = if filter.is_empty() { format!("{} lines", lines.len()) } else { format!("{}/{} lines", ordered.len(), lines.len()) };
    let mut title_line = pill_title(title, false, Style::default());
    title_line.push_span(Span::raw(format!("  {direction}  {follow_status}  {count} ")));
    let block = with_search(Block::default().borders(Borders::ALL).border_set(border_set()).title(title_line), filter, filter_editing, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Only the rows on screen are built. Following shows the newest end; scrolled,
    // the view starts at line `scroll`.
    let (width, height) = (usize::from(inner.width), usize::from(inner.height));
    let mut rows: Vec<Line<'static>> = Vec::new();
    if follow && order == LogOrder::OldestFirst {
        for raw in ordered.iter().rev() {
            let mut own = rows_of(raw, timestamp_format, filter, width);
            own.append(&mut rows);
            rows = own;
            if rows.len() >= height {
                break;
            }
        }
        rows.drain(..rows.len().saturating_sub(height));
    } else {
        let start = if follow { 0 } else { scroll.min(ordered.len().saturating_sub(1)) };
        for raw in ordered.iter().skip(start) {
            rows.extend(rows_of(raw, timestamp_format, filter, width));
            if rows.len() >= height {
                break;
            }
        }
        rows.truncate(height);
    }
    frame.render_widget(Paragraph::new(rows), inner);
}

/// The furthest a paused view can scroll: the first line after which everything still
/// fits. Scrolling past it resumes following.
fn logs_max_scroll(frame_area: Rect, lines: &[String], filter: &str, format: TimestampFormat, order: LogOrder) -> usize {
    let inner = text_area(frame_area);
    let ordered = shown(lines, filter, order);
    ordered.len().saturating_sub(lines_fitting_at_end(&ordered, format, filter, usize::from(inner.width), usize::from(inner.height)))
}

/// Moves up one line. Oldest-first, that pauses on older lines; newest-first, it heads
/// to the newest line and resumes following at the top.
pub fn logs_scroll_up(frame_area: Rect, lines: &[String], filter: &str, format: TimestampFormat, order: LogOrder, follow: &mut bool, scroll: &mut usize) {
    match order {
        LogOrder::OldestFirst => {
            if *follow {
                *scroll = logs_max_scroll(frame_area, lines, filter, format, order);
                *follow = false;
            }
            *scroll = scroll.saturating_sub(1);
        }
        LogOrder::NewestFirst => {
            if *follow {
                return;
            }
            *scroll = scroll.saturating_sub(1);
            if *scroll == 0 {
                *follow = true;
            }
        }
    }
}

/// The opposite move. Oldest-first, down resumes following at the end like `tail -f`;
/// newest-first, it pauses.
pub fn logs_scroll_down(frame_area: Rect, lines: &[String], filter: &str, format: TimestampFormat, order: LogOrder, follow: &mut bool, scroll: &mut usize) {
    match order {
        LogOrder::OldestFirst => {
            if *follow {
                return;
            }
            *scroll = scroll.saturating_add(1);
            if *scroll >= logs_max_scroll(frame_area, lines, filter, format, order) {
                *follow = true;
            }
        }
        LogOrder::NewestFirst => {
            if *follow {
                *scroll = 0;
                *follow = false;
            }
            *scroll = scroll.saturating_add(1).min(logs_max_scroll(frame_area, lines, filter, format, order));
        }
    }
}

/// Colours one log line: the timestamp in cyan, the message red for error words,
/// yellow for warnings, else grey. A guess, since stdout and stderr arrive merged.
pub(super) fn colorize_log_line(raw: &str, timestamp_format: TimestampFormat, filter: &str) -> Line<'static> {
    let (timestamp, rest) = match raw.split_once(' ') {
        Some((ts, rest)) if looks_like_timestamp(ts) => (Some(ts), rest),
        _ => (None, raw),
    };
    // An aggregated view tags lines `\u{200B}[pod/container] ` after the timestamp. The
    // zero-width space never appears in real logs, so an app's own `[INFO]` isn't misread.
    let (tag, message) = match rest.strip_prefix('\u{200B}').and_then(|r| r.strip_prefix('[')).and_then(|r| r.split_once("] ")) {
        Some((tag, message)) => (Some(tag), message),
        None => (None, rest),
    };

    let lower = message.to_ascii_lowercase();
    let level_color = if ["error", "fatal", "panic", "fail"].iter().any(|kw| lower.contains(kw)) {
        theme().bad
    } else if lower.contains("warn") {
        theme().warn
    } else {
        theme().text_soft
    };

    let mut spans = Vec::new();
    if let Some(ts) = timestamp {
        let display = match timestamp_format {
            TimestampFormat::Short => short_timestamp(ts),
            TimestampFormat::Full => ts.to_string(),
        };
        spans.push(Span::styled(format!("[{display}] "), Style::default().fg(theme().namespace)));
    }
    if let Some(tag) = tag {
        spans.push(Span::styled(format!("[{tag}] "), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)));
    }
    spans.extend(highlight_matches(message, filter, Style::default().fg(level_color)));
    Line::from(spans)
}

/// Splits `text` around each case-insensitive match of `needle` and highlights it,
/// matching by character so case changes that alter length still cut cleanly.
pub(super) fn highlight_matches(text: &str, needle: &str, base_style: Style) -> Vec<Span<'static>> {
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }
    // Each character with its lowercase form; a match is a run of whole characters.
    let chars: Vec<(usize, char, Vec<char>)> = text.char_indices().map(|(i, c)| (i, c, c.to_lowercase().collect())).collect();
    let mut spans = Vec::new();
    let (mut at, mut plain_from) = (0, 0);
    while at < chars.len() {
        // Try to consume `needle` from `at`, character by character.
        let (mut n, mut end) = (0, at);
        while end < chars.len() && n < needle.len() && chars[end].2.iter().enumerate().all(|(k, c)| needle.get(n + k) == Some(c)) {
            n += chars[end].2.len();
            end += 1;
        }
        if n == needle.len() {
            let (from, to) = (chars[at].0, chars.get(end).map_or(text.len(), |c| c.0));
            if from > plain_from {
                spans.push(Span::styled(text[plain_from..from].to_string(), base_style));
            }
            spans.push(Span::styled(text[from..to].to_string(), match_style()));
            plain_from = to;
            at = end;
        } else {
            at += 1;
        }
    }
    if plain_from < text.len() {
        spans.push(Span::styled(text[plain_from..].to_string(), base_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }
    spans
}

/// `2026-09-16T18:36:38.477289255Z` becomes `18:36:38.477`: no date, milliseconds only.
pub(super) fn short_timestamp(ts: &str) -> String {
    let time_part = ts.split('T').nth(1).unwrap_or(ts).trim_end_matches('Z');
    match time_part.split_once('.') {
        Some((secs, frac)) => format!("{secs}.{}", &frac[..frac.len().min(3)]),
        None => time_part.to_string(),
    }
}

/// A quick shape check for the RFC3339 timestamp the server adds, so lines without
/// one (like our own errors) aren't misread.
pub(super) fn looks_like_timestamp(s: &str) -> bool {
    s.len() >= 20 && s.as_bytes().get(4) == Some(&b'-') && s.contains('T') && s.ends_with('Z')
}

#[cfg(test)]
mod log_color_tests {
    use super::*;

    #[test]
    fn timestamped_error_line_splits_and_colors_red() {
        let line = colorize_log_line(
            "2026-09-16T18:36:38.477289255Z connection refused: ERROR dialing upstream",
            TimestampFormat::Full,
            ""
        );
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content, "[2026-09-16T18:36:38.477289255Z] ");
        assert_eq!(line.spans[0].style.fg, Some(theme().namespace));
        assert_eq!(line.spans[1].style.fg, Some(theme().bad));
    }

    #[test]
    fn short_format_truncates_to_millisecond_time_of_day() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z line 0", TimestampFormat::Short, "");
        assert_eq!(line.spans[0].content, "[18:36:38.477] ");
    }

    #[test]
    fn warning_line_colors_yellow() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z WARN: retrying in 5s", TimestampFormat::Full, "");
        assert_eq!(line.spans[1].style.fg, Some(theme().warn));
    }

    #[test]
    fn plain_line_colors_gray() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z line 0", TimestampFormat::Full, "");
        assert_eq!(line.spans[1].style.fg, Some(theme().text_soft));
    }

    #[test]
    fn filter_match_is_highlighted_case_insensitively() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z hello World", TimestampFormat::Full, "world");
        let hl: Vec<_> = line.spans.iter().filter(|s| s.style.bg == Some(theme().highlight)).collect();
        assert_eq!(hl.len(), 1);
        assert_eq!(hl[0].content, "World");
    }

    #[test]
    fn line_without_timestamp_has_no_timestamp_span() {
        let line = colorize_log_line("[failed to start log stream: connection reset]", TimestampFormat::Short, "");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].style.fg, Some(theme().bad)); // "failed" matches
    }

    #[test]
    fn an_aggregated_lines_tag_gets_its_own_span() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z \u{200B}[web-x/app] line 0", TimestampFormat::Short, "");
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content, "[18:36:38.477] ");
        assert_eq!(line.spans[1].content, "[web-x/app] ");
        assert_eq!(line.spans[2].content, "line 0");
    }

    #[test]
    fn a_real_line_starting_with_brackets_is_not_mistaken_for_a_tag() {
        // Without the zero-width space, an app's own `[INFO]` prefix stays in the message.
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z [INFO] starting up", TimestampFormat::Full, "");
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[1].content, "[INFO] starting up");
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line {i}")).collect()
    }

    const FMT: TimestampFormat = TimestampFormat::Short;
    const AREA: Rect = Rect { x: 0, y: 0, width: 100, height: 30 };

    #[test]
    fn scrolling_down_while_following_keeps_following() {
        let (mut follow, mut scroll) = (true, 0usize);
        logs_scroll_down(AREA, &lines(500), "", FMT, LogOrder::OldestFirst, &mut follow, &mut scroll);
        assert!(follow);
    }

    #[test]
    fn scrolling_up_leaves_follow_from_the_tail_not_the_top() {
        let (mut follow, mut scroll) = (true, 0usize);
        let l = lines(500);
        logs_scroll_up(AREA, &l, "", FMT, LogOrder::OldestFirst, &mut follow, &mut scroll);
        assert!(!follow);
        assert_eq!(scroll, logs_max_scroll(AREA, &l, "", FMT, LogOrder::OldestFirst) - 1);
    }

    #[test]
    fn scrolling_back_to_the_end_resumes_following() {
        let (mut follow, mut scroll) = (true, 0usize);
        let l = lines(500);
        logs_scroll_up(AREA, &l, "", FMT, LogOrder::OldestFirst, &mut follow, &mut scroll);
        logs_scroll_down(AREA, &l, "", FMT, LogOrder::OldestFirst, &mut follow, &mut scroll);
        assert!(follow);
    }

    #[test]
    fn newest_first_scrolling_down_leaves_follow_and_up_at_the_top_resumes_it() {
        let (mut follow, mut scroll) = (true, 0usize);
        let l = lines(500);
        logs_scroll_up(AREA, &l, "", FMT, LogOrder::NewestFirst, &mut follow, &mut scroll);
        assert!(follow, "up while at the newest line does nothing");
        logs_scroll_down(AREA, &l, "", FMT, LogOrder::NewestFirst, &mut follow, &mut scroll);
        assert!(!follow);
        assert_eq!(scroll, 1);
        logs_scroll_up(AREA, &l, "", FMT, LogOrder::NewestFirst, &mut follow, &mut scroll);
        assert!(follow, "back at the top: following again");
    }

    #[test]
    fn newest_first_scrolling_stops_at_the_oldest_line() {
        let (mut follow, mut scroll) = (true, 0usize);
        let l = lines(100);
        for _ in 0..1000 {
            logs_scroll_down(AREA, &l, "", FMT, LogOrder::NewestFirst, &mut follow, &mut scroll);
        }
        assert_eq!(scroll, logs_max_scroll(AREA, &l, "", FMT, LogOrder::OldestFirst));
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    #[test]
    fn a_long_line_becomes_rows_no_wider_than_the_screen() {
        let rows = rows_of(&"x".repeat(25), TimestampFormat::Short, "", 10);
        assert_eq!(rows.iter().map(Line::width).collect::<Vec<_>>(), [10, 10, 5]);
    }

    #[test]
    fn wide_characters_count_two_cells() {
        let rows = rows_of("日本語日本語", TimestampFormat::Short, "", 6);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.width() <= 6));
    }

    #[test]
    fn the_last_lines_that_fit_count_their_wrapped_rows() {
        let long = "y".repeat(30);
        let ordered = vec!["a", "b", long.as_str(), "c"];
        // At width 10, "c" takes 1 row and the long line 3, so 4 rows fit those two.
        assert_eq!(lines_fitting_at_end(&ordered, TimestampFormat::Short, "", 10, 4), 2);
    }

    #[test]
    fn highlighting_survives_text_whose_lowercase_form_is_longer() {
        let spans = highlight_matches("İstanbul ERROR here", "error", Style::default());
        let hit: Vec<_> = spans.iter().filter(|s| s.style == match_style()).map(|s| s.content.to_string()).collect();
        assert_eq!(hit, ["ERROR"]);
        assert_eq!(spans.iter().map(|s| s.content.to_string()).collect::<String>(), "İstanbul ERROR here");
    }
}
