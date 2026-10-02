//! Keys on an extension dashboard: no selection, just a scroll position the drawing
//! clamps to what fits.

use super::*;

const PAGE: usize = 10;

pub(super) fn keys(key: crossterm::event::KeyEvent, st: &mut State) {
    if !matches!(st.current_kind, ResourceKind::ExtensionDashboard(_)) {
        return;
    }
    let scroll = &mut st.dashboard_scroll;
    let ctrl = key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => *scroll += 1,
        KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
        KeyCode::PageDown => *scroll += PAGE,
        KeyCode::Char('f') if ctrl => *scroll += PAGE,
        KeyCode::Char('d') if ctrl => *scroll += PAGE / 2,
        KeyCode::PageUp => *scroll = scroll.saturating_sub(PAGE),
        KeyCode::Char('b') if ctrl => *scroll = scroll.saturating_sub(PAGE),
        KeyCode::Char('u') if ctrl => *scroll = scroll.saturating_sub(PAGE / 2),
        KeyCode::Char('g') => *scroll = 0,
        KeyCode::Char('G') => *scroll = usize::MAX / 2, // clamped to content at draw time
        _ => {}
    }
}
