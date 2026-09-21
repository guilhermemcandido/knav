//! The logs popup: filtering, level colouring and match highlighting.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_logs_popup(
    frame: &mut Frame,
    title: &str,
    lines: &[String],
    scroll: u16,
    follow: bool,
    timestamp_format: TimestampFormat,
    order: LogOrder,
    filter: &str,
    filter_editing: bool,
) {
    let area = centered_rect(90, 90, frame.area());
    frame.render_widget(Clear, area);

    // A plain substring match, not the fuzzy scorer the rest of the app
    // uses, log lines are prose to scan, not identifiers to narrow.
    let needle = filter.to_lowercase();
    let mut filtered: Vec<&str> =
        if filter.is_empty() { lines.iter().map(String::as_str).collect() } else { lines.iter().map(String::as_str).filter(|l| l.to_lowercase().contains(&needle)).collect() };

    // Just the live state, not how to control it, the keybindings for
    // pausing/resuming/toggling timestamps live in the `?` commands
    // panel now instead of being spelled out here every time.
    let follow_status = if follow { "following" } else { "scrolled" };
    let count = if filter.is_empty() { format!("{} lines", lines.len()) } else { format!("{}/{} lines", filtered.len(), lines.len()) };
    let mut title_line = colored_slash_title(title);
    title_line.push_span(Span::raw(format!("  {follow_status}  {count}")));
    if let Some(span) = search_span(filter, filter_editing, false) {
        title_line.push_span(span);
    }
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).title(title_line);

    // Newest-first is the same log read from the other end.
    if order == LogOrder::NewestFirst {
        filtered.reverse();
    }

    // When following, show exactly the newest lines that fit: the tail when
    // oldest-first, the head when newest-first.
    let (text, effective_scroll): (Vec<Line>, u16) = if follow {
        let visible = area.height.saturating_sub(2) as usize; // minus borders
        let shown = match order {
            LogOrder::OldestFirst => &filtered[filtered.len().saturating_sub(visible)..],
            LogOrder::NewestFirst => &filtered[..filtered.len().min(visible)],
        };
        (shown.iter().copied().map(|l| colorize_log_line(l, timestamp_format, filter)).collect(), 0)
    } else {
        (filtered.iter().copied().map(|l| colorize_log_line(l, timestamp_format, filter)).collect(), scroll)
    };

    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false }).scroll((effective_scroll, 0));

    frame.render_widget(paragraph, area);

}

/// The furthest a non-following view can scroll: everything past the
/// last screenful. Also where scrolling down hands back to following.
fn logs_max_scroll(frame_area: Rect, lines: &[String], filter: &str) -> u16 {
    let needle = filter.to_lowercase();
    let shown = if filter.is_empty() { lines.len() } else { lines.iter().filter(|l| l.to_lowercase().contains(&needle)).count() };
    let visible = centered_rect(90, 90, frame_area).height.saturating_sub(2) as usize;
    shown.saturating_sub(visible).min(u16::MAX as usize) as u16
}

