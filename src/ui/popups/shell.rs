//! The embedded shell and the YAML viewer.

use super::*;

/// Where an embedded shell's screen goes: inside the body's border.
pub fn shell_inner(frame_area: Rect) -> Rect {
    let area = body_area(frame_area, true);
    Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) }
}

fn shell_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub(in crate::ui) fn draw_shell_popup(frame: &mut Frame, title: &str, screen: &vt100::Screen, exited: bool) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let bottom = if exited { " the shell ended; press any key " } else { " ctrl-] closes " };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(pill_title(&format!("Shell {title}"), false, theme_border(false)))
        .title_bottom(Line::styled(bottom, Style::default().fg(if exited { theme().warn } else { theme().muted })).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let buffer = frame.buffer_mut();
    for row in 0..inner.height {
        for column in 0..inner.width {
            let Some(cell) = screen.cell(row, column) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut style = Style::default().fg(shell_color(cell.fgcolor())).bg(shell_color(cell.bgcolor()));
            for (on, modifier) in [(cell.bold(), Modifier::BOLD), (cell.italic(), Modifier::ITALIC), (cell.underline(), Modifier::UNDERLINED), (cell.inverse(), Modifier::REVERSED)] {
                if on {
                    style = style.add_modifier(modifier);
                }
            }
            let contents = cell.contents();
            buffer[(inner.x + column, inner.y + row)].set_symbol(if contents.is_empty() { " " } else { contents }).set_style(style);
        }
    }
    if !exited && !screen.hide_cursor() {
        let (row, column) = screen.cursor_position();
        if row < inner.height && column < inner.width {
            frame.set_cursor_position((inner.x + column, inner.y + row));
        }
    }
}

/// One YAML line coloured by role: keys blue, the rest plain, list dashes muted.
fn yaml_line(line: &str) -> Line<'static> {
    let indent = line.len() - line.trim_start().len();
    let (lead, rest) = line.split_at(indent);
    let (dash, rest) = match rest.strip_prefix("- ") {
        Some(after) => ("- ", after),
        None => ("", rest),
    };
    let key_style = Style::default().fg(theme().key);
    let plain = Style::default().fg(theme().text_soft);
    let mut spans = vec![Span::raw(lead.to_string()), Span::styled(dash.to_string(), Style::default().fg(theme().muted))];
    match rest.split_once(": ").or_else(|| rest.strip_suffix(':').map(|k| (k, ""))) {
        Some((key, value)) if !key.contains(' ') || key.starts_with('"') => {
            spans.push(Span::styled(key.to_string(), key_style));
            spans.push(Span::styled(":", Style::default().fg(theme().muted)));
            if !value.is_empty() {
                spans.push(Span::styled(format!(" {value}"), plain));
            }
        }
        _ => spans.push(Span::styled(rest.to_string(), plain)),
    }
    Line::from(spans)
}

pub(in crate::ui) fn draw_yaml_popup(frame: &mut Frame, title: &str, text: &str, scroll: usize, hscroll: usize) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(pill_title(title, false, theme_border(false)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines: Vec<Line> = text.lines().skip(scroll).take(usize::from(inner.height)).map(|l| shift_line(yaml_line(l), hscroll)).collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
