//! The readable object summary: sections of labelled values, wrapped to the width.

use super::*;
use crate::k8s::details::{Chunk, Line as DLine, Section, Style as DStyle};

const LABEL_W: usize = 18;

fn style_of(style: DStyle) -> Style {
    match style {
        DStyle::Plain => Style::default(),
        DStyle::Strong => Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD),
        DStyle::Muted => Style::default().fg(theme().muted),
        DStyle::Good => Style::default().fg(theme().ok),
        DStyle::Warn => Style::default().fg(theme().warn),
        DStyle::Bad => Style::default().fg(theme().bad),
        DStyle::Chip => Style::default().fg(theme().text_strong).bg(theme().pill_bg),
    }
}

/// A run of chunks starting after `indent` cells, wrapping under that indent.
fn flow(first_prefix: Vec<Span<'static>>, indent: usize, chunks: &[Chunk], width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans = first_prefix;
    let mut used = indent;
    let mut previous_chip = false;
    for chunk in chunks {
        let chip = chunk.style == DStyle::Chip;
        let text = if chip { format!(" {} ", chunk.text) } else { chunk.text.clone() };
        let gap = usize::from(chip && previous_chip);
        let w = text.chars().count();
        if used + gap + w > width && used > indent {
            lines.push(Line::from(std::mem::take(&mut spans)));
            spans.push(Span::raw(" ".repeat(indent)));
            used = indent;
        } else if gap > 0 {
            spans.push(Span::raw(" "));
            used += 1;
        }
        spans.push(Span::styled(text, style_of(chunk.style)));
        used += w;
        previous_chip = chip;
    }
    lines.push(Line::from(spans));
    lines
}

/// Every line of the summary at `width` columns.
pub(super) fn details_lines(sections: &[Section], width: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            out.push(Line::raw(""));
        }
        out.push(Line::styled(section.title.clone(), Style::default().fg(theme().heading).add_modifier(Modifier::BOLD)));
        for line in &section.lines {
            match line {
                DLine::Blank => out.push(Line::raw("")),
                DLine::Field(label, chunks) => {
                    let prefix = vec![Span::styled(format!("  {label:<w$}", w = LABEL_W), Style::default().fg(theme().muted))];
                    out.extend(flow(prefix, LABEL_W + 2, chunks, width));
                }
                DLine::Item(chunks) => out.extend(flow(vec![Span::raw("  ")], 2, chunks, width)),
                DLine::Sub(label, chunks) => {
                    let prefix = vec![Span::styled(format!("      {label:<10}"), Style::default().fg(theme().muted))];
                    out.extend(flow(prefix, 16, chunks, width));
                }
            }
        }
    }
    out
}

/// How many lines the summary takes in a screen of `frame_area`.
pub fn details_line_count(sections: &[Section], frame_area: Rect) -> usize {
    let inner = Block::default().borders(Borders::ALL).inner(body_area(frame_area, true));
    details_lines(sections, usize::from(inner.width)).len()
}

pub(super) fn draw_details(frame: &mut Frame, title: &str, sections: &[Section], scroll: usize) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(Line::styled(format!(" {title} "), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)).centered())
        .title_bottom(Line::styled(" ↑↓ scroll   y yaml   esc close ", Style::default().fg(theme().muted)).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let padded = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let lines = details_lines(sections, usize::from(padded.width));
    let scroll = scroll.min(lines.len().saturating_sub(usize::from(padded.height)));
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), padded);
}
