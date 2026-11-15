//! Small drawing pieces for dashboards: smooth bars, sparklines, bordered cards and a grid
//! of them. They only build styled lines, so every dashboard kind can use them.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use knav_common::theme::theme;
use knav_common::util::text::{cell_width, truncate};

/// One slice of a stacked bar.
pub struct Slice {
    pub count: usize,
    pub color: Color,
}

impl Slice {
    pub fn new(count: usize, color: Color) -> Self {
        Slice { count, color }
    }
}

fn track() -> Color {
    theme().pill_bg
}

/// A bar `width` cells wide, filled to `ratio` in eighths of a cell over a dim track.
pub fn gauge(width: usize, ratio: f64, color: Color) -> Vec<Span<'static>> {
    const PARTIAL: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let eighths = (ratio.clamp(0.0, 1.0) * (width * 8) as f64).round() as usize;
    let (full, part) = (eighths / 8, eighths % 8);
    let mut spans = Vec::new();
    if full > 0 {
        spans.push(Span::styled("█".repeat(full), Style::default().fg(color).bg(track())));
    }
    let mut used = full;
    if part > 0 && full < width {
        spans.push(Span::styled(PARTIAL[part].to_string(), Style::default().fg(color).bg(track())));
        used += 1;
    }
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), Style::default().bg(track())));
    }
    spans
}

/// Whole cells for each slice out of `width`, by largest remainder, with every non-empty
/// slice getting at least one cell while there is room.
fn cells(slices: &[Slice], width: usize) -> Vec<usize> {
    let total: usize = slices.iter().map(|s| s.count).sum();
    if total == 0 || width == 0 {
        return vec![0; slices.len()];
    }
    let exact: Vec<f64> = slices.iter().map(|s| s.count as f64 / total as f64 * width as f64).collect();
    let mut out: Vec<usize> = exact.iter().map(|e| e.floor() as usize).collect();
    for (i, slice) in slices.iter().enumerate() {
        if slice.count > 0 && out[i] == 0 && out.iter().sum::<usize>() < width {
            out[i] = 1;
        }
    }
    let mut order: Vec<usize> = (0..slices.len()).filter(|&i| slices[i].count > 0).collect();
    order.sort_by(|&a, &b| (exact[b] - exact[b].floor()).partial_cmp(&(exact[a] - exact[a].floor())).unwrap_or(std::cmp::Ordering::Equal));
    for i in order.into_iter().cycle().take(width.saturating_sub(out.iter().sum())) {
        out[i] += 1;
    }
    out
}

/// One bar split between the slices in proportion, each in its own colour.
pub fn stacked(width: usize, slices: &[Slice]) -> Vec<Span<'static>> {
    let sizes = cells(slices, width);
    let mut spans: Vec<Span<'static>> = slices.iter().zip(&sizes).filter(|(_, n)| **n > 0).map(|(s, n)| Span::styled("█".repeat(*n), Style::default().fg(s.color).bg(track()))).collect();
    let used: usize = sizes.iter().sum();
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), Style::default().bg(track())));
    }
    spans
}

/// One block per value, as tall as the value is next to the largest.
pub fn sparkline(values: &[usize], color: Color) -> Vec<Span<'static>> {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().copied().max().unwrap_or(0);
    values
        .iter()
        .map(|&v| {
            if v == 0 || max == 0 {
                Span::styled("▁", Style::default().fg(theme().muted))
            } else {
                let level = ((v * (LEVELS.len() - 1)).div_ceil(max)).min(LEVELS.len() - 1);
                Span::styled(LEVELS[level].to_string(), Style::default().fg(color))
            }
        })
        .collect()
}

/// A legend entry: a dot, the number and its label, in the slice's colour. Entries with
/// nothing in them are left out so the line stays short.
pub fn legend(entries: &[(usize, &str, Color)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (count, label, color) in entries.iter().filter(|(count, ..)| *count > 0) {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!("● {count} {label}"), Style::default().fg(*color)));
    }
    if spans.is_empty() {
        spans.push(Span::styled("nothing yet", Style::default().fg(theme().muted)));
    }
    Line::from(spans)
}

