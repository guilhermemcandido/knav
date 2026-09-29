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
        DStyle::WarnChip => Style::default().fg(theme().warn).bg(theme().pill_bg),
        DStyle::PairChip => Style::default().fg(theme().text_strong).bg(theme().pill_bg),
        DStyle::Key => Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD),
    }
}

/// `text` cut into `width`-cell pieces on whole characters.
fn wrap_at(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut pieces = Vec::new();
    let (mut piece, mut used) = (String::new(), 0);
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width && !piece.is_empty() {
            pieces.push(std::mem::take(&mut piece));
            used = 0;
        }
        piece.push(ch);
        used += w;
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
    pieces
}

/// A run of chunks starting after `indent` cells, wrapping under that indent.
fn flow(first_prefix: Vec<Span<'static>>, indent: usize, chunks: &[Chunk], width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans = first_prefix;
    let mut used = indent;
    let mut previous_chip = false;
    // A pill wider than a whole line (a long annotation value) wraps instead of
    // running off the edge; its background still reads as one pill.
    let usable = width.saturating_sub(indent).max(1);
    for chunk in chunks {
        let chip = chunk.style.is_chip();
        let text = if chip { format!(" {} ", chunk.text) } else { chunk.text.clone() };
        let gap = usize::from(chip && previous_chip);
        let w = cell_width(&text);
        if chip && w > usable {
            if used > indent {
                lines.push(Line::from(std::mem::take(&mut spans)));
            }
            for piece in wrap_at(&text, usable) {
                lines.push(Line::from(vec![Span::raw(" ".repeat(indent)), Span::styled(piece, style_of(chunk.style))]));
            }
            spans = vec![Span::raw(" ".repeat(indent))];
            used = indent;
            previous_chip = false;
            continue;
        }
        // Only pills wrap; text stays on its line and is reached by scrolling sideways.
        if chip && used + gap + w > width && used > indent {
            lines.push(Line::from(std::mem::take(&mut spans)));
            spans.push(Span::raw(" ".repeat(indent)));
            used = indent;
        } else if gap > 0 {
            spans.push(Span::raw(" "));
            used += 1;
        }
        if chunk.style == DStyle::PairChip
            && let Some((key, value)) = chunk.text.split_once('=')
        {
            // key=value: key and value in their own colours on one pill.
            let pill = Style::default().bg(theme().pill_bg);
            spans.push(Span::styled(format!(" {key}"), pill.fg(theme().namespace)));
            spans.push(Span::styled("=", pill.fg(theme().muted)));
            spans.push(Span::styled(format!("{value} "), pill.fg(theme().text_strong)));
        } else {
            spans.push(Span::styled(text, style_of(chunk.style)));
        }
        used += w;
        previous_chip = chip;
    }
    // Skip a last line that is only the indent, left behind by an oversized pill.
    if used > indent || lines.is_empty() {
        lines.push(Line::from(spans));
    }
    lines
}

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
                DLine::Pad(indent, chunks) => out.extend(flow(vec![Span::raw(" ".repeat(*indent))], *indent, chunks, width)),
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
        .title(pill_title(title, false, theme_border(false)))
        .title_bottom(hint_strip(&[("g/G", "top/bottom"), ("enter", "open list"), ("y", "yaml"), ("q/esc", "close")]).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let padded = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let lines = details_lines(sections, usize::from(padded.width));
    let scroll = scroll.min(lines.len().saturating_sub(usize::from(padded.height)));
    let hscroll = hscroll.min(lines.iter().map(Line::width).max().unwrap_or(0).saturating_sub(usize::from(padded.width)));
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, hscroll as u16)), padded);
}

pub struct SidePanel {
    pub title: String,
    pub sections: Vec<Section>,
    pub scroll: usize,
    pub hscroll: usize,
    /// The keys are scrolling it.
    pub focused: bool,
}


/// Terminals narrower than this show the info full screen instead of beside the list.
pub const SIDE_PANEL_MIN_WIDTH: u16 = 100;

