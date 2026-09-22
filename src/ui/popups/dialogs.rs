//! Notices, confirmation, prompts and the port-forward form.

use super::*;

/// The width most dialogs in this file settle on: 3/5 of the terminal, kept readable.
fn narrow_dialog_width(full_width: u16) -> u16 {
    (full_width * 3 / 5).clamp(44, 72)
}

/// A small centered message box, green-bordered for success, red for
/// an error. Sized to the text so a one-liner doesn't get a huge box.
pub(in crate::ui) fn draw_notice_popup(frame: &mut Frame, text: &str, error: bool) {
    let full = frame.area();
    let width = (full.width * 3 / 5).max(30).min(full.width);
    let inner_w = width.saturating_sub(2).max(1) as usize;
    let lines: usize = text.lines().map(|l| cell_width(l).div_ceil(inner_w).max(1)).sum::<usize>().max(1);
    let height = (lines as u16 + 2).min(full.height);
    let area = Rect {
        x: full.x + full.width.saturating_sub(width) / 2,
        y: full.y + full.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, area);
    let color = if error { theme().bad } else { theme().ok };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(Style::default().fg(color))
        .title(pill_title(if error { "Failed" } else { "Done" }, false, Style::default().fg(color)));
    frame.render_widget(Paragraph::new(text.to_string()).wrap(Wrap { trim: false }).block(block), area);
}

/// A small centred box with a title and body lines, for the question popups.
fn small_popup(frame: &mut Frame, title: &str, color: Color, body: Vec<Line<'static>>) {
    let full = frame.area();
    let width = (full.width * 3 / 5).max(30).min(full.width);
    let height = (body.len() as u16 + 2).min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 2, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(Style::default().fg(color)).title(pill_title(title, false, Style::default().fg(color)));
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }).block(block), area);
}

/// The confirmation dialog: what is about to happen, to what, what follows, and
/// two buttons. Destructive actions are drawn in red and need an explicit `y`.
pub(in crate::ui) fn draw_confirm_popup(frame: &mut Frame, spec: &crate::ops::actions::ConfirmSpec) {
    let full = frame.area();
    let color = if spec.danger { theme().bad } else { theme().accent };
    let width = narrow_dialog_width(full.width).min(full.width);
    let inner_w = usize::from(width).saturating_sub(6);
    let muted = Style::default().fg(theme().muted);
    let mut lines: Vec<Line> = vec![Line::raw("")];
    let kind_w = spec.subjects.iter().map(|(k, _)| cell_width(k)).max().unwrap_or(0).min(20);
    for (kind, place) in &spec.subjects {
        let mut spans = Vec::new();
        if !kind.is_empty() {
            spans.push(Span::styled(format!("{kind:<w$}  ", w = kind_w), muted));
        } else if kind_w > 0 {
            spans.push(Span::raw(" ".repeat(kind_w + 2)));
        }
        spans.push(Span::styled(place.clone(), if kind.is_empty() { muted } else { Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD) }));
        lines.push(Line::from(spans));
    }
    if !spec.notes.is_empty() {
        lines.push(Line::raw(""));
    }
    for (note, warning) in &spec.notes {
        lines.push(Line::styled(note.clone(), if *warning { Style::default().fg(theme().warn) } else { muted }));
    }
    lines.push(Line::raw(""));
    let key = |k: &str| Span::styled(k.to_string(), Style::default().add_modifier(Modifier::BOLD));
    let yes = Span::styled(format!("  y  {}  ", spec.verb), Style::default().bg(color).fg(crate::theme::on(color)).add_modifier(Modifier::BOLD));
    let no = Span::styled("  n  Cancel  ", Style::default().bg(theme().pill_bg).fg(theme().text_strong));
    lines.push(Line::from(vec![yes, Span::raw("   "), no]).centered());
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![if spec.danger { key("y") } else { key("y / enter") }, Span::styled(" confirms   ", muted), key("n / esc"), Span::styled(" cancels", muted)]).centered());
    // Wrapped notes can take more than one row each.
    let wrapped: usize = lines.iter().map(|l| (l.width() / inner_w.max(1)) + 1).sum();
    let height = (wrapped as u16 + 2).min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 2, width, height };
    // The screen behind recedes, and the dialog gets its own clear panel.
    let backdrop = full;
    frame.buffer_mut().set_style(backdrop, Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(Clear, area);
    let icon = if spec.danger { "⚠ " } else { "" };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(Style::default().fg(color))
        .title(pill_title(&format!("{icon}{}", spec.title), false, Style::default().fg(color)));
    let text_area = block.inner(area);
    frame.render_widget(block, area);
    let text_area = Rect { x: text_area.x + 2, width: text_area.width.saturating_sub(4), ..text_area };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text_area);
}

