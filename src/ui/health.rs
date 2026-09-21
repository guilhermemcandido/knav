//! Health bars and legends for the Overview cards.

use super::*;
use crate::k8s::Health;

/// A bar `width` cells wide: green for what is fine, yellow for what needs a
/// look, red for what is broken, the rest muted.
pub(super) fn health_bar(health: Health, total: usize, width: usize, dimmed: bool) -> Line<'static> {
    let paint = |color: Color| if dimmed { dim_style() } else { Style::default().fg(color) };
    let bar = width.max(1);
    if total == 0 {
        return Line::styled("░".repeat(bar), paint(theme().muted));
    }
    // Each state gets its share of the bar (largest remainders, so the bar is
    // always full), and never disappears if it exists. The neutral rest is what
    // is neither ok, warning nor error (finished, nothing wanted).
    let counts = [health.good, health.warn, health.bad, total.saturating_sub(health.good + health.warn + health.bad)];
    let sum: usize = counts.iter().sum();
    let mut cells = counts.map(|n| n * bar / sum);
    let mut order: Vec<usize> = (0..4).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(counts[i] * bar % sum));
    for &i in order.iter().cycle().take(bar - cells.iter().sum::<usize>()) {
        cells[i] += 1;
    }
    for i in [2, 1, 0] {
        if counts[i] > 0 && cells[i] == 0 {
            let biggest = (0..4).max_by_key(|&j| cells[j]).unwrap_or(0);
            cells[biggest] -= 1;
            cells[i] = 1;
        }
    }
    let [good, warn, bad, rest] = cells;
    Line::from(vec![
        Span::styled("█".repeat(good), paint(theme().ok)),
        Span::styled("█".repeat(warn), paint(theme().warn)),
        Span::styled("█".repeat(bad), paint(theme().bad)),
        Span::styled("░".repeat(rest), paint(theme().muted)),
    ])
}

/// 1234 as `1.2k`, 15000 as `15k`, so big clusters still fit.
fn compact(n: usize) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=9_999 => format!("{}.{}k", n / 1000, n % 1000 / 100),
        10_000..=999_999 => format!("{}k", n / 1000),
        _ => format!("{}M", n / 1_000_000),
    }
}

/// `● 14 ok  ● 2 warning  ● 1 error`, only the states that have objects. It gives up
/// the words, then exact numbers, to fit `width`.
pub(super) fn health_legend(health: Health, total: usize, width: usize, dimmed: bool) -> Line<'static> {
    let paint = |color: Color| if dimmed { dim_style() } else { Style::default().fg(color) };
    if total == 0 {
        return Line::styled("none", paint(theme().muted));
    }
    let groups = [(health.good, "ok", theme().ok), (health.warn, "warning", theme().warn), (health.bad, "error", theme().bad)];
    let build = |words: bool, short: bool| -> Vec<(String, Color)> {
        groups
            .iter()
            .filter(|(n, _, _)| *n > 0)
            .map(|(n, word, color)| {
                let number = if short { compact(*n) } else { n.to_string() };
                (if words { format!("● {number} {word}") } else { format!("● {number}") }, *color)
            })
            .collect()
    };
    let fits = |parts: &[(String, Color)]| parts.iter().map(|(t, _)| cell_width(t)).sum::<usize>() + 2 * parts.len().saturating_sub(1) <= width;
    let parts = [(true, false), (false, false), (true, true), (false, true)].into_iter().map(|(w, s)| build(w, s)).find(|p| fits(p)).unwrap_or_default();
    if parts.is_empty() {
        return Line::styled(format!("{} total", compact(total)), paint(theme().muted));
    }
    let mut spans = Vec::new();
    for (text, color) in parts {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(text, paint(color)));
    }
    Line::from(spans)
}

#[cfg(test)]
mod health_tests {
    use super::*;

    #[test]
    fn the_bar_fills_its_width() {
        assert_eq!(health_bar(Health { good: 15, warn: 1, bad: 1 }, 17, 20, false).width(), 20);
    }

    #[test]
    fn a_bar_of_everything_accounted_for_has_no_grey() {
        let line = health_bar(Health { good: 14, warn: 1, bad: 2 }, 17, 40, false);
        assert_eq!(line.spans[3].content.chars().count(), 0);
        assert_eq!(line.width(), 40);
    }

    #[test]
    fn a_broken_object_always_shows_even_among_many() {
        let line = health_bar(Health { good: 199, warn: 0, bad: 1 }, 200, 20, false);
        assert_eq!(line.spans[2].content.chars().count(), 1, "one red cell");
    }

    #[test]
    fn the_legend_lists_only_states_that_exist() {
        let text: String = health_legend(Health { good: 14, warn: 0, bad: 3 }, 17, 30, false).spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "● 14 ok  ● 3 error");
        assert_eq!(health_legend(Health::default(), 0, 30, false).spans[0].content, "none");
    }

    #[test]
    fn a_big_cluster_still_fits_the_card() {
        let big = Health { good: 41_250, warn: 1_800, bad: 950 };
        for width in [31, 20, 12] {
            assert!(health_legend(big, 44_000, width, false).width() <= width.max(12), "{width}");
        }
        assert_eq!(compact(1_234), "1.2k");
        assert_eq!(compact(15_000), "15k");
    }
}
