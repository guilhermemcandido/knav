//! `N`: the last notifications, newest first.

use super::super::*;

pub(super) fn open(st: &mut State) {
    let back = std::mem::replace(&mut st.mode, Mode::List);
    st.mode = Mode::Notifications { state: TableState::default().with_selected(0), back: Box::new(back) };
}

pub(super) fn handle(event: Event, st: &mut State) -> Result<Option<SessionEnd>> {
    let Mode::Notifications { state, back } = &mut st.mode else { return Ok(None) };
    let len = ui::notify::all().len();
    match event {
        Event::Key(key) => match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'N') => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Down | KeyCode::Char('j') => select_next(state, len),
            KeyCode::Up | KeyCode::Char('k') => select_prev(state, len),
            KeyCode::Char('g') | KeyCode::Home => state.select(Some(0)),
            KeyCode::Char('G') | KeyCode::End => state.select(Some(len.saturating_sub(1))),
            _ => {}
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollDown => select_next(state, len),
            MouseEventKind::ScrollUp => select_prev(state, len),
            _ => {}
        },
        _ => {}
    }
    Ok(None)
}