/// The port-forward dialog, laid out like k9s's: labelled fields, a warning
/// when the port is a guess, and OK / Cancel.
pub(in crate::ui) fn draw_port_forward_popup(frame: &mut Frame, title: &str, form: &crate::ops::portforward::PortForm) {
    use crate::ops::portforward::Field;
    let full = frame.area();
    let width = (full.width * 3 / 5).clamp(44, full.width.max(1)).min(full.width);
    let height = 11u16.min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 3, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(pill_title("Port forward", false, theme_border(false)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label = Style::default().fg(theme().label);
    let value = Style::default().fg(theme().text_strong);
    let hint = Style::default().fg(theme().muted);
    let field = |name: &str, text: &str, placeholder: &str, focused: bool| {
        let shown = if text.is_empty() && !focused { Span::styled(placeholder.to_string(), hint) } else { Span::styled(format!("{text}{}", if focused { "▏" } else { "" }), value) };
        Line::from(vec![Span::styled(format!(" {name:<16}"), label), shown])
    };
    let button = |name: &str, focused: bool| {
        let style = if focused { Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD) } else { Style::default().fg(value.fg.unwrap_or(theme().text_strong)) };
        Span::styled(format!(" {name} "), style)
    };
    let mut lines = vec![
        Line::styled(title.to_string(), value.add_modifier(Modifier::BOLD)).centered(),
        Line::raw(""),
        field("Container Port:", &form.container, "Enter the container port", form.focus == Field::Container),
        field("Local Port:", &form.local, "Enter a local port", form.focus == Field::Local),
        field("Address:", &form.address, "localhost", form.focus == Field::Address),
        Line::raw(""),
    ];
    let note = match (&form.error, form.warning()) {
        (Some(e), _) => Some(Line::styled(format!(" {e}"), Style::default().fg(theme().bad))),
        (None, Some(w)) => Some(Line::styled(format!(" ⚠ {w}"), Style::default().fg(theme().warn))),
        _ => None,
    };
    lines.push(note.unwrap_or_else(|| Line::raw("")));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![button("OK", form.focus == Field::Ok), Span::raw("   "), button("Cancel", form.focus == Field::Cancel)]).centered());
    frame.render_widget(Paragraph::new(lines), inner);
}

pub(in crate::ui) fn draw_prompt_popup(frame: &mut Frame, title: &str, value: &str, hint: &str) {
    let mut body = vec![Line::from(vec![Span::raw("> "), Span::styled(format!("{value}▏"), Style::default().fg(theme().highlight))])];
    if !hint.is_empty() {
        body.push(Line::styled(hint.to_string(), Style::default().fg(theme().muted)));
    }
    small_popup(frame, title, theme().namespace, body);
}

/// "Working" box for a background job: a spinner, what it is doing, a progress bar when the
/// total is known, and how to cancel.
pub(in crate::ui) fn draw_working_popup(frame: &mut Frame, title: &str, elapsed: std::time::Duration, done: usize, total: usize, cancellable: bool) {
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let spinner = SPINNER[(elapsed.as_millis() / 80) as usize % SPINNER.len()];
    let muted = Style::default().fg(theme().muted);
    let mut body = vec![Line::raw(""), Line::from(vec![Span::styled(format!("  {spinner} "), Style::default().fg(theme().accent)), Span::styled(title.to_string(), Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD)), Span::styled(format!("   {:.1}s", elapsed.as_secs_f32()), muted)])];
    if total > 1 {
        let full = frame.area();
        let bar = usize::from(narrow_dialog_width(full.width)).saturating_sub(16);
        let filled = (done * bar / total).min(bar);
        body.push(Line::from(vec![Span::raw("    "), Span::styled("█".repeat(filled), Style::default().fg(theme().ok)), Span::styled("░".repeat(bar - filled), muted), Span::styled(format!("  {done}/{total}"), muted)]));
    }
    if cancellable {
        body.push(Line::raw(""));
        body.push(Line::styled("  esc  cancel", muted));
    }
    small_popup(frame, "Working", theme().accent, body);
}
