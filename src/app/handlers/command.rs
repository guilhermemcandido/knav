//! The `>` command line and the `/` search bar.

use super::super::*;
use super::Cx;

/// Handles one input event for these modes; `Some` ends the session.
pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let catalog = &mut *cx.catalog;
    let active_context = cx.active_context;
    match (event, &mut st.mode) {
        (Event::Key(key), Mode::Command { input, selected, back }) => match key.code {
            KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::Down => {
                let len = command_suggestions(input, &catalog.crds).len();
                *selected = (*selected + 1).min(len.saturating_sub(1));
            }
            KeyCode::Enter => {
                let cmd = input.trim().to_lowercase();
                // The highlighted autocomplete suggestion wins
                // when there is one; `from_command` is only the
                // fallback for an exact alias that didn't happen
                // to fuzzy-score into the visible list.
                let suggestions = command_suggestions(input, &catalog.crds);
                let highlighted = suggestions.get(*selected).map(|s| s.cmd);
                if matches!(highlighted, Some(Cmd::Quit)) || matches!(cmd.as_str(), "q" | "quit" | "exit") {
                    return Ok(Some(Outcome::Quit));
                }
                if matches!(highlighted, Some(Cmd::Forwards)) {
                    st.mode = Mode::Forwards { state: TableState::default().with_selected(0), back: std::mem::replace(back, Box::new(Mode::List)) };
                } else if matches!(highlighted, Some(Cmd::Events)) {
                    st.mode = Mode::Events { filter: k8s::EventFilter::All, search: String::new(), editing: false, state: TableState::default().with_selected(0), sort: ListSort::default() };
                } else if is_context_command(&cmd) || matches!(highlighted, Some(Cmd::Context)) {
                    let mut opened = std::mem::replace(&mut **back, Mode::List);
                    open_context_switcher(&mut opened, active_context);
                    st.mode = opened;
                } else if let Some(kind) = match highlighted {
                    Some(Cmd::Kind(k)) => Some(k),
                    _ => k8s::ResourceKind::from_command(&cmd),
                } {
                    st.switch_kind(kind);
                    st.mode = Mode::List;
                } else {
                    st.mode = std::mem::replace(&mut **back, Mode::List);
                }
            }
            KeyCode::Backspace => {
                input.pop();
                *selected = 0;
            }
            KeyCode::Char(c) => {
                input.push(c);
                *selected = 0;
            }
            _ => {}
        },
        (Event::Key(key), Mode::Search) => match key.code {
            KeyCode::Esc => {
                st.search.clear();
                st.mode = Mode::List;
            }
            KeyCode::Enter => st.mode = Mode::List,
            KeyCode::Backspace => {
                st.search.pop();
            }
            KeyCode::Char(c) => st.search.push(c),
            _ => {}
        },
        _ => {}
    }
    Ok(None)
}
