//! Selecting text with the mouse: drag from one point to another and, on release, the
//! text between them is copied in reading order, line by line. Text panes (details,
//! logs, YAML) say which line each row shows, so whole lines are copied, the parts
//! scrolled off to the right included, and a wrapped log line comes back as one line.
//! Anywhere else the text comes from the last frame drawn.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;

/// One screen row of a text pane: the line it shows, the cell of that line it starts
/// at, and the whole line.
#[derive(Clone)]
pub(super) struct TextRow {
    pub line: usize,
    pub start: usize,
    pub text: Arc<str>,
}

#[derive(Clone)]
struct Region {
    area: Rect,
    /// `rows[i]` is drawn at `area.y + i`.
    rows: Vec<TextRow>,
}

#[derive(Default)]
struct Selection {
    anchor: (u16, u16),
    head: (u16, u16),
    dragging: bool,
    /// The last frame's cell symbols by row.
    screen: Vec<Vec<String>>,
    /// Text panes of the frame being drawn, then of the last one drawn.
    drawing: Vec<Region>,
    regions: Vec<Region>,
    toast: Option<(String, bool, Instant)>,
    /// A name following the pointer while something is dragged.
    ghost: Option<(String, u16, u16)>,
}

static SELECTION: Mutex<Selection> =
    Mutex::new(Selection { anchor: (0, 0), head: (0, 0), dragging: false, screen: Vec::new(), drawing: Vec::new(), regions: Vec::new(), toast: None, ghost: None });

fn with<T>(f: impl FnOnce(&mut Selection) -> T) -> T {
    f(&mut SELECTION.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Says that `area` shows `lines` (whole, unclipped) from line `first`, each scrolled
/// `hscroll` cells to the left, one per row.
pub(super) fn text_lines(area: Rect, lines: impl IntoIterator<Item = String>, first: usize, hscroll: usize) {
    let rows = lines.into_iter().take(usize::from(area.height)).enumerate().map(|(i, text)| TextRow { line: first + i, start: hscroll, text: text.into() }).collect();
    text_rows(area, rows);
}

/// Says what each row of `area` shows, for panes that wrap lines over several rows.
pub(super) fn text_rows(area: Rect, rows: Vec<TextRow>) {
    with(|s| s.drawing.push(Region { area, rows }));
}

/// A line's text without its styles.
pub(super) fn plain(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
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
        copied(&region_at(s, s.anchor), s.anchor, s.head)
    })
}

