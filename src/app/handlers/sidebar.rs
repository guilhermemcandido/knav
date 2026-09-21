//! The resource sidebar: `b` (or `m`) shows or hides it, Shift-Left gives it the keys, clicks open a kind.

use super::super::sidebar::{self as rows, Entry};
use super::super::*;
use super::Cx;

/// Handles an event for the sidebar; `true` when it was used up.
pub(super) fn handle(event: &Event, st: &mut State, cx: &mut Cx) -> bool {
    if !matches!(st.mode, Mode::List) {
        return false;
    }
    match event {
        Event::Key(key) => keys(key, st, cx),
        Event::Mouse(mouse) => mouse_event(mouse, st, cx),
        _ => false,
    }
}

fn shown(st: &State, cx: &Cx) -> bool {
    st.sidebar && cx.frame_area.width >= ui::SIDEBAR_MIN_WIDTH
}

fn entries(st: &State, cx: &Cx) -> Vec<Entry> {
    rows::entries(st.current_kind, &st.sidebar_folded, &cx.catalog.crds, &cx.d.overview)
}

fn keys(key: &crossterm::event::KeyEvent, st: &mut State, cx: &mut Cx) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if matches!(key.code, KeyCode::Char('b' | 'm')) {
        toggle(st, cx);
        return true;
    }
    if !shown(st, cx) {
        return false;
    }
    if !st.sidebar_focus {
        if shift && key.code == KeyCode::Left && !st.info_focus {
            let all = entries(st, cx);
            st.sidebar_focus = true;
            st.sidebar_cursor = rows::current_index(&all);
            return true;
        }
        return false;
    }
    let all = entries(st, cx);
    let last = all.len().saturating_sub(1);
    st.sidebar_cursor = st.sidebar_cursor.min(last);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Tab => st.sidebar_focus = false,
        KeyCode::Right if shift => st.sidebar_focus = false,
        KeyCode::Down | KeyCode::Char('j') => st.sidebar_cursor = (st.sidebar_cursor + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => st.sidebar_cursor = st.sidebar_cursor.saturating_sub(1),
        KeyCode::Home | KeyCode::Char('g') => st.sidebar_cursor = 0,
        KeyCode::End | KeyCode::Char('G') => st.sidebar_cursor = last,
        KeyCode::Enter | KeyCode::Char(' ') => activate(st, &all, st.sidebar_cursor),
        // Right opens a folded category; Left folds it, or steps out to the heading above a kind.
        KeyCode::Right | KeyCode::Char('l') => {
            if let Some(entry) = all.get(st.sidebar_cursor).filter(|e| e.row.heading && e.row.collapsed) {
                st.sidebar_folded.remove(entry.section);
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if let Some(entry) = all.get(st.sidebar_cursor) {
                if entry.row.heading {
                    st.sidebar_folded.insert(entry.section);
                } else if let Some(at) = all.iter().position(|e| e.row.heading && e.section == entry.section) {
                    st.sidebar_cursor = at;
                }
            }
        }
        _ => return false,
    }
    true
}

/// Shows the sidebar (with the keys) or hides it.
fn toggle(st: &mut State, cx: &Cx) {
    st.sidebar = !st.sidebar;
    st.sidebar_focus = st.sidebar;
    if st.sidebar {
        st.sidebar_cursor = rows::current_index(&entries(st, cx));
        if cx.frame_area.width < ui::SIDEBAR_MIN_WIDTH {
            let back = std::mem::replace(&mut st.mode, Mode::List);
            st.mode = Mode::Notice { text: format!("The sidebar needs a terminal at least {} columns wide.", ui::SIDEBAR_MIN_WIDTH), error: false, back: Box::new(back) };
        }
    }
}

/// Opens the kind on `index`, or folds and unfolds a category.
fn activate(st: &mut State, all: &[Entry], index: usize) {
    let Some(entry) = all.get(index) else { return };
    match entry.kind {
        Some(kind) => {
            st.switch_kind(kind);
            st.sidebar_focus = false;
        }
        None => {
            if !st.sidebar_folded.remove(entry.section) {
                st.sidebar_folded.insert(entry.section);
            }
        }
    }
}

fn mouse_event(mouse: &crossterm::event::MouseEvent, st: &mut State, cx: &mut Cx) -> bool {
    if !shown(st, cx) {
        return false;
    }
    let area = ui::sidebar_area(cx.frame_area, st.current_kind != ResourceKind::Overview);
    let inside = mouse.column >= area.x && mouse.column < area.x + area.width && mouse.row >= area.y && mouse.row < area.y + area.height;
    if !inside {
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            st.sidebar_focus = false;
        }
        return false;
    }
    let all = entries(st, cx);
    let shown_row = if st.sidebar_focus { st.sidebar_cursor.min(all.len().saturating_sub(1)) } else { rows::current_index(&all) };
    match mouse.kind {
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            if let Some(index) = ui::sidebar_row_at(area, shown_row, all.len(), mouse.column, mouse.row) {
                st.sidebar_cursor = index;
                st.sidebar_focus = true;
                activate(st, &all, index);
            }
        }
        MouseEventKind::ScrollDown => {
            st.sidebar_focus = true;
            st.sidebar_cursor = (shown_row + 1).min(all.len().saturating_sub(1));
        }
        MouseEventKind::ScrollUp => {
            st.sidebar_focus = true;
            st.sidebar_cursor = shown_row.saturating_sub(1);
        }
        _ => {}
    }
    true
}
