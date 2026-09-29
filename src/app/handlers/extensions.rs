//! The extensions browser (`E`): toggle a third-party kind's category/icon/view
//! on or off, live-search by name. Its own screen, not a Settings tab, so it's
//! reachable from anywhere the same way `C` reaches the context switcher.

use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let mut enable_change: Option<Vec<String>> = None;
    let mut close = false;
    if let Mode::Extensions { filter, filter_editing, state, error, back } = &mut st.mode {
        // The visible order (filtered, bundled-first, alphabetical): what
        // `state.selected()` indexes into, and the same list the draw side
        // builds its rows from, so a toggle always lands on the extension
        // actually on screen.
        let order = crate::extensions::visible_order(&cx.catalog.extensions.loaded, filter);
        let len = order.len().max(1);
        match event {
            // While typing a filter: every key is text, `/` included, same as Logs.
            Event::Key(key) if *filter_editing => {
                if super::edit_line(key.code, filter, filter_editing) {
                    state.select(Some(0));
                }
            }
            Event::Key(key) => {
                *error = None;
                let at = state.selected().unwrap_or(0).min(order.len().saturating_sub(1));
                let loaded = order.get(at).and_then(|&i| cx.catalog.extensions.loaded.get(i));
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => close = true,
                    KeyCode::Char('/') => *filter_editing = true,
                    KeyCode::Char('j') | KeyCode::Down => select_next(state, len),
                    KeyCode::Char('k') | KeyCode::Up => select_prev(state, len),
                    KeyCode::Char('g') | KeyCode::Home => state.select(Some(0)),
                    KeyCode::Char('G') | KeyCode::End => state.select(Some(len.saturating_sub(1))),
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        if let Some(loaded) = loaded {
                            if loaded.error.is_some() {
                                *error = Some(format!("{}: {}", loaded.id, loaded.error.as_deref().unwrap_or("")));
                            } else {
                                let mut enabled = cx.config.extensions.enabled.clone();
                                match enabled.iter().position(|e| e == &loaded.id) {
                                    Some(pos) => {
                                        enabled.remove(pos);
                                    }
                                    None => enabled.push(loaded.id.clone()),
                                }
                                enable_change = Some(enabled);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse) if !wheel_select(mouse.kind, state, len) && matches!(mouse.kind, MouseEventKind::Down(_)) => {
                let bundled_count = order.iter().take_while(|&&i| cx.catalog.extensions.loaded[i].bundled).count();
                if let Some(index) = ui::extension_row_at(cx.frame_area, bundled_count, order.len(), state.offset(), mouse.row) {
                    state.select(Some(index));
                }
            }
            _ => {}
        }
        if close {
            let back = std::mem::replace(back, Box::new(Mode::List));
            st.mode = *back;
        }
    }
    if let Some(enabled) = enable_change {
        let mut array = toml_edit::Array::new();
        for name in &enabled {
            array.push(name.clone());
        }
        let value = (!enabled.is_empty()).then(|| toml_edit::Value::Array(array));
        match crate::config::settings::save(&Config::path(), "extensions.enabled", value) {
            Ok(config) => st.reload(config),
            Err(e) => {
                if let Mode::Extensions { error, .. } = &mut st.mode {
                    *error = Some(format!("{e:#}"));
                }
            }
        }
    }
    Ok(None)
}

/// Opens the extensions browser over whatever is showing.
pub(super) fn open(st: &mut State) {
    let back = std::mem::replace(&mut st.mode, Mode::List);
    st.mode = Mode::Extensions { filter: String::new(), filter_editing: false, state: TableState::default().with_selected(0), error: None, back: Box::new(back) };
}
