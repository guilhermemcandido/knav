//! Input handling: one module per family of screens. `dispatch` routes an
//! event to the right one; each `handle` changes the `State` (or ends the
//! session by returning an `Outcome`).

mod command;
mod inspect;
mod list;
pub(crate) use list::selected_manifest;
mod operate;
mod overview_popups;
mod details;
mod pickers;
mod related;
mod settings;
mod sidebar;
mod themes;

use super::derive::Derived;
use super::*;

/// One key of a search box being typed in: Esc clears and leaves, Enter keeps and leaves,
/// characters and Backspace edit. `true` when the text changed, so the caller can reset its selection.
pub(super) fn edit_line(code: KeyCode, text: &mut String, editing: &mut bool) -> bool {
    match code {
        KeyCode::Esc => {
            *editing = false;
            !std::mem::take(text).is_empty()
        }
        KeyCode::Enter => {
            *editing = false;
            false
        }
        KeyCode::Backspace => text.pop().is_some(),
        KeyCode::Char(c) => {
            text.push(c);
            true
        }
        _ => false,
    }
}

/// What a handler may read besides the state it changes.
pub(super) struct Cx<'a> {
    pub terminal: &'a mut ratatui::DefaultTerminal,
    pub catalog: &'a mut Catalog,
    pub pod_store: &'a k8s::PodKept,
    pub dep_store: &'a k8s::DeploymentKept,
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
        handles: vec![crate::mode::AbortOnDrop(handle)],
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
    // `Q` quits from anywhere (`q` only goes back), except while text is being typed.
    if let Event::Key(key) = &event
        && key.code == KeyCode::Char('Q')
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !owns_keys(&st.mode)
    {
        return Ok(Some(Outcome::Quit));
    }
    // `H` goes Home from anywhere, closing whatever is open.
    if let Event::Key(key) = &event
        && key.code == KeyCode::Char('H')
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !owns_keys(&st.mode)
    {
        st.switch_kind(ResourceKind::Overview);
        st.mode = Mode::List;
        st.show_hints_panel = false;
        return Ok(None);
    }
    // While the help is open it takes the keys: `?`, `q` and Esc close it.
    if st.show_hints_panel {
        if let Event::Key(key) = &event
            && matches!(key.code, KeyCode::Char('?' | 'q') | KeyCode::Esc)
        {
            st.show_hints_panel = false;
        }
        return Ok(None);
    }
    // The resource sidebar comes first: `m`, Shift-Left, its own keys and clicks.
    if sidebar::handle(&event, st, cx) {
        return Ok(None);
    }
    // The info panel beside the list: Shift-Right hands it the keys, Shift-Left takes them back.
    if st.info_panel
        && matches!(st.mode, Mode::List)
        && st.current_kind != ResourceKind::Overview
        && cx.frame_area.width >= crate::ui::SIDE_PANEL_MIN_WIDTH
        && let Event::Key(key) = &event
    {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = usize::from(cx.frame_area.height.saturating_sub(8)).max(1);
        if key.code == KeyCode::Right && shift {
            st.info_focus = true;
            return Ok(None);
        }
        if st.info_focus {
            let handled = match key.code {
                KeyCode::Left if shift => {
                    st.info_focus = false;
                    true
                }
                KeyCode::Esc => {
                    st.info_focus = false;
                    true
                }
                // Enter opens the same summary full screen, where it was.
                KeyCode::Enter => {
                    if let Some(manifest) = selected_manifest(st, cx.d, &mut *cx.catalog, cx.client) {
                        let sections = k8s::details::details(&manifest, &cx.d.overview.events, st.reveal);
                        st.mode = Mode::Details { manifest, sections, scroll: st.info_scroll, hscroll: st.info_hscroll, back: Box::new(Mode::List) };
                    }
                    true
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    st.info_hscroll = st.info_hscroll.saturating_sub(6);
                    true
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    st.info_hscroll += 6;
                    true
                }
                KeyCode::Char('x') => {
                    st.reveal = !st.reveal;
                    true
                }
                KeyCode::Char('i') => {
                    st.info_panel = false;
                    st.info_focus = false;
                    true
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    st.info_scroll += 1;
                    true
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    st.info_scroll = st.info_scroll.saturating_sub(1);
                    true
                }
                KeyCode::PageDown => {
                    st.info_scroll += page;
                    true
                }
                KeyCode::Char('f') if ctrl => {
                    st.info_scroll += page;
                    true
                }
                KeyCode::PageUp => {
                    st.info_scroll = st.info_scroll.saturating_sub(page);
                    true
                }
                KeyCode::Char('b') if ctrl => {
                    st.info_scroll = st.info_scroll.saturating_sub(page);
                    true
                }
                KeyCode::Char('g') | KeyCode::Home => {
                    st.info_scroll = 0;
                    true
                }
                KeyCode::Char('G') | KeyCode::End => {
                    st.info_scroll = usize::MAX / 2;
                    true
                }
                // Anything else (`:`, `?`, ...) works as usual.
                _ => false,
            };
            if handled {
                return Ok(None);
            }
        }
    }
    // Outside the list, Ctrl combinations are not their plain letters.
    if matches!(&event, Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL)) && !matches!(st.mode, Mode::List | Mode::Settings { capture: Some(_), .. }) {
        return Ok(None);
    }
    // `s` and the digits sort a popup's table when one has focus.
    if matches!(&event, Event::Key(key) if popup_sort_key(&mut st.mode, key.code)) {
        return Ok(None);
    }
    // g/G/Home/End and paging, in whichever table has focus.
    if let Event::Key(key) = &event
        && !owns_keys(&st.mode)
    {
        let page = usize::from(cx.frame_area.height.saturating_sub(8)).max(1);
        if let Some((state, len)) = focused_table(st, cx)
            && jump_select(key, state, len, page)
        {
            return Ok(None);
        }
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
        Mode::NamespacePick { .. } | Mode::Slots { .. } | Mode::Notice { .. } | Mode::Context { .. } => pickers::handle(event, st, cx),
        Mode::Events { .. } | Mode::EventDetail { .. } | Mode::ResourcesDetail | Mode::ColumnDetail { .. } => overview_popups::handle(event, st, cx),
        Mode::Confirm { .. } | Mode::Working { .. } | Mode::Scale { .. } | Mode::Ports { .. } | Mode::OpenUrl { .. } => operate::handle(event, st, cx),
        Mode::ThemePicker { .. } => themes::handle(event, st, cx),
        Mode::Settings { .. } => settings::handle(event, st, cx),
        Mode::Relations { .. } => related::handle(event, st, cx),
        Mode::Details { .. } => details::handle(event, st, cx),
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
    if owns_keys(&st.mode) {
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
