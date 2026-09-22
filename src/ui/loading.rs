//! The screen shown while knav opens a cluster: the wordmark and what it is waiting for.

use super::*;

const LOGO: [&str; 5] = [r" _", r"| | __ _ __    __ _ __   __", r"| |/ /| '_ \  / _` |\ \ / /", r"|   < | | | || (_| | \ V /", r"|_|\_\|_| |_| \__,_|  \_/"];

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// What the loading screen shows.
pub struct Loading<'a> {
    pub context: &'a str,
    pub version: &'a str,
    /// Each thing being waited for, and whether it has arrived.
    pub steps: &'a [(&'static str, bool)],
    /// Counts up while waiting, to turn the spinner.
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
    for (label, done) in loading.steps {
        let (mark, mark_style, text_style) = if *done {
            ("✓", Style::default().fg(theme().ok), Style::default().fg(theme().muted))
        } else {
            (SPINNER[loading.tick % SPINNER.len()], Style::default().fg(theme().warn), Style::default().fg(theme().text_strong))
        };
        lines.push(Line::from(vec![Span::raw("   "), Span::styled(mark, mark_style), Span::raw(" "), Span::styled(*label, text_style)]));
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
        let steps = [("Connected", true), ("Pods", false)];
        let text = screen(&Loading { context: "prod-eu", version: "v1.31.2", steps: &steps, tick: 0, hint: None });
        assert!(text.contains("prod-eu") && text.contains("v1.31.2"));
        assert!(text.contains("✓ Connected") && text.contains("Pods"));
        assert!(text.contains("q quits"));
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
        let steps = [("Connected", true), ("API types", true), ("Pods", false), ("Deployments", false), ("Nodes", true)];
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw_loading(frame, &Loading { context: "prod-eu-west", version: "v1.31.2-eks", steps: &steps, tick: 4, hint: Some("Still waiting for pods, deployments") })).unwrap();
        let buffer = terminal.backend().buffer();
        for y in 0..24 {
            println!("{}", (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>().trim_end());
        }
    }
}