/// The text of `region` from `anchor` to `head`, whichever comes first, line by line.
fn copied(region: &Region, anchor: (u16, u16), head: (u16, u16)) -> Option<String> {
    let (from, to) = ordered(point(region, anchor), point(region, head));
    let mut lines: Vec<String> = Vec::new();
    let mut last = None;
    for row in &region.rows {
        if last == Some(row.line) || row.line < from.0 || row.line > to.0 {
            continue;
        }
        last = Some(row.line);
        let first = if row.line == from.0 { from.1 } else { 0 };
        let end = if row.line == to.0 { to.1 } else { usize::MAX };
        lines.push(cells(&row.text, first, end).trim_end().to_string());
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let text = lines.join("\n");
    (!text.trim().is_empty()).then_some(text)
}

/// The text pane under `at` in the last frame, or the whole screen as one.
fn region_at(s: &Selection, at: (u16, u16)) -> Region {
    let inside = |r: &&Region| at.0 >= r.area.x && at.0 < r.area.right() && at.1 >= r.area.y && at.1 < r.area.bottom();
    // A pane with something drawn over it doesn't show what it says, so it is skipped.
    let shown = |r: &&Region| {
        let i = usize::from(at.1 - r.area.y);
        let Some(row) = r.rows.get(i) else { return true };
        let width = usize::from(r.area.width);
        if width == 0 {
            return false;
        }
        let expected = cells(&row.text, row.start, row.start + width - 1);
        let on_screen: String = s.screen.get(usize::from(at.1)).map(|cells| cells.iter().skip(usize::from(r.area.x)).take(width).map(String::as_str).collect()).unwrap_or_default();
        expected.trim_end() == on_screen.trim_end()
    };
    if let Some(region) = s.regions.iter().rev().filter(inside).find(shown) {
        return region.clone();
    }
    let height = s.screen.len() as u16;
    let width = s.screen.first().map_or(0, |r| r.len()) as u16;
    let rows = s.screen.iter().enumerate().map(|(i, cells)| TextRow { line: i, start: 0, text: cells.concat().into() }).collect();
    Region { area: Rect { x: 0, y: 0, width, height }, rows }
}

/// Where `(column, row)` falls in `region`'s text: a line and a cell of it. Past the
/// right edge is the end of what that row shows (all of the line, when it is cut off).
fn point(region: &Region, (column, row): (u16, u16)) -> (usize, usize) {
    let Some(last) = region.rows.last() else { return (0, 0) };
    if row < region.area.y {
        return (region.rows[0].line, region.rows[0].start);
    }
    let i = usize::from(row - region.area.y);
    let Some(shown) = region.rows.get(i) else { return (last.line, usize::MAX) };
    if column < region.area.x {
        return (shown.line, shown.start);
    }
    if column >= region.area.right() {
        let wrapped = region.rows.get(i + 1).filter(|next| next.line == shown.line);
        return (shown.line, wrapped.map_or(usize::MAX, |next| next.start.saturating_sub(1)));
    }
    (shown.line, shown.start + usize::from(column - region.area.x))
}

fn ordered(a: (usize, usize), b: (usize, usize)) -> ((usize, usize), (usize, usize)) {
    if a <= b { (a, b) } else { (b, a) }
}

/// The characters of `text` that start between cells `from` and `to`, both included.
fn cells(text: &str, from: usize, to: usize) -> String {
    let mut at = 0;
    let mut out = String::new();
    for ch in text.chars() {
        if at > to {
            break;
        }
        if at >= from {
            out.push(ch);
        }
        at += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    out
}

/// A short message in the bottom corner for a couple of seconds.
pub fn show_toast(text: String, good: bool) {
    with(|s| s.toast = Some((text, good, Instant::now())));
}

/// A short red message for something that can't be done, with the reason.
pub fn refuse(text: impl Into<String>) {
    show_toast(text.into(), false);
}

/// Shows `name` beside the pointer at `(column, row)` while it drags something; `None` ends it.
pub fn set_ghost(ghost: Option<(String, u16, u16)>) {
    with(|s| s.ghost = ghost);
}

/// Called after each frame is drawn: remembers its text, marks the selection and
/// shows the toast.
pub fn after_frame(frame: &mut Frame) {
    let area = frame.area();
    with(|s| {
        let buf = frame.buffer_mut();
        s.screen = (0..area.height).map(|y| (0..area.width).map(|x| buf[(area.x + x, area.y + y)].symbol().to_string()).collect()).collect();
        s.regions = std::mem::take(&mut s.drawing);
        if s.dragging {
            // The selection runs in reading order, so it is lit row by row, not as a box.
            let region = region_at(s, s.anchor);
            let (from, to) = ordered(point(&region, s.anchor), point(&region, s.head));
            for (i, row) in region.rows.iter().enumerate() {
                let y = region.area.y + i as u16;
                if y >= area.bottom() {
                    break;
                }
                for x in 0..region.area.width.min(area.right().saturating_sub(region.area.x)) {
                    let at = (row.line, row.start + usize::from(x));
                    if at >= from && at <= to {
                        buf[(region.area.x + x, y)].set_style(Style::default().add_modifier(Modifier::REVERSED));
                    }
                }
            }
        }
        if let Some((name, column, row)) = &s.ghost {
            let label = format!(" ⠿ {name} ");
            let width = (cell_width(&label) as u16).min(area.width);
            let x = (column + 1).min(area.width.saturating_sub(width));
            let rect = Rect { x: area.x + x, y: area.y + (*row).min(area.height.saturating_sub(1)), width, height: 1 };
            frame.render_widget(Clear, rect);
            frame.render_widget(Paragraph::new(Span::styled(label, Style::default().fg(theme().pill_bg).bg(theme().namespace).add_modifier(Modifier::BOLD))), rect);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(line: usize, start: usize, text: &str) -> TextRow {
        TextRow { line, start, text: text.into() }
    }

    #[test]
    fn a_selection_runs_line_by_line_and_takes_what_is_cut_off_to_the_right() {
        // Two lines, 5 cells shown each, scrolled 0.
        let region = Region { area: Rect { x: 10, y: 2, width: 5, height: 2 }, rows: vec![row(0, 0, "alpha beta"), row(1, 0, "gamma delta")] };
        // From "ph" of line 0 to past the right edge on line 1: the rest of line 0, all of line 1.
        assert_eq!(copied(&region, (12, 2), (30, 3)).as_deref(), Some("pha beta\ngamma delta"));
        // Backwards gives the same.
        assert_eq!(copied(&region, (30, 3), (12, 2)).as_deref(), Some("pha beta\ngamma delta"));
        // Within one line, only the cells between.
        assert_eq!(copied(&region, (10, 3), (12, 3)).as_deref(), Some("gam"));
    }

    #[test]
    fn a_wrapped_line_is_copied_as_one() {
        let text = "abcdefgh";
        let region = Region { area: Rect { x: 0, y: 0, width: 4, height: 3 }, rows: vec![row(0, 0, text), row(0, 4, text), row(1, 0, "xy")] };
        assert_eq!(copied(&region, (0, 0), (3, 1)).as_deref(), Some("abcdefgh"));
        assert_eq!(copied(&region, (2, 0), (1, 2)).as_deref(), Some("cdefgh\nxy"));
        // Past the edge of a wrapped row stops at that row's end.
        assert_eq!(copied(&region, (0, 0), (9, 0)).as_deref(), Some("abcd"));
    }

    #[test]
    fn a_scrolled_line_starts_where_it_is_shown() {
        let region = Region { area: Rect { x: 0, y: 0, width: 3, height: 1 }, rows: vec![row(4, 2, "0123456789")] };
        assert_eq!(copied(&region, (0, 0), (1, 0)).as_deref(), Some("23"));
        assert_eq!(cells("日本語", 2, 3), "本");
    }
}
