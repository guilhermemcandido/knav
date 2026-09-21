//! Input handling: one module per family of screens. `dispatch` routes an
//! event to the right one; each `handle` changes the `State` (or ends the
//! session by returning an `Outcome`).

mod command;
mod inspect;
mod list;
mod operate;
mod overview_popups;
mod pickers;
mod settings;
mod themes;

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

/// The log view of one container, opened over `back`.
pub(super) fn logs_mode(cx: &Cx, namespace: &str, pod: &str, container: &str, previous: bool, back: Mode) -> Mode {
    let (rx, handle) = k8s::stream_logs(cx.client.clone(), namespace.to_string(), pod.to_string(), container.to_string(), previous);
    let suffix = if previous { " (previous)" } else { "" };
    Mode::Logs {
        title: format!("{namespace}/{pod}/{container}{suffix}"),
        lines: Vec::new(),
        scroll: 0,
        follow: true,
        timestamp_format: cx.config.logs.timestamp_format,
        order: cx.config.logs.order,
        rx,
        handle,
        filter: String::new(),
        filter_editing: false,
        back: Box::new(back),
    }
}

/// Opens a shell in a container inside knav, over whatever screen is up now.
pub(super) fn open_shell(st: &mut State, cx: &Cx, namespace: &str, pod: &str, container: &str) {
    let inner = ui::shell_inner(cx.frame_area);
    let back = std::mem::replace(&mut st.mode, Mode::List);
    st.mode = match shell::ShellSession::exec(cx.active_context, namespace, pod, container, inner.height, inner.width) {
        Ok(session) => Mode::Shell { title: format!("{namespace}/{pod}/{container}"), session: Box::new(session), back: Box::new(back) },
        Err(e) => Mode::Notice { text: format!("{e:#}"), error: true, back: Box::new(back) },
    };
}

pub(super) fn dispatch(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    // A shell gets every key; only Ctrl-] is ours.
    if matches!(st.mode, Mode::Shell { .. }) {
        return inspect::handle(event, st, cx);
    }
    // Your key bindings: the key pressed becomes the built-in key of the
    // action it is bound to (or is dropped if that action moved elsewhere).
    let event = match event {
        Event::Key(key) => match crate::input::keymap::screen_of(&st.mode, st.current_kind) {
            Some(screen) => match st.keymap.translate(screen, &key) {
                Some(translated) => Event::Key(translated),
                None => return Ok(None),
            },
            None => Event::Key(key),
        },
        other => other,
    };
    // While the help is open it takes the keys: `?`, `q` and Esc close it.
    if st.show_hints_panel {
        if let Event::Key(key) = &event
            && matches!(key.code, KeyCode::Char('?' | 'q') | KeyCode::Esc)
        {
            st.show_hints_panel = false;
        }
        return Ok(None);
    }
    // `s` and the digits sort a popup's table when one has focus.
    if matches!(&event, Event::Key(key) if popup_sort_key(&mut st.mode, key.code)) {
        return Ok(None);
    }
    // g/G/Home/End and paging, in whichever table has focus.
    if let Event::Key(key) = &event
        && !is_typing(&st.mode)
    {
        let page = usize::from(cx.frame_area.height.saturating_sub(8)).max(1);
        if let Some((state, len)) = focused_table(st, cx)
            && jump_select(key, state, len, page)
        {
            return Ok(None);
        }
    }
    // Outside the list, Ctrl combinations are not their plain letters.
    if matches!(&event, Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL)) && !matches!(st.mode, Mode::List) {
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
        Mode::Confirm { .. } | Mode::Scale { .. } | Mode::Ports { .. } | Mode::OpenUrl { .. } => operate::handle(event, st, cx),
        Mode::ThemePicker { .. } => themes::handle(event, st, cx),
        Mode::Settings { .. } => settings::handle(event, st, cx),
        Mode::Spec { .. } | Mode::Yaml { .. } | Mode::Shell { .. } | Mode::Containers { .. } | Mode::NodeDetail { .. } | Mode::Logs { .. } => inspect::handle(event, st, cx),
    }
}

/// The table the keyboard is on, with its row count, for the screens that
/// have one.
fn focused_table<'a>(st: &'a mut State, cx: &Cx) -> Option<(&'a mut TableState, usize)> {
    match &mut st.mode {
        Mode::List if st.current_kind != ResourceKind::Overview => Some((&mut st.table_state, cx.row_count)),
        Mode::Events { filter, search, state, sort, .. } => {
            let len = k8s::filter_events(&cx.d.overview.events, *filter, search, sort.spec).len();
            Some((state, len))
        }
        Mode::NamespacePick { names, filter, state, sort, .. } => {
            let len = filtered_names(names, filter, *sort, &st.favorites).len();
            Some((state, len))
        }
        Mode::Context { contexts, filter, state, sort, .. } => {
            let len = filtered_contexts(contexts, filter, *sort).len();
            Some((state, len))
        }
        Mode::Containers { containers, state, .. } => Some((state, containers.len())),
        Mode::NodeDetail { state, .. } => Some((state, cx.d.node_detail_rows.len())),
        _ => None,
    }
}

/// Keys that work on every screen except while typing: `?` the commands panel, `:` the command line, `C` the context switcher.
/// True when the key was one of them.
fn global_key(code: KeyCode, st: &mut State, active_context: &str) -> Result<bool> {
    if is_typing(&st.mode) {
        return Ok(false);
    }
    match code {
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
        State::new(icons::IconCache::halfblocks(), Favorites::default(), Config::default())
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

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn c_is_left_to_the_screens_that_use_it() {
        let mut st = State::new(icons::IconCache::halfblocks(), Favorites::default(), Config::default());
        assert!(!global_key(KeyCode::Char('c'), &mut st, "ctx").unwrap());
    }
}
