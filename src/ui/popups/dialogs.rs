//! Notices, confirmation, scaling and the port-forward form.

use super::*;

/// The width the dialogs share: 3/5 of the terminal, between 44 and 72 columns.
fn narrow_dialog_width(full_width: u16) -> u16 {
    (full_width * 3 / 5).clamp(44, 72)
}

/// A small centred message, as wide as its text up to the dialog width: green "Done",
/// accent "Info" or red "Failed". Any key closes it.
pub(in crate::ui) fn draw_notice_popup(frame: &mut Frame, text: &str, tone: crate::ops::NoticeTone) {
    use crate::ops::NoticeTone as Tone;
    let full = frame.area();
    let (title, color) = match tone {
        Tone::Done => ("Done", theme().ok),
        Tone::Info => ("Info", theme().accent),
        Tone::Failed => ("Failed", theme().bad),
    };
    const PAD: u16 = 2;
    let hint = hint_strip(&[("any key", "close")]);
    let text_w = text.lines().map(cell_width).max().unwrap_or(0) as u16;
    let floor = (hint.width() as u16 + 4).max(30);
    let width = (text_w + 2 * PAD + 2).clamp(floor, narrow_dialog_width(full.width)).min(full.width);
    let inner_w = usize::from(width.saturating_sub(2 * PAD + 2)).max(1);
    let lines: usize = text.lines().map(|l| cell_width(l).div_ceil(inner_w).max(1)).sum::<usize>().max(1);
    let height = (lines as u16 + 4 /* borders, a blank line above and below */).min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 2, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(Style::default().fg(color))
        .title(pill_title(title, false, Style::default().fg(color)))
        .title_bottom(hint.right_aligned())
        .padding(Padding::new(PAD, PAD, 1, 1));
    frame.render_widget(Paragraph::new(text.to_string()).wrap(Wrap { trim: false }).block(block), area);
}

/// A small centred box with a title and body lines, as wide as the other dialogs.
fn small_popup(frame: &mut Frame, title: &str, color: Color, body: Vec<Line<'static>>) {
    let full = frame.area();
    let width = narrow_dialog_width(full.width).min(full.width);
    let height = (body.len() as u16 + 2).min(full.height);
    let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 2, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(Style::default().fg(color)).title(pill_title(title, false, Style::default().fg(color)));
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }).block(block), area);
}

/// A dialog button: filled with `fill` when focused, quiet otherwise.
fn dialog_button(text: &str, fill: Color, focused: bool) -> Span<'static> {
    let style = if focused { Style::default().bg(fill).fg(crate::theme::on(fill)).add_modifier(Modifier::BOLD) } else { Style::default().bg(theme().pill_bg).fg(theme().muted) };
    Span::styled(text.to_string(), style)
}

/// Where a dialog's two buttons are on screen, for clicks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DialogButtons {
    pub yes: Rect,
    pub no: Rect,
}

/// A dialog with a body that wraps, then two buttons and a line of keys. Drawing and
/// clicking share its layout, so a click lands where the button is drawn.
struct Dialog {
    title: String,
    color: Color,
    body: Vec<Line<'static>>,
    buttons: [Span<'static>; 2],
    keys: Line<'static>,
}

const BUTTON_GAP: u16 = 3;

impl Dialog {
    /// The box, the space for the body, and the buttons.
    fn layout(&self, full: Rect) -> (Rect, Rect, DialogButtons) {
        let width = narrow_dialog_width(full.width).min(full.width);
        // Borders and two cells of padding on each side.
        let text_w = width.saturating_sub(6);
        let body_h = Paragraph::new(self.body.clone()).wrap(Wrap { trim: false }).line_count(text_w) as u16;
        // The body, the buttons, a blank line, the keys and the borders.
        let height = (body_h + 5).min(full.height);
        let area = Rect { x: full.x + full.width.saturating_sub(width) / 2, y: full.y + full.height.saturating_sub(height) / 2, width, height };
        let body = Rect { x: area.x + 3, y: area.y + 1, width: text_w, height: body_h };
        let [yes_w, no_w] = self.buttons.each_ref().map(|b| b.width() as u16);
        let x = body.x + text_w.saturating_sub(yes_w + BUTTON_GAP + no_w) / 2;
        let y = body.y + body_h;
        let buttons = DialogButtons { yes: Rect { x, y, width: yes_w, height: 1 }, no: Rect { x: x + yes_w + BUTTON_GAP, y, width: no_w, height: 1 } };
        (area, body, buttons)
    }