/// Cuts a line to `width` cells, so a card's border never gets pushed out.
fn clip(line: Line<'static>, width: usize) -> Line<'static> {
    if line.width() <= width {
        return line;
    }
    let mut used = 0;
    let mut spans = Vec::new();
    for span in line.spans {
        let w = cell_width(&span.content);
        if used + w <= width {
            used += w;
            spans.push(span);
            continue;
        }
        let room = width - used;
        spans.push(Span::styled(truncate(&span.content, room), span.style));
        break;
    }
    Line::from(spans)
}

/// A rounded box `width` cells wide (borders included) around `body`, with `title` set in
/// the top edge.
pub fn card(title: &str, width: usize, body: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let edge = Style::default().fg(theme().muted);
    let inner = width.saturating_sub(4);
    let title = truncate(title, width.saturating_sub(6));
    let dashes = width.saturating_sub(cell_width(&title) + 5);
    let mut out = vec![Line::from(vec![
        Span::styled("╭─ ", edge),
        Span::styled(title, Style::default().fg(theme().accent).add_modifier(ratatui::style::Modifier::BOLD)),
        Span::styled(format!(" {}╮", "─".repeat(dashes)), edge),
    ])];
    for line in body {
        let line = clip(line, inner);
        let pad = inner - line.width();
        let mut spans = vec![Span::styled("│ ", edge)];
        spans.extend(line.spans);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(" │", edge));
        out.push(Line::from(spans));
    }
    out.push(Line::styled(format!("╰{}╯", "─".repeat(width.saturating_sub(2))), edge));
    out
}

/// How many cards of at least `min_width` fit in `total`, with a one-cell gap, and the
/// width each then gets.
pub fn columns(total: usize, min_width: usize) -> (usize, usize) {
    let cols = ((total + 1) / (min_width + 1)).max(1);
    let each = (total.saturating_sub(cols - 1) / cols).max(1);
    (cols, each)
}

/// Lays cards out in rows of equal-width columns. `build` gets a card's full width and
/// returns its lines.
pub fn grid<T>(total: usize, min_width: usize, items: &[T], build: impl Fn(&T, usize) -> Vec<Line<'static>>) -> Vec<Line<'static>> {
    let (cols, each) = columns(total, min_width);
    let mut out = Vec::new();
    for row in items.chunks(cols) {
        let cards: Vec<Vec<Line<'static>>> = row.iter().map(|item| build(item, each)).collect();
        let height = cards.iter().map(Vec::len).max().unwrap_or(0);
        for y in 0..height {
            let mut spans = Vec::new();
            for (i, card) in cards.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                match card.get(y) {
                    Some(line) => spans.extend(line.spans.clone()),
                    None => spans.push(Span::raw(" ".repeat(each))),
                }
            }
            out.push(Line::from(spans));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn a_gauge_is_always_its_width() {
        for ratio in [0.0, 0.03, 0.5, 0.86, 1.0, 2.0] {
            assert_eq!(cell_width(&text(&gauge(20, ratio, Color::Green))), 20, "{ratio}");
        }
    }

    #[test]
    fn a_gauge_fills_in_part_cells() {
        let spans = gauge(10, 0.55, Color::Green);
        assert!(text(&spans).starts_with("█████▌"), "{}", text(&spans));
    }

    #[test]
    fn stacked_fills_the_width_and_keeps_small_slices_visible() {
        let slices = [Slice::new(1000, Color::Green), Slice::new(1, Color::Red)];
        let spans = stacked(20, &slices);
        assert_eq!(cell_width(&text(&spans)), 20);
        assert!(spans.iter().any(|s| s.style.fg == Some(Color::Red)));
    }

    #[test]
    fn an_empty_stacked_bar_is_all_track() {
        assert_eq!(text(&stacked(8, &[Slice::new(0, Color::Green)])), " ".repeat(8));
    }

    #[test]
    fn a_sparkline_scales_to_its_largest_value() {
        let line = text(&sparkline(&[0, 1, 4, 8], Color::Green));
        assert_eq!(line.chars().count(), 4);
        assert!(line.ends_with('█') && line.starts_with('▁'));
    }

    #[test]
    fn a_card_is_as_wide_as_asked_even_with_long_lines() {
        let body = vec![Line::from("short"), Line::from("a line that is much too long for this box")];
        for line in card("Title", 24, body) {
            assert_eq!(line.width(), 24, "{:?}", text(&line.spans));
        }
    }

    #[test]
    fn the_grid_fits_columns_in_the_width() {
        assert_eq!(columns(80, 30), (2, 39));
        assert_eq!(columns(50, 30), (1, 50));
        assert_eq!(columns(200, 30), (6, 32));
    }

    #[test]
    fn grid_rows_are_the_same_width() {
        let lines = grid(61, 30, &[1, 2, 3], |n, w| card(&format!("c{n}"), w, vec![Line::from("x"); *n]));
        assert!(lines.iter().all(|l| l.width() <= 61));
    }
}
