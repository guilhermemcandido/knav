//! Choosers: namespaces and their number keys, contexts, the resource menu, notices.

use super::super::*;
use super::Cx;

/// Handles one input event for these modes; `Some` ends the session.
pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let active_context = cx.active_context;
    let frame_area = cx.frame_area;
    match (event, &mut st.mode) {
        // Any key (or click) closes a notice, checked before the
        // global keys below so they don't also fire on that press.
        (Event::Key(key), Mode::NamespacePick { filter, editing: editing @ true, state, .. }) => {
            if super::edit_line(key.code, filter, editing) {
                state.select(Some(0));
            }
        }
        (Event::Key(key), Mode::NamespacePick { names, filter, editing, state, sort, back }) => {
            let mut chosen: Option<String> = None;
            let mut close = false;
            // A number gives the highlighted namespace that key right here;
            // `d` (or Delete) takes its key away.
            let highlighted = state.selected().and_then(|i| filtered_names(names, filter, *sort, &st.favorites).get(i).map(|n| (*n).clone()));
            match key.code {
                KeyCode::Char(c @ '1'..='9') => {
                    if let Some(name) = &highlighted {
                        st.favorites.assign(c as usize - '0' as usize, name);
                        st.favorites.save(active_context);
                    }
                }
                KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
                    if let Some(key) = highlighted.as_ref().and_then(|n| st.favorites.key_of(n)) {
                        st.favorites.clear(key);
                        st.favorites.save(active_context);
                    }
                }
                KeyCode::Char('q') | KeyCode::Esc => close = true,
                KeyCode::Char('/') | KeyCode::Char('f') => *editing = true,
                KeyCode::Char('j') | KeyCode::Down => select_next(state, filtered_names(names, filter, *sort, &st.favorites).len()),
                KeyCode::Char('k') | KeyCode::Up => select_prev(state, filtered_names(names, filter, *sort, &st.favorites).len()),
                KeyCode::Enter => chosen = state.selected().and_then(|i| filtered_names(names, filter, *sort, &st.favorites).get(i).map(|n| (*n).clone())),
                _ => {}
            }
            if let Some(name) = chosen {
                // Straight on to choosing its key; Esc from there goes
                // back to the view this was opened from.
                let back = std::mem::replace(&mut **back, Mode::List);
                let mut next = key_picker(name, &st.favorites);
                if let Mode::Slots { back: slot_back, .. } = &mut next {
                    *slot_back = Box::new(back);
                }
                st.mode = next;
            } else if close {
                st.mode = std::mem::replace(&mut **back, Mode::List);
            }
        }
        (Event::Mouse(mouse), Mode::NamespacePick { names, filter, state, sort, .. }) if matches!(mouse.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) => {
            wheel_select(mouse.kind, state, filtered_names(names, filter, *sort, &st.favorites).len());
        }
        // A click selects a row, or (on the chip strip below) puts the
        // highlighted namespace on that number.
        (Event::Mouse(mouse), Mode::NamespacePick { names, filter, state, sort, .. }) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
            let matches: Vec<String> = filtered_names(names, filter, *sort, &st.favorites).into_iter().cloned().collect();
            if let Some(key) = ui::slot_chip_at(frame_area, mouse.column, mouse.row) {
                if let Some(name) = state.selected().and_then(|i| matches.get(i)) {
                    st.favorites.assign(key, name);
                    st.favorites.save(active_context);
                }
            } else if let Some(idx) = ui::event_row_at(frame_area, matches.len(), state.offset(), mouse.row) {
                state.select(Some(idx));
            }
        }
        (Event::Key(key), Mode::Slots { namespace, selected, back }) => {
            let mut assign_to = None;
            let mut close = false;
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => close = true,
                KeyCode::Char('j') | KeyCode::Down => *selected = (*selected + 1).min(favorites::SLOTS - 1),
                KeyCode::Char('k') | KeyCode::Up => *selected = selected.saturating_sub(1),
                KeyCode::Char(c @ '1'..='9') => assign_to = c.to_digit(10).map(|d| d as usize),
                KeyCode::Enter => assign_to = Some(*selected + 1),
                KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
                    st.favorites.clear(*selected + 1);
                    st.favorites.save(active_context);
                }
                _ => {}
            }
            if let Some(key_number) = assign_to {
                st.favorites.assign(key_number, namespace);
                st.favorites.save(active_context);
                close = true;
            }
            if close {
                st.mode = std::mem::replace(&mut **back, Mode::List);
            }
        }
        (Event::Key(_), Mode::Notice { back, .. }) => st.mode = std::mem::replace(&mut **back, Mode::List),
        (Event::Mouse(m), Mode::Notice { back, .. }) if matches!(m.kind, MouseEventKind::Down(_)) => {
            st.mode = std::mem::replace(&mut **back, Mode::List)
        }
        // No separate typing mode: any letter filters immediately, arrows/wheel/click
        // scroll, same as typing into a search engine's box instead of a command
        // line. Esc clears the filter first, then (pressed again) backs out; `q`
        // only backs out on an empty filter, since a context can contain a 'q'.
        (Event::Key(key), Mode::Context { contexts, filter, editing: _, state, error, sort, back }) => match key.code {
            KeyCode::Esc => {
                if filter.is_empty() {
                    st.mode = std::mem::replace(&mut **back, Mode::List);
                } else {
                    filter.clear();
                    state.select(Some(0));
                }
            }
            KeyCode::Char('q') if filter.is_empty() => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Up => select_prev(state, filtered_contexts(contexts, filter, *sort).len()),
            KeyCode::Down => select_next(state, filtered_contexts(contexts, filter, *sort).len()),
            KeyCode::Backspace => {
                filter.pop();
                state.select(Some(0));
                *error = None;
            }
            KeyCode::Char(c) => {
                filter.push(c);
                state.select(Some(0));
                *error = None;
            }
            KeyCode::Enter => {
                let name = state.selected().and_then(|i| filtered_contexts(contexts, filter, *sort).get(i).map(|c| c.name.clone()));
                if let Some(name) = name {
                    if name == active_context {
                        st.mode = std::mem::replace(&mut **back, Mode::List);
                    } else {
                        *error = None;
                        crate::app::jobs::check_context(st, name);
                    }
                }
            }
            _ => {}
        },
        (Event::Mouse(mouse), Mode::Context { contexts, filter, state, sort, .. }) if matches!(mouse.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) => {
            wheel_select(mouse.kind, state, filtered_contexts(contexts, filter, *sort).len());
        }
        (Event::Mouse(mouse), Mode::Context { contexts, filter, state, error, sort, back, .. }) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
            let matches = filtered_contexts(contexts, filter, *sort);
            if let Some(idx) = ui::event_row_at(frame_area, matches.len(), state.offset(), mouse.row) {
                state.select(Some(idx));
                let name = matches[idx].name.clone();
                if name == active_context {
                    st.mode = std::mem::replace(&mut **back, Mode::List);
                } else {
                    *error = None;
                    crate::app::jobs::check_context(st, name);
                }
            }
        }
        _ => {}
    }
    Ok(None)
}