    fn draw(self, frame: &mut Frame) {
        let (area, body, buttons) = self.layout(frame.area());
        frame.render_widget(Clear, area);
        let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(Style::default().fg(self.color)).title(pill_title(&self.title, false, Style::default().fg(self.color)));
        frame.render_widget(block, area);
        let visible = |r: Rect| r.intersection(area.inner(ratatui::layout::Margin::new(1, 1)));
        frame.render_widget(Paragraph::new(self.body).wrap(Wrap { trim: false }), visible(body));
        let [yes, no] = self.buttons;
        frame.render_widget(Paragraph::new(Line::from(yes)), visible(buttons.yes));
        frame.render_widget(Paragraph::new(Line::from(no)), visible(buttons.no));
        let keys = Rect { y: buttons.yes.y + 2, height: 1, ..body };
        frame.render_widget(Paragraph::new(self.keys.centered()), visible(keys));
    }
}

fn key_span(k: &str) -> Span<'static> {
    Span::styled(k.to_string(), Style::default().add_modifier(Modifier::BOLD))
}

fn confirm_dialog(spec: &crate::ops::actions::ConfirmSpec, yes: bool) -> Dialog {
    let color = if spec.danger { theme().bad } else { theme().accent };
    let muted = Style::default().fg(theme().muted);
    let mut body: Vec<Line> = vec![Line::raw("")];
    let kind_w = spec.subjects.iter().map(|(k, _)| cell_width(k)).max().unwrap_or(0).min(20);
    for (kind, place) in &spec.subjects {
        let mut spans = Vec::new();
        if !kind.is_empty() {
            spans.push(Span::styled(format!("{kind:<w$}  ", w = kind_w), muted));
        } else if kind_w > 0 {
            spans.push(Span::raw(" ".repeat(kind_w + 2)));
        }
        spans.push(Span::styled(place.clone(), if kind.is_empty() { muted } else { Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD) }));
        body.push(Line::from(spans));
    }
    if !spec.notes.is_empty() {
        body.push(Line::raw(""));
    }
    for (note, warning) in &spec.notes {
        body.push(Line::styled(note.clone(), if *warning { Style::default().fg(theme().warn) } else { muted }));
    }
    body.push(Line::raw(""));
    let icon = if spec.danger { "⚠ " } else { "" };
    Dialog {
        title: format!("{icon}{}", spec.title),
        color,
        body,
        buttons: [dialog_button(&format!("  y  {}  ", spec.verb), color, yes), dialog_button("  n  Cancel  ", theme().select_bg, !yes)],
        keys: Line::from(vec![key_span("←→"), Span::styled(" choose   ", muted), key_span("enter"), Span::styled(" press   ", muted), key_span("esc"), Span::styled(" cancel", muted)]),
    }
}

/// The confirmation dialog: what is about to happen, to what, and two buttons with
/// `yes` telling which has focus. Destructive actions are red.
pub(in crate::ui) fn draw_confirm_popup(frame: &mut Frame, spec: &crate::ops::actions::ConfirmSpec, yes: bool) {
    let full = frame.area();
    frame.buffer_mut().set_style(full, Style::default().add_modifier(Modifier::DIM));
    confirm_dialog(spec, yes).draw(frame);
}

/// Where the confirmation dialog's buttons are on a screen of `full`.
pub fn confirm_buttons(full: Rect, spec: &crate::ops::actions::ConfirmSpec) -> DialogButtons {
    confirm_dialog(spec, true).layout(full).2
}

