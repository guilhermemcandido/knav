//! Selection movement shared by the list and the popups.

use super::*;

pub(crate) fn select_next(state: &mut TableState, len: usize) {
    if len == 0 {
        return;
    }
    let next = state.selected().map(|i| (i + 1).min(len - 1)).unwrap_or(0);
    state.select(Some(next));
}

pub(crate) fn select_prev(state: &mut TableState, len: usize) {
    if len == 0 {
        return;
    }
    let prev = state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
    state.select(Some(prev));
}

/// `g`, `G`, Home and End jump to the top or bottom; Ctrl-f, Ctrl-b, PageDown and PageUp
/// move a page, Ctrl-d and Ctrl-u half of one. False for any other key.
pub(crate) fn jump_select(key: &KeyEvent, state: &mut TableState, len: usize, page: usize) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let current = state.selected().unwrap_or(0);
    let last = len.saturating_sub(1);
    let target = match key.code {
        KeyCode::Char('g') | KeyCode::Home if !ctrl => 0,
        KeyCode::Char('G') | KeyCode::End if !ctrl => last,
        KeyCode::Char('f') if ctrl => (current + page).min(last),
        KeyCode::PageDown => (current + page).min(last),
        KeyCode::Char('b') if ctrl => current.saturating_sub(page),
        KeyCode::Char('d') if ctrl => (current + (page / 2).max(1)).min(last),
        KeyCode::Char('u') if ctrl => current.saturating_sub((page / 2).max(1)),
        KeyCode::PageUp => current.saturating_sub(page),
        _ => return false,
    };
    if len > 0 {
        state.select(Some(target));
    }
    true
}

/// Moves a table's selection for a wheel notch; false for any other mouse event.
pub(crate) fn wheel_select(kind: MouseEventKind, state: &mut TableState, len: usize) -> bool {
    let step: fn(&mut TableState, usize) = match kind {
        MouseEventKind::ScrollDown => select_next,
        MouseEventKind::ScrollUp => select_prev,
        _ => return false,
    };
    for _ in 0..crate::config::tunables::tunables().wheel_rows {
        step(state, len);
    }
    true
}

/// Sends `s` and the digits to the focused popup table; `true` if used as a sort key.
pub(crate) fn popup_sort_key(mode: &mut Mode, code: KeyCode) -> bool {
    match mode {
        Mode::Events { sort, editing, .. } => sort.handle(code, EVENT_COLUMNS, *editing),
        Mode::Containers { sort, .. } => sort.handle(code, CONTAINER_COLUMNS, false),
        Mode::NamespacePick { sort, editing, .. } => sort.handle(code, NAMESPACE_PICKER_COLUMNS, *editing),
        Mode::NodeDetail { sort, editing, .. } => sort.handle(code, POD_COLUMNS, *editing),
        _ => false,
    }
}

#[cfg(test)]
mod wheel_tests {
    use super::*;

    #[test]
    fn the_wheel_moves_three_rows_and_stops_at_the_ends() {
        let mut state = TableState::default().with_selected(0);
        assert!(wheel_select(MouseEventKind::ScrollDown, &mut state, 10));
        assert_eq!(state.selected(), Some(3));
        wheel_select(MouseEventKind::ScrollDown, &mut state, 5);
        assert_eq!(state.selected(), Some(4));
        wheel_select(MouseEventKind::ScrollUp, &mut state, 5);
        assert_eq!(state.selected(), Some(1));
        wheel_select(MouseEventKind::ScrollUp, &mut state, 5);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn other_mouse_events_are_ignored() {
        let mut state = TableState::default().with_selected(2);
        assert!(!wheel_select(MouseEventKind::Moved, &mut state, 10));
        assert_eq!(state.selected(), Some(2));
    }
}

#[cfg(test)]
mod jump_tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn g_and_capital_g_go_to_the_ends() {
        let mut state = TableState::default().with_selected(4);
        assert!(jump_select(&key(KeyCode::Char('G'), KeyModifiers::SHIFT), &mut state, 20, 5));
        assert_eq!(state.selected(), Some(19));
        assert!(jump_select(&key(KeyCode::Char('g'), KeyModifiers::NONE), &mut state, 20, 5));
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn ctrl_f_and_ctrl_b_move_a_page_and_stop_at_the_ends() {
        let mut state = TableState::default().with_selected(3);
        jump_select(&key(KeyCode::Char('f'), KeyModifiers::CONTROL), &mut state, 20, 10);
        assert_eq!(state.selected(), Some(13));
        jump_select(&key(KeyCode::PageDown, KeyModifiers::NONE), &mut state, 20, 10);
        assert_eq!(state.selected(), Some(19));
        jump_select(&key(KeyCode::Char('b'), KeyModifiers::CONTROL), &mut state, 20, 10);
        assert_eq!(state.selected(), Some(9));
        jump_select(&key(KeyCode::PageUp, KeyModifiers::NONE), &mut state, 20, 10);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn plain_f_and_b_are_not_jumps() {
        let mut state = TableState::default().with_selected(3);
        assert!(!jump_select(&key(KeyCode::Char('f'), KeyModifiers::NONE), &mut state, 20, 10));
        assert!(!jump_select(&key(KeyCode::Char('b'), KeyModifiers::NONE), &mut state, 20, 10));
        assert_eq!(state.selected(), Some(3));
    }

    #[test]
    fn an_empty_table_stays_unselected() {
        let mut state = TableState::default();
        assert!(jump_select(&key(KeyCode::Char('G'), KeyModifiers::NONE), &mut state, 0, 5));
        assert_eq!(state.selected(), None);
    }
}
