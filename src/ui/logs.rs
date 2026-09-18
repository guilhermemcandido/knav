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
    filter: &str,
    filter_editing: bool,
) {
    let area = centered_rect(90, 90, frame.area());
    frame.render_widget(Clear, area);

    // A plain substring match, not the fuzzy scorer the rest of the app
    // uses — log lines are prose to scan, not identifiers to narrow.
    let needle = filter.to_lowercase();
    let filtered: Vec<&str> =
        if filter.is_empty() { lines.iter().map(String::as_str).collect() } else { lines.iter().map(String::as_str).filter(|l| l.to_lowercase().contains(&needle)).collect() };

    // Just the live state, not how to control it — the keybindings for
    // pausing/resuming/toggling timestamps live in the `?` commands
    // panel now instead of being spelled out here every time.
    let follow_status = if follow { "following" } else { "paused" };
    let filter_status = if filter.is_empty() { String::new() } else { format!(", {}/{} match \"{filter}\"", filtered.len(), lines.len()) };
    let mut title_line = colored_slash_title(title);
    title_line.push_span(Span::raw(format!("  —  {follow_status}  ({} lines{filter_status})", lines.len())));
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(title_line);

    // When following, always show exactly the tail that fits the visible
    // area — simpler and more robust than trusting Paragraph's own scroll
    // clamping to not show blank space past the end of the content.
    let (text, effective_scroll): (Vec<Line>, u16) = if follow {
        let visible = area.height.saturating_sub(2) as usize; // minus borders
        let start = filtered.len().saturating_sub(visible);
        (filtered[start..].iter().copied().map(|l| colorize_log_line(l, timestamp_format, filter)).collect(), 0)
    } else {
        (filtered.iter().copied().map(|l| colorize_log_line(l, timestamp_format, filter)).collect(), scroll)
    };

    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false }).scroll((effective_scroll, 0));

    frame.render_widget(paragraph, area);

    // The filter's own input line, pinned just inside the bottom border
    // while actively being typed — same treatment as the `/` search box
    // elsewhere, just scoped to this popup instead of floating over it.
    if filter_editing {
        let bar = Rect { x: area.x + 1, y: area.y + area.height.saturating_sub(2), width: area.width.saturating_sub(2), height: 1 };
        frame.render_widget(Clear, bar);
        let line = Line::styled(format!("/{filter}"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        frame.render_widget(Paragraph::new(line), bar);
    }
}

/// Kubernetes' log API merges stdout/stderr into one stream and doesn't
/// preserve which one a line came from — there's no real "is this
/// stderr" signal to key off. This is the practical substitute: split
/// off the leading server-side timestamp (see `k8s::stream_logs`,
/// `timestamps: true`), bracket it and give it its own color (cyan,
/// matching the metadata/key color used in the spec tree view) so it
/// doesn't compete with gray — gray is reserved for normal-severity
/// message text. The message itself is heuristically colored by
/// scanning for error/warning keywords: substring match on
/// error/fatal/panic/fail → red, warn → yellow, else gray. It's a naive
/// heuristic, not a real log-level parser — "no errors occurred" would
/// still show red, since it's just checking for the substring "error."
/// Same approach most terminal log viewers fall back to in the absence
/// of real stream/level metadata.
pub(super) fn colorize_log_line(raw: &str, timestamp_format: TimestampFormat, filter: &str) -> Line<'static> {
    let (timestamp, message) = match raw.split_once(' ') {
        Some((ts, rest)) if looks_like_timestamp(ts) => (Some(ts), rest),
        _ => (None, raw),
    };

    let lower = message.to_ascii_lowercase();
    let level_color = if ["error", "fatal", "panic", "fail"].iter().any(|kw| lower.contains(kw)) {
        Color::Red
    } else if lower.contains("warn") {
        Color::Yellow
    } else {
        Color::Gray
    };

    let mut spans = Vec::new();
    if let Some(ts) = timestamp {
        let display = match timestamp_format {
            TimestampFormat::Short => short_timestamp(ts),
            TimestampFormat::Full => ts.to_string(),
        };
        spans.push(Span::styled(format!("[{display}] "), Style::default().fg(Color::Cyan)));
    }
    spans.extend(highlight_matches(message, filter, Style::default().fg(level_color)));
    Line::from(spans)
}

/// Splits `text` around every case-insensitive occurrence of `needle`,
/// highlighting the matched part — otherwise a live filter narrows
/// *which* lines show up but gives no indication of *where* in each one
/// it actually matched. `needle` empty means no filter is active, so
/// the whole text just gets `base_style` unchanged.
pub(super) fn highlight_matches(text: &str, needle: &str, base_style: Style) -> Vec<Span<'static>> {
    if needle.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }
    let highlight_style = Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD);
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

/// `2026-09-16T18:36:38.477289255Z` -> `18:36:38.477` — drops the date
/// (a live pod-log view is almost always "recent" logs, and if you're
/// scrolled back far enough for that to matter that's a rare edge case)
/// and truncates nanoseconds down to milliseconds, which is as much
/// precision as a human can actually use when reading logs by eye.
pub(super) fn short_timestamp(ts: &str) -> String {
    let time_part = ts.split('T').nth(1).unwrap_or(ts).trim_end_matches('Z');
    match time_part.split_once('.') {
        Some((secs, frac)) => format!("{secs}.{}", &frac[..frac.len().min(3)]),
        None => time_part.to_string(),
    }
}

/// Cheap shape check for the RFC3339 timestamp `timestamps: true` adds
/// (e.g. `2026-09-16T18:36:38.477289255Z`) — not a full parse, just
/// enough to avoid misidentifying an ordinary line that happens to have
/// an early space (like our own `[failed to start log stream: ...]`
/// messages, which have no timestamp prefix at all).
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
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(line.spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn short_format_truncates_to_millisecond_time_of_day() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z line 0", TimestampFormat::Short, "");
        assert_eq!(line.spans[0].content, "[18:36:38.477] ");
    }

    #[test]
    fn warning_line_colors_yellow() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z WARN: retrying in 5s", TimestampFormat::Full, "");
        assert_eq!(line.spans[1].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn plain_line_colors_gray() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z line 0", TimestampFormat::Full, "");
        assert_eq!(line.spans[1].style.fg, Some(Color::Gray));
    }

    #[test]
    fn filter_match_is_highlighted_case_insensitively() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z hello World", TimestampFormat::Full, "world");
        let hl: Vec<_> = line.spans.iter().filter(|s| s.style.bg == Some(Color::Yellow)).collect();
        assert_eq!(hl.len(), 1);
        assert_eq!(hl[0].content, "World");
    }

    #[test]
    fn line_without_timestamp_has_no_timestamp_span() {
        let line = colorize_log_line("[failed to start log stream: connection reset]", TimestampFormat::Short, "");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].style.fg, Some(Color::Red)); // "failed" matches
    }
}