/// The port-forward dialog: labelled fields, a warning when the port is a guess, and
/// OK or Cancel.
pub(in crate::ui) fn draw_port_forward_popup(frame: &mut Frame, title: &str, form: &crate::ops::portforward::PortForm) {
    use crate::ops::portforward::Field;
    let full = frame.area();
    let width = narrow_dialog_width(full.width).min(full.width);
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

fn scale_dialog(view: &ScaleView) -> Dialog {
    let color = theme().accent;
    let muted = Style::default().fg(theme().muted);
    let strong = Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD);
    let mut body: Vec<Line> = vec![Line::raw("")];
    const SHOWN: usize = 4;
    let kind_w = view.subjects.iter().map(|(k, _, _)| cell_width(k)).max().unwrap_or(0);
    for (kind, place, ready) in view.subjects.iter().take(SHOWN) {
        body.push(Line::from(vec![Span::styled(format!("{kind:<kind_w$}  "), muted), Span::styled(place.clone(), strong), Span::styled(format!("   {ready}"), muted)]));
    }
    if view.subjects.len() > SHOWN {
        body.push(Line::styled(format!("and {} more", view.subjects.len() - SHOWN), muted));
    }
    body.push(Line::raw(""));

    let target = view.value.parse::<i64>().ok();
    let number = if view.value.is_empty() { " ".to_string() } else { view.value.to_string() };
    body.push(Line::from(vec![Span::styled("◀   ", muted), Span::styled(format!(" {number} "), Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD)), Span::styled("   ▶", muted)]).centered());
    let (change, style) = match (view.current, target) {
        (_, None) => ("Type a number".to_string(), muted),
        (_, Some(0)) => ("0 stops every pod".to_string(), Style::default().fg(theme().warn)),
        (Some(now), Some(to)) if now == to => (format!("{now} now, no change"), muted),
        (Some(now), Some(to)) => (format!("{now} → {to} replicas ({:+})", to - now), Style::default().fg(if to > now { theme().ok } else { theme().warn })),
        (None, Some(to)) => (format!("Each one to {to} replicas"), muted),
    };
    body.push(Line::styled(change, style).centered());
    body.push(Line::raw(""));
    Dialog {
        title: "Scale".into(),
        color,
        body,
        buttons: [dialog_button("  Scale  ", color, view.yes), dialog_button("  esc  Cancel  ", theme().select_bg, !view.yes)],
        keys: Line::from(vec![key_span("↑↓"), Span::styled(" or ", muted), key_span("0-9"), Span::styled(" change   ", muted), key_span("tab"), Span::styled(" choose   ", muted), key_span("enter"), Span::styled(" press", muted)]),
    }
}

/// Scaling: the objects, a number stepped with the arrows or typed, what changes, and
/// the buttons. Zero is a warning, since it stops every pod.
pub(in crate::ui) fn draw_scale_popup(frame: &mut Frame, view: &ScaleView) {
    scale_dialog(view).draw(frame);
}

/// Where the scale dialog's buttons are on a screen of `full`.
pub fn scale_buttons(full: Rect, view: &ScaleView) -> DialogButtons {
    scale_dialog(view).layout(full).2
}

/// A background job's box: a spinner, what it is doing, progress when the total is
/// known, and how to cancel.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::actions::ConfirmSpec;

    fn text_at(buffer: &ratatui::buffer::Buffer, r: Rect) -> String {
        (r.x..r.x + r.width).map(|x| buffer[(x, r.y)].symbol()).collect()
    }

    #[test]
    fn buttons_are_drawn_where_clicks_look_for_them() {
        // A long note wraps, which moves the buttons down.
        let spec = ConfirmSpec { title: "Delete pod?".into(), verb: "Delete".into(), danger: true, subjects: vec![("Pod".into(), "default/web-1".into())], notes: vec![("word ".repeat(40), false)] };
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw_confirm_popup(f, &spec, false)).unwrap();
        let buttons = confirm_buttons(Rect::new(0, 0, 100, 30), &spec);
        let buffer = terminal.backend().buffer();
        assert_eq!(text_at(buffer, buttons.yes), "  y  Delete  ");
        assert_eq!(text_at(buffer, buttons.no), "  n  Cancel  ");
    }

    #[test]
    fn scale_buttons_are_drawn_where_clicks_look_for_them() {
        let view = ScaleView { subjects: vec![("Deployment".into(), "default/web".into(), "2/2 ready".into())], value: "3", current: Some(2), yes: true };
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| draw_scale_popup(f, &view)).unwrap();
        let buttons = scale_buttons(Rect::new(0, 0, 80, 24), &view);
        let buffer = terminal.backend().buffer();
        assert_eq!(text_at(buffer, buttons.yes), "  Scale  ");
        assert_eq!(text_at(buffer, buttons.no), "  esc  Cancel  ");
    }
}
