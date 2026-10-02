//! The screen shown while knav opens a cluster: the wordmark and what it is waiting for.

use super::*;
use std::time::Duration;

const LOGO: [&str; 5] = [r" _", r"| | __ _ __    __ _ __   __", r"| |/ /| '_ \  / _` |\ \ / /", r"|   < | | | || (_| | \ V /", r"|_|\_\|_| |_| \__,_|  \_/"];

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// One thing the loading screen waits for.
pub struct Step {
    pub label: &'static str,
    /// How long it took, once it has arrived.
    pub took: Option<Duration>,
    /// How many it found, like the number of pods.
    pub count: Option<usize>,
}

pub struct Loading<'a> {
    pub context: &'a str,
    pub version: &'a str,
    /// What it waits for, from the cluster down to its pods.
    pub steps: &'a [Step],
    /// Counts up while waiting, turning the spinner.
    pub tick: usize,
    /// Said under the steps when something is taking long.
    pub hint: Option<&'a str>,
}

pub fn draw_loading(frame: &mut Frame, loading: &Loading) {
    let full = frame.area();
    let height = (LOGO.len() + loading.steps.len() + 10) as u16;
    let area = Rect { x: full.x + full.width.saturating_sub(58) / 2, y: full.y + full.height.saturating_sub(height) / 2, width: full.width.min(58), height: full.height.min(height) };
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(theme_border(false)).title(pill_title_centered("Opening the cluster", false));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = vec![Line::raw("")];
    // Padded to one width, so the rows stay aligned when the block is centred.
    let width = LOGO.iter().map(|row| row.chars().count()).max().unwrap_or(0);
    lines.extend(LOGO.iter().map(|row| Line::styled(format!("{row:<width$}"), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)).centered()));
    lines.push(Line::raw(""));
    lines.push(Line::styled(format!("knav v{}", env!("CARGO_PKG_VERSION")), Style::default().fg(theme().muted)).centered());
    lines.push(Line::from(vec![Span::styled("context ", Style::default().fg(theme().info_label)), Span::styled(loading.context.to_string(), Style::default().fg(theme().text_strong)), Span::styled(format!("  {}", loading.version), Style::default().fg(theme().muted))]).centered());
    lines.push(Line::raw(""));
    let label_width = loading.steps.iter().map(|s| s.label.len()).max().unwrap_or(0);
    for step in loading.steps {
        let count = step.count.map(group_digits).unwrap_or_default();
        let mut spans = vec![Span::raw("   ")];
        match step.took {
            Some(took) => {
                spans.push(Span::styled("✓", Style::default().fg(theme().ok)));
                spans.push(Span::styled(format!(" {:<label_width$}", step.label), Style::default().fg(theme().row)));
                spans.push(Span::styled(format!(" {count:>8}"), Style::default().fg(theme().text_strong)));
                spans.push(Span::styled(format!("  {}", seconds(took)), Style::default().fg(theme().muted)));
            }
            None => {
                spans.push(Span::styled(SPINNER[loading.tick % SPINNER.len()], Style::default().fg(theme().warn)));
                spans.push(Span::styled(format!(" {:<label_width$}", step.label), Style::default().fg(theme().text_strong)));
                spans.push(Span::styled(format!(" {count:>8}"), Style::default().fg(theme().muted)));
            }
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    match loading.hint {
        Some(hint) => {
            lines.push(Line::styled(hint.to_string(), Style::default().fg(theme().warn)).centered());
            lines.push(Line::styled("a big cluster or a slow link takes a while (q quits)", Style::default().fg(theme().desc)).centered());
        }
        None => lines.push(Line::styled("q quits", Style::default().fg(theme().desc)).centered()),
    }
    frame.render_widget(Paragraph::new(lines), inner);
    paint_theme_base(frame);
}

/// `12345` as `12,345`.
fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn seconds(took: Duration) -> String {
    if took < Duration::from_secs(1) { format!("{}ms", took.as_millis()) } else { format!("{:.1}s", took.as_secs_f32()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn screen(loading: &Loading) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw_loading(frame, loading)).unwrap();
        terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect::<String>()
    }

    #[test]
    fn it_names_the_cluster_and_marks_finished_steps() {
        let steps = [Step { label: "Nodes", took: Some(Duration::from_millis(420)), count: Some(1204) }, Step { label: "Pods", took: None, count: None }];
        let text = screen(&Loading { context: "prod-eu", version: "v1.31.2", steps: &steps, tick: 0, hint: None });
        assert!(text.contains("prod-eu") && text.contains("v1.31.2"));
        assert!(text.contains("✓ Nodes") && text.contains("1,204") && text.contains("420ms") && text.contains("Pods"));
        assert!(text.contains("q quits"));
    }

    #[test]
    fn big_numbers_get_commas() {
        assert_eq!(group_digits(7), "7");
        assert_eq!(group_digits(1234567), "1,234,567");
    }

    #[test]
    fn a_hint_says_what_is_slow() {
        let text = screen(&Loading { context: "c", version: "v", steps: &[], tick: 3, hint: Some("still waiting for pods") });
        assert!(text.contains("still waiting for pods") && text.contains("takes a while"));
    }
}

#[cfg(test)]
mod preview {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    #[ignore]
    fn print_loading_screen() {
        let done = |label, ms, count| Step { label, took: Some(Duration::from_millis(ms)), count: Some(count) };
        let steps = [done("API types", 310, 214), done("Nodes", 480, 36), Step { label: "Deployments", took: None, count: None }, Step { label: "Pods", took: None, count: None }];
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw_loading(frame, &Loading { context: "prod-eu-west", version: "v1.31.2-eks", steps: &steps, tick: 4, hint: Some("Still waiting for pods, deployments") })).unwrap();
        let buffer = terminal.backend().buffer();
        for y in 0..24 {
            println!("{}", (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>().trim_end());
        }
    }
}
