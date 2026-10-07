//! Selecting text with the mouse: drag a rectangle over the screen and, on release,
//! it is copied. The text comes from the last frame drawn, so it is what is on screen.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::*;

#[derive(Default)]
struct Selection {
    anchor: (u16, u16),
    head: (u16, u16),
    dragging: bool,
    /// The last frame's cell symbols by row.
    screen: Vec<Vec<String>>,
    toast: Option<(String, bool, Instant)>,
}

static SELECTION: Mutex<Selection> = Mutex::new(Selection { anchor: (0, 0), head: (0, 0), dragging: false, screen: Vec::new(), toast: None });

fn with<T>(f: impl FnOnce(&mut Selection) -> T) -> T {
    f(&mut SELECTION.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The button went down at `(column, row)`: a drag may start from here.
pub fn selection_start(column: u16, row: u16) {
    with(|s| {
        s.anchor = (column, row);
        s.head = (column, row);
        s.dragging = false;
    });
}

/// The mouse moved with the button down. True once it is a selection rather than a click.
pub fn selection_drag(column: u16, row: u16) -> bool {
    with(|s| {
        s.head = (column, row);
        s.dragging |= s.head != s.anchor;
        s.dragging
    })
}

/// The button came up: the selected text if there was a drag, and the selection ends.
pub fn selection_finish() -> Option<String> {
    with(|s| {
        if !std::mem::take(&mut s.dragging) {
            return None;
        }
        let ((c0, r0), (c1, r1)) = (s.anchor, s.head);
        let (cols, rows) = ((c0.min(c1), c0.max(c1)), (r0.min(r1), r0.max(r1)));
        let mut lines: Vec<String> = (rows.0..=rows.1)
            .map(|row| {
                let cells = s.screen.get(usize::from(row));
                let line: String = (cols.0..=cols.1).map(|col| cells.and_then(|c| c.get(usize::from(col))).map_or("", String::as_str)).collect();
                line.trim_end().to_string()
            })
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        let text = lines.join("\n");
        (!text.trim().is_empty()).then_some(text)
    })
}

/// A short message in the bottom corner for a couple of seconds.
pub fn show_toast(text: String, good: bool) {
    with(|s| s.toast = Some((text, good, Instant::now())));
}

/// Called after each frame is drawn: remembers its text, marks the selection and
/// shows the toast.
pub fn after_frame(frame: &mut Frame) {
    let area = frame.area();
    with(|s| {
        let buf = frame.buffer_mut();
        s.screen = (0..area.height).map(|y| (0..area.width).map(|x| buf[(area.x + x, area.y + y)].symbol().to_string()).collect()).collect();
        if s.dragging {
            let ((c0, r0), (c1, r1)) = (s.anchor, s.head);
            for y in r0.min(r1)..=r0.max(r1).min(area.height.saturating_sub(1)) {
                for x in c0.min(c1)..=c0.max(c1).min(area.width.saturating_sub(1)) {
                    let cell = &mut buf[(area.x + x, area.y + y)];
                    cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
                }
            }
        }
        if let Some((text, good, at)) = &s.toast {
            if at.elapsed() > Duration::from_millis(2000) {
                s.toast = None;
            } else if area.height > 0 {
                let label = format!(" {text} ");
                let width = (cell_width(&label) as u16).min(area.width);
                let rect = Rect { x: area.x + area.width - width, y: area.y + area.height - 1, width, height: 1 };
                let color = if *good { theme().ok } else { theme().bad };
                frame.render_widget(Clear, rect);
                frame.render_widget(Paragraph::new(Span::styled(label, Style::default().fg(color).bg(theme().pill_bg).add_modifier(Modifier::BOLD))), rect);
            }
        }
    });
}