/// Moves the view one line up. Oldest-first, up goes to older lines and leaves
/// following from where the tail was. Newest-first, up goes toward the newest line
/// and resumes following at the top.
pub fn logs_scroll_up(frame_area: Rect, lines: &[String], filter: &str, order: LogOrder, follow: &mut bool, scroll: &mut u16) {
    match order {
        LogOrder::OldestFirst => {
            if *follow {
                *scroll = logs_max_scroll(frame_area, lines, filter);
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

/// The opposite move. Oldest-first, down goes toward the newest line and resumes
/// following at the end, like `tail -f`. Newest-first, down leaves following.
pub fn logs_scroll_down(frame_area: Rect, lines: &[String], filter: &str, order: LogOrder, follow: &mut bool, scroll: &mut u16) {
    match order {
        LogOrder::OldestFirst => {
            if *follow {
                return;
            }
            *scroll = scroll.saturating_add(1);
            if *scroll >= logs_max_scroll(frame_area, lines, filter) {
                *follow = true;
            }
        }
        LogOrder::NewestFirst => {
            if *follow {
                *scroll = 0;
                *follow = false;
            }
            *scroll = scroll.saturating_add(1).min(logs_max_scroll(frame_area, lines, filter));
        }
    }
}

/// Colours one log line: the server timestamp in cyan brackets, the message red for
/// error/fatal/panic/fail, yellow for warn, else gray. A substring guess, since the
/// API merges stdout and stderr.
pub(super) fn colorize_log_line(raw: &str, timestamp_format: TimestampFormat, filter: &str) -> Line<'static> {
    let (timestamp, message) = match raw.split_once(' ') {
        Some((ts, rest)) if looks_like_timestamp(ts) => (Some(ts), rest),
        _ => (None, raw),
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
    spans.extend(highlight_matches(message, filter, Style::default().fg(level_color)));
    Line::from(spans)
}

/// Splits `text` around each case-insensitive match of `needle` and highlights it.
/// An empty `needle` means no filter, so the text keeps `base_style`.
pub(super) fn highlight_matches(text: &str, needle: &str, base_style: Style) -> Vec<Span<'static>> {
    if needle.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }
    let highlight_style = match_style();
    let lower_text = text.to_lowercase();
    let lower_needle = needle.to_lowercase();
    // Lowercasing can change byte lengths for some Unicode; byte offsets
    // wouldn't line up with `text`, so skip highlighting rather than panic.
    if lower_text.len() != text.len() || lower_needle.len() != needle.len() {
        return vec![Span::styled(text.to_string(), base_style)];
    }
    let mut spans = Vec::new();
    let mut rest = text;
    let mut rest_lower = lower_text.as_str();
    let mut consumed = 0;
    while let Some(pos) = rest_lower.find(&lower_needle) {
        if pos > 0 {
            spans.push(Span::styled(rest[..pos].to_string(), base_style));
        }
        spans.push(Span::styled(rest[pos..pos + needle.len()].to_string(), highlight_style));
        consumed += pos + needle.len();
        rest = &text[consumed..];
        rest_lower = &lower_text[consumed..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), base_style));
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

/// Cheap shape check for the RFC3339 timestamp `timestamps: true` adds, so lines
/// without one (like our own `[failed to start log stream: ...]`) aren't misread.
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
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line {i}")).collect()
    }

    const AREA: Rect = Rect { x: 0, y: 0, width: 100, height: 30 };

    #[test]
    fn scrolling_down_while_following_keeps_following() {
        let (mut follow, mut scroll) = (true, 0);
        logs_scroll_down(AREA, &lines(500), "", LogOrder::OldestFirst, &mut follow, &mut scroll);
        assert!(follow);
    }

    #[test]
    fn scrolling_up_leaves_follow_from_the_tail_not_the_top() {
        let (mut follow, mut scroll) = (true, 0);
        let l = lines(500);
        logs_scroll_up(AREA, &l, "", LogOrder::OldestFirst, &mut follow, &mut scroll);
        assert!(!follow);
        assert_eq!(scroll, logs_max_scroll(AREA, &l, "") - 1);
    }

    #[test]
    fn scrolling_back_to_the_end_resumes_following() {
        let (mut follow, mut scroll) = (true, 0);
        let l = lines(500);
        logs_scroll_up(AREA, &l, "", LogOrder::OldestFirst, &mut follow, &mut scroll);
        logs_scroll_down(AREA, &l, "", LogOrder::OldestFirst, &mut follow, &mut scroll);
        assert!(follow);
    }

    #[test]
    fn newest_first_scrolling_down_leaves_follow_and_up_at_the_top_resumes_it() {
        let (mut follow, mut scroll) = (true, 0);
        let l = lines(500);
        logs_scroll_up(AREA, &l, "", LogOrder::NewestFirst, &mut follow, &mut scroll);
        assert!(follow, "up while at the newest line does nothing");
        logs_scroll_down(AREA, &l, "", LogOrder::NewestFirst, &mut follow, &mut scroll);
        assert!(!follow);
        assert_eq!(scroll, 1);
        logs_scroll_up(AREA, &l, "", LogOrder::NewestFirst, &mut follow, &mut scroll);
        assert!(follow, "back at the top: following again");
    }

    #[test]
    fn newest_first_scrolling_stops_at_the_oldest_line() {
        let (mut follow, mut scroll) = (true, 0);
        let l = lines(100);
        for _ in 0..1000 {
            logs_scroll_down(AREA, &l, "", LogOrder::NewestFirst, &mut follow, &mut scroll);
        }
        assert_eq!(scroll, logs_max_scroll(AREA, &l, ""));
    }
}
