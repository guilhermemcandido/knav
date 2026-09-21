//! Looking inside one object: spec tree, containers, node detail, logs.

use super::super::*;
use super::{Cx, logs_mode, open_shell};
use crate::app::derive::Derived;

/// Handles one input event for these modes; `Some` ends the session.
pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let Derived { nodes, node_detail_pods, node_detail_rows, .. } = cx.d;
    let client = cx.client;
    let config = cx.config;
    let frame_area = cx.frame_area;
    let mut shell_request: Option<(String, String, String)> = None;
    match (event, &mut st.mode) {
        // Every key goes to the shell; Ctrl-] (or any key once it has ended) leaves.
        (Event::Key(key), Mode::Shell { session, back, .. }) => {
            // Terminals send Ctrl-] as the byte 0x1d, which arrives as Ctrl-5.
            let leave = matches!(key.code, KeyCode::Char(']' | '5')) && key.modifiers.contains(KeyModifiers::CONTROL);
            if leave || session.exited() {
                st.mode = std::mem::replace(&mut **back, Mode::List);
            } else {
                let bytes = keys::encode(&key, session.app_cursor());
                session.send(&bytes);
            }
        }
        (Event::Key(key), Mode::Yaml { text, scroll, back, .. }) => {
            let last = text.lines().count().saturating_sub(1);
            let page = usize::from(cx.frame_area.height.saturating_sub(8)).max(1);
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
                KeyCode::Char('j') | KeyCode::Down => *scroll = (*scroll + 1).min(last),
                KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
                KeyCode::Char('g') | KeyCode::Home => *scroll = 0,
                KeyCode::Char('G') | KeyCode::End => *scroll = last,
                KeyCode::Char('f') if ctrl => *scroll = (*scroll + page).min(last),
                KeyCode::PageDown => *scroll = (*scroll + page).min(last),
                KeyCode::Char('b') if ctrl => *scroll = scroll.saturating_sub(page),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(page),
                KeyCode::Char('c') => {
                    let outcome = match clipboard::copy(text) {
                        Ok(how) => actions::Outcome { text: format!("Copied the YAML with {how}"), error: false },
                        Err(e) => actions::Outcome { text: format!("{e:#}"), error: true },
                    };
                    let back = std::mem::replace(&mut st.mode, Mode::List);
                    st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back: Box::new(back) };
                }
                _ => {}
            }
        }
        (Event::Mouse(mouse), Mode::Yaml { text, scroll, .. }) => {
            let last = text.lines().count().saturating_sub(1);
            match mouse.kind {
                MouseEventKind::ScrollDown => *scroll = (*scroll + 3).min(last),
                MouseEventKind::ScrollUp => *scroll = scroll.saturating_sub(3),
                _ => {}
            }
        }
        (Event::Key(key), Mode::Spec { viewing: viewing @ Some(_), .. }) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => *viewing = None,
            _ => {}
        },
        (Event::Key(key), Mode::Spec { items, state, expanded_all, leaf_values, viewing, back, .. }) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Char('j') | KeyCode::Down => {
                state.key_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.key_up();
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                state.toggle_selected();
            }
            // Toggles between "everything open" and "everything
            // closed" — `TreeState` only gives us the latter as a
            // single call, so expanding needs walking every
            // identifier ourselves.
            KeyCode::Char('a') => {
                if *expanded_all {
                    state.close_all();
                } else {
                    let mut ids = Vec::new();
                    all_tree_identifiers(items, &mut Vec::new(), &mut ids);
                    for id in ids {
                        state.open(id);
                    }
                }
                *expanded_all = !*expanded_all;
            }
            // Shows the selected leaf's full value, untruncated —
            // a no-op on a branch node (nothing in `leaf_values`
            // for it).
            KeyCode::Char('v') => {
                if let Some(id) = state.selected().last()
                    && let Some((label, value)) = leaf_values.get(id)
                {
                    *viewing = Some((label.clone(), value.clone()));
                }
            }
            _ => {}
        },
        (Event::Mouse(mouse), Mode::Spec { state, .. }) => match mouse.kind {
            MouseEventKind::Down(_) => ui::click_tree(state, mouse.column, mouse.row),
            MouseEventKind::ScrollDown => {
                state.scroll_down(1);
            }
            MouseEventKind::ScrollUp => {
                state.scroll_up(1);
            }
            _ => {}
        },
        (Event::Key(key), Mode::Containers { title, namespace, pod, containers, state, sort, back }) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                st.mode = std::mem::replace(&mut **back, Mode::List);
            }
            KeyCode::Char('j') | KeyCode::Down => select_next(state, containers.len()),
            KeyCode::Char('k') | KeyCode::Up => select_prev(state, containers.len()),
            // A shell in the selected container.
            KeyCode::Char('S') => {
                let shown = sorted_containers(containers, *sort);
                if let Some(container) = state.selected().and_then(|i| shown.get(i)) {
                    shell_request = Some((namespace.clone(), pod.clone(), container.name.clone()));
                }
            }
            // Logs of the selected container; `p` reads the previous run's.
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char('p') => {
                let shown = sorted_containers(containers, *sort);
                if let Some(container) = state.selected().and_then(|i| shown.get(i)) {
                    let snapshot = Mode::Containers {
                        title: title.clone(),
                        namespace: namespace.clone(),
                        pod: pod.clone(),
                        containers: containers.clone(),
                        state: *state,
                        sort: *sort,
                        back: std::mem::replace(back, Box::new(Mode::List)),
                    };
                    st.mode = logs_mode(cx, namespace, pod, &container.name, key.code == KeyCode::Char('p'), snapshot);
                }
            }
            _ => {}
        },
        (Event::Key(key), Mode::NodeDetail { search, editing: editing @ true, state, .. }) => match key.code {
            // Esc clears the search; Enter keeps it and goes back to
            // browsing the matches.
            KeyCode::Esc => {
                search.clear();
                *editing = false;
            }
            KeyCode::Enter => *editing = false,
            KeyCode::Backspace => {
                search.pop();
                state.select(Some(0));
            }
            KeyCode::Char(c) => {
                search.push(c);
                state.select(Some(0));
            }
            _ => {}
        },
        (Event::Key(key), Mode::NodeDetail { name, state, sort, search, editing, back }) => match key.code {
            KeyCode::Char('/') | KeyCode::Char('f') => *editing = true,
            KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Char('d') => {
                if let Some(node) = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str())) {
                    let title = name.clone();
                    open_spec(&mut st.mode, title, node.as_ref());
                }
            }
            KeyCode::Char('e') => {
                if let Some(node) = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str())) {
                    let manifest = k8s::manifest_value(node.as_ref());
                    let back = Box::new(Mode::NodeDetail {
                        name: name.clone(),
                        state: *state,
                        sort: *sort,
                        search: search.clone(),
                        editing: false,
                        back: std::mem::replace(back, Box::new(Mode::List)),
                    });
                    let outcome = edit::edit_resource(cx.terminal, &client, &manifest);
                    st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back };
                }
            }
            KeyCode::Char('j') | KeyCode::Down => select_next(state, node_detail_rows.len()),
            KeyCode::Char('k') | KeyCode::Up => select_prev(state, node_detail_rows.len()),
            KeyCode::Enter => {
                if let Some(pod) = state.selected().and_then(|i| node_detail_pods.get(i)) {
                    let title = title_for(pod.metadata.namespace.as_deref(), pod.metadata.name.as_deref());
                    let pod_namespace = pod.metadata.namespace.clone().unwrap_or_default();
                    let pod_name = pod.metadata.name.clone().unwrap_or_default();
                    let containers = k8s::containers_for(pod);
                    let node_detail_snapshot = Mode::NodeDetail {
                        name: name.clone(),
                        state: *state,
                        sort: *sort,
                        search: search.clone(),
                        editing: false,
                        back: std::mem::replace(back, Box::new(Mode::List)),
                    };
                    st.mode = Mode::Containers {
                        title,
                        namespace: pod_namespace,
                        pod: pod_name,
                        containers,
                        state: TableState::default().with_selected(0),
                        sort: ListSort::default(),
                        back: Box::new(node_detail_snapshot),
                    };
                }
            }
            _ => {}
        },
        (Event::Key(key), Mode::Logs { filter, filter_editing: filter_editing @ true, .. }) => match key.code {
            // Esc while typing clears the filter rather than
            // leaving; Enter confirms and goes back to normal
            // scrolling with it applied — the filtered set is
            // what you land back on, having "scrolled past"
            // everything that didn't match.
            KeyCode::Esc => {
                filter.clear();
                *filter_editing = false;
            }
            KeyCode::Enter => *filter_editing = false,
            KeyCode::Backspace => {
                filter.pop();
            }
            KeyCode::Char(c) => filter.push(c),
            _ => {}
        },
        (Event::Key(key), Mode::Logs { lines, filter, scroll, follow, timestamp_format, order, handle, filter_editing, back, .. }) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                handle.abort();
                st.mode = std::mem::replace(&mut **back, Mode::List);
            }
            KeyCode::Char('j') | KeyCode::Down => ui::logs_scroll_down(frame_area, lines, filter, *order, follow, scroll),
            KeyCode::Char('k') | KeyCode::Up => ui::logs_scroll_up(frame_area, lines, filter, *order, follow, scroll),
            KeyCode::Char('G') => *follow = true,
            KeyCode::Char('/') => *filter_editing = true,
            KeyCode::Char(c) if c == config.keybindings.logs.toggle_timestamp => {
                *timestamp_format = timestamp_format.toggled();
            }
            KeyCode::Char(c) if c == config.keybindings.logs.toggle_order => {
                *order = order.toggled();
                *follow = true;
            }
            _ => {}
        },
        (Event::Mouse(mouse), Mode::Containers { containers, state, .. }) if matches!(mouse.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) => {
            wheel_select(mouse.kind, state, containers.len());
        }
        (Event::Mouse(mouse), Mode::NodeDetail { state, .. }) if matches!(mouse.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) => {
            wheel_select(mouse.kind, state, node_detail_rows.len());
        }
        (Event::Mouse(mouse), Mode::Logs { lines, filter, scroll, follow, order, .. }) => match mouse.kind {
            MouseEventKind::ScrollDown => ui::logs_scroll_down(frame_area, lines, filter, *order, follow, scroll),
            MouseEventKind::ScrollUp => ui::logs_scroll_up(frame_area, lines, filter, *order, follow, scroll),
            _ => {}
        },
        _ => {}
    }
    if let Some((namespace, pod, container)) = shell_request {
        open_shell(st, cx, &namespace, &pod, &container);
    }
    Ok(None)
}