/// The panel's width at `full_width` columns; 0 when there is none.
pub fn side_panel_width(full_width: u16, chrome: &Chrome) -> u16 {
    if full_width >= SIDE_PANEL_MIN_WIDTH && chrome.panel.is_some() { (full_width * 2 / 5).max(44) } else { 0 }
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
pub fn list_body(frame_area: Rect, chrome: &Chrome) -> Rect {
    let body = beside_sidebar(frame_area, true, chrome);
    Rect { width: body.width - side_panel_width(frame_area.width, chrome).min(body.width), ..body }
}

pub(super) fn draw_side_panel(frame: &mut Frame, body: Rect, chrome: &Chrome) {
    let width = side_panel_width(frame.area().width, chrome).min(body.width);
    if width == 0 {
        return;
    }
    let Some(panel) = chrome.panel.as_ref() else { return };
    let area = Rect { x: body.x + body.width - width, width, ..body };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(if panel.focused { Style::default().fg(theme().accent).add_modifier(Modifier::BOLD) } else { theme_border(false) })
        .title(pill_title(&panel.title, false, if panel.focused { Style::default().fg(theme().accent).add_modifier(Modifier::BOLD) } else { theme_border(false) }))
        .title_bottom(
            if panel.focused {
                hint_strip(&[("enter", "full screen"), ("shift-←", "list"), ("i", "close")])
            } else {
                hint_strip(&[("shift-→", "focus"), ("i", "close")])
            }
            .right_aligned(),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let padded = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let lines = details_lines(&panel.sections, usize::from(padded.width));
    let scroll = panel.scroll.min(lines.len().saturating_sub(usize::from(padded.height)));
    let hscroll = panel.hscroll.min(lines.iter().map(Line::width).max().unwrap_or(0).saturating_sub(usize::from(padded.width)));
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, hscroll as u16)), padded);
}

#[cfg(test)]
mod flow_tests {
    use super::*;

    fn chip(text: &str) -> Chunk {
        Chunk { text: text.into(), style: DStyle::Chip }
    }

    fn pair(key: &str, value: &str) -> Chunk {
        Chunk { text: format!("{key}={value}"), style: DStyle::PairChip }
    }

    #[test]
    fn a_chip_that_fits_stays_on_one_line() {
        let lines = flow(vec![Span::raw("Labels ")], 7, &[chip("role")], 40);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].width() <= 40);
    }

    #[test]
    fn a_pill_wider_than_the_line_wraps_instead_of_running_off_the_edge() {
        // 40 cells, more than this test's width of 20 can hold on one line.
        let long = "a".repeat(40);
        let lines = flow(vec![Span::raw("Annotations ")], 12, &[pair("some.thing.io/key", &long)], 20);
        assert!(lines.len() > 1, "expected the oversized pill to wrap across lines, got {}", lines.len());
        for line in &lines {
            assert!(line.width() <= 20, "line {line:?} is {} cells wide, wider than the 20-cell limit", line.width());
        }
        // Every 'a' still shows up somewhere; the pieces are split by indent padding,
        // so the run can't be searched for intact.
        let count = lines.iter().flat_map(|l| l.spans.iter()).flat_map(|s| s.content.chars()).filter(|&c| c == 'a').count();
        assert_eq!(count, 40);
    }

    #[test]
    fn an_oversized_pill_followed_by_nothing_leaves_no_stray_blank_line() {
        let long = "b".repeat(40);
        let lines = flow(vec![Span::raw("Labels ")], 7, &[pair("k", &long)], 20);
        let last_has_content = lines.last().unwrap().spans.iter().any(|s| !s.content.trim().is_empty());
        assert!(last_has_content, "trailing line should carry real content, not be blank");
    }

    #[test]
    fn an_empty_value_pair_is_a_bare_chip_not_a_blank_pill() {
        // An annotation with an empty value is a short chip of just the key.
        let bare = chip("objectset.rio.cattle.io/id");
        let lines = flow(vec![Span::raw("Annotations ")], 12, &[bare], 60);
        assert_eq!(lines.len(), 1, "a chip that fits shouldn't wrap");
        assert_eq!(lines[0].width(), cell_width("Annotations ") + cell_width(" objectset.rio.cattle.io/id "));
    }
}
