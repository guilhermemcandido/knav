//! Input handling: one module per family of screens. `dispatch` routes an
//! event to the right one; each `handle` changes the `State` (or ends the
//! session by returning an `Outcome`).

mod command;
mod inspect;
mod list;
mod operate;
mod overview_popups;
mod pickers;

use super::derive::Derived;
use super::*;

/// What a handler may read besides the state it changes.
pub(super) struct Cx<'a> {
    pub terminal: &'a mut ratatui::DefaultTerminal,
    pub catalog: &'a mut Catalog,
    pub client: &'a Client,
    pub config: &'a Config,
    pub active_context: &'a str,
    /// The screen area of the frame the user was looking at.
    pub frame_area: Rect,
    pub row_count: usize,
    pub d: &'a Derived,
}

pub(super) fn dispatch(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    // `s` and the digits sort a popup's table when one has focus.
    if matches!(&event, Event::Key(key) if popup_sort_key(&mut st.mode, key.code)) {
        return Ok(None);
    }
    // These take every key, so the global keys below never fire in them.
    if matches!(st.mode, Mode::NamespacePick { .. } | Mode::Slots { .. } | Mode::Notice { .. }) {
        return pickers::handle(event, st, cx);
    }
    if let Event::Key(key) = &event
        && global_key(key.code, st, cx.active_context)?
    {
        return Ok(None);
    }
    match st.mode {
        Mode::List => list::handle(event, st, cx),
        Mode::Command { .. } | Mode::Search => command::handle(event, st, cx),
        Mode::NamespacePick { .. } | Mode::Slots { .. } | Mode::Notice { .. } | Mode::Context { .. } | Mode::Menu { .. } => pickers::handle(event, st, cx),
        Mode::Events { .. } | Mode::EventDetail { .. } | Mode::ResourcesDetail | Mode::ColumnDetail { .. } => overview_popups::handle(event, st, cx),
        Mode::Confirm { .. } | Mode::Scale { .. } => operate::handle(event, st, cx),
        Mode::Spec { .. } | Mode::Containers { .. } | Mode::NodeDetail { .. } | Mode::Logs { .. } => inspect::handle(event, st, cx),
    }
}

/// Keys that work on every screen except while typing: `c` mouse capture,
/// `?` the commands panel, `:` the command line, `C` the context switcher.
/// True when the key was one of them.
fn global_key(code: KeyCode, st: &mut State, active_context: &str) -> Result<bool> {
    if is_typing(&st.mode) {
        return Ok(false);
    }
    match code {
        // Toggling mouse reporting off hands click-drag text selection (and
        // therefore copy) back to the terminal — the only thing enabling it
        // took away.
        KeyCode::Char('c') => {
            st.mouse_capture_enabled = !st.mouse_capture_enabled;
            if st.mouse_capture_enabled {
                execute!(stdout(), EnableMouseCapture)?;
            } else {
                execute!(stdout(), DisableMouseCapture)?;
            }
        }
        KeyCode::Char('?') => st.show_hints_panel = !st.show_hints_panel,
        // Remembers whatever mode was active as `back`, so Esc returns to
        // exactly where the command line was opened from.
        KeyCode::Char(':') => {
            let back = Box::new(std::mem::replace(&mut st.mode, Mode::List));
            st.mode = Mode::Command { input: String::new(), selected: 0, back };
        }
        KeyCode::Char('C') => open_context_switcher(&mut st.mode, active_context),
        _ => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State::new(icons::IconCache::halfblocks(), Favorites::default())
    }

    #[test]
    fn question_mark_toggles_the_commands_panel() {
        let mut st = state();
        assert!(global_key(KeyCode::Char('?'), &mut st, "ctx").unwrap());
        assert!(st.show_hints_panel);
        assert!(global_key(KeyCode::Char('?'), &mut st, "ctx").unwrap());
        assert!(!st.show_hints_panel);
    }

    #[test]
    fn colon_opens_the_command_line_and_remembers_where_it_came_from() {
        let mut st = state();
        st.mode = Mode::ResourcesDetail;
        assert!(global_key(KeyCode::Char(':'), &mut st, "ctx").unwrap());
        let Mode::Command { back, .. } = &st.mode else { panic!("not the command line") };
        assert!(matches!(**back, Mode::ResourcesDetail));
    }

    #[test]
    fn global_keys_are_plain_characters_while_typing() {
        let mut st = state();
        st.mode = Mode::Search;
        assert!(!global_key(KeyCode::Char('?'), &mut st, "ctx").unwrap());
        assert!(!global_key(KeyCode::Char(':'), &mut st, "ctx").unwrap());
        assert!(!st.show_hints_panel && matches!(st.mode, Mode::Search));
    }

    #[test]
    fn other_keys_are_not_global() {
        let mut st = state();
        assert!(!global_key(KeyCode::Char('x'), &mut st, "ctx").unwrap());
    }
}
