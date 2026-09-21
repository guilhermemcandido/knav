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
        // Only pills wrap; text stays on its line and is reached by scrolling sideways.
        if chip && used + gap + w > width && used > indent {
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

/// How far the summary can scroll (down, right) in a viewport `width` by `height`.
pub fn details_extent(sections: &[Section], width: usize, height: usize) -> (usize, usize) {
    let lines = details_lines(sections, width);
    let widest = lines.iter().map(Line::width).max().unwrap_or(0);
    (lines.len().saturating_sub(height), widest.saturating_sub(width))
}

/// The most the full-screen summary can scroll on a screen of `frame_area`.
pub fn details_max_scroll(sections: &[Section], frame_area: Rect) -> (usize, usize) {
    let inner = Block::default().borders(Borders::ALL).inner(body_area(frame_area, true));
    details_extent(sections, usize::from(inner.width.saturating_sub(2)), usize::from(inner.height))
}

pub(super) fn draw_details(frame: &mut Frame, title: &str, sections: &[Section], scroll: usize, hscroll: usize) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(Line::styled(format!(" {title} "), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)).centered())
        .title_bottom(Line::styled(" ↑↓←→ scroll   y yaml   esc close ", Style::default().fg(theme().muted)).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let padded = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let lines = details_lines(sections, usize::from(padded.width));
    let scroll = scroll.min(lines.len().saturating_sub(usize::from(padded.height)));
    let hscroll = hscroll.min(lines.iter().map(Line::width).max().unwrap_or(0).saturating_sub(usize::from(padded.width)));
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, hscroll as u16)), padded);
}

/// What the side panel next to a list shows.
pub struct SidePanel {
    pub title: String,
    pub sections: Vec<Section>,
    pub scroll: usize,
    pub hscroll: usize,
    /// The keys are scrolling it.
    pub focused: bool,
}

static PANEL: std::sync::RwLock<Option<SidePanel>> = std::sync::RwLock::new(None);

/// Sets (or clears) the panel drawn beside the list.
pub fn set_side_panel(panel: Option<SidePanel>) {
    if let Ok(mut slot) = PANEL.write() {
        *slot = panel;
    }
}

/// Terminals narrower than this show the info full screen instead of beside the list.
pub const SIDE_PANEL_MIN_WIDTH: u16 = 100;

/// How wide the panel is at `full_width` columns; 0 when there is none.
pub fn side_panel_width(full_width: u16) -> u16 {
    if full_width >= SIDE_PANEL_MIN_WIDTH && PANEL.read().is_ok_and(|p| p.is_some()) { (full_width * 2 / 5).max(44) } else { 0 }
}

/// How far the panel can scroll for `sections` on a terminal of `size`.
pub fn side_panel_max_scroll(sections: &[Section], size: ratatui::layout::Size) -> (usize, usize) {
    let full = Rect { x: 0, y: 0, width: size.width, height: size.height };
    let body = body_area(full, true);
    let width = (size.width * 2 / 5).max(44).min(body.width);
    let (inner_w, inner_h) = (usize::from(width).saturating_sub(4), usize::from(body.height).saturating_sub(2));
    details_extent(sections, inner_w, inner_h)
}

/// The part of the body a list uses: all of it, or what is left of the panel.
pub fn list_body(frame_area: Rect) -> Rect {
    let body = body_area(frame_area, true);
    Rect { width: body.width - side_panel_width(frame_area.width).min(body.width), ..body }
}

/// Draws the panel, if there is one, in the right of `body`.
pub(super) fn draw_side_panel(frame: &mut Frame, body: Rect) {
    let width = side_panel_width(frame.area().width).min(body.width);
    if width == 0 {
        return;
    }
    let Ok(panel) = PANEL.read() else { return };
    let Some(panel) = panel.as_ref() else { return };
    let area = Rect { x: body.x + body.width - width, width, ..body };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(if panel.focused { Style::default().fg(theme().accent).add_modifier(Modifier::BOLD) } else { theme_border(false) })
        .title(Line::styled(format!(" {} ", panel.title), Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)))
        .title_bottom(Line::styled(if panel.focused { " ↑↓←→ scroll   enter full screen   shift-← list   i close " } else { " shift-→ focus   i close " }, Style::default().fg(theme().muted)).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let padded = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let lines = details_lines(&panel.sections, usize::from(padded.width));
    let scroll = panel.scroll.min(lines.len().saturating_sub(usize::from(padded.height)));
    let hscroll = panel.hscroll.min(lines.iter().map(Line::width).max().unwrap_or(0).saturating_sub(usize::from(padded.width)));
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, hscroll as u16)), padded);
}
