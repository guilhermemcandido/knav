//! Looking inside one object: spec tree, containers, node detail, logs.

use crate::ops::NoticeTone;
use super::super::*;
use super::{Cx, logs_mode, open_shell};
use crate::app::derive::Derived;

/// Handles one input event for these modes; `Some` ends the session.
/// Columns a sideways step moves.
const SIDEWAYS: usize = 6;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let Derived { nodes, node_detail_pods, node_detail_rows, .. } = cx.d;
    let frame_area = cx.frame_area;
    let mut shell_request: Option<(String, String, String)> = None;
    // A debug container for (namespace, pod, container), asked about first.
    let mut debug_request: Option<(String, String, String)> = None;
    let read_only = st.read_only();
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
        (Event::Key(key), Mode::Yaml { text, scroll, hscroll, back, .. }) => {
            let last = text.lines().count().saturating_sub(1);
            let right_edge = text.lines().map(|l| l.chars().count()).max().unwrap_or(0).saturating_sub(SIDEWAYS);
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
                KeyCode::Left | KeyCode::Char('h') => *hscroll = hscroll.saturating_sub(SIDEWAYS),
                KeyCode::Right | KeyCode::Char('l') => *hscroll = (*hscroll + SIDEWAYS).min(right_edge),
                KeyCode::Char('d') if ctrl => *scroll = (*scroll + (page / 2).max(1)).min(last),
                KeyCode::Char('u') if ctrl => *scroll = scroll.saturating_sub((page / 2).max(1)),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(page),
                KeyCode::Char('c') => {
                    let outcome = match clipboard::copy(text) {
                        Ok(how) => crate::ops::Outcome { text: format!("Copied the YAML{}", clipboard::how_note(how)), tone: NoticeTone::Done },
                        Err(e) => crate::ops::Outcome { text: format!("{e:#}"), tone: NoticeTone::Failed },
                    };
                    let back = std::mem::replace(&mut st.mode, Mode::List);
                    st.mode = Mode::Notice { text: outcome.text, tone: outcome.tone, back: Box::new(back) };
                }
                _ => {}
            }
        }
        (Event::Mouse(mouse), Mode::Yaml { text, scroll, hscroll, .. }) => {
            let last = text.lines().count().saturating_sub(1);
            let right_edge = text.lines().map(|l| l.chars().count()).max().unwrap_or(0).saturating_sub(SIDEWAYS);
            match mouse.kind {
                _ if crate::app::nav::sideways(&mouse) == Some(true) => *hscroll = (*hscroll + SIDEWAYS).min(right_edge),
                _ if crate::app::nav::sideways(&mouse) == Some(false) => *hscroll = hscroll.saturating_sub(SIDEWAYS),
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
            // Toggles all open or all closed. `TreeState` can only close all in one call,
            // so opening walks every identifier.
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
            // The selected leaf's full value; nothing on a branch.
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
                let shown = sorted_containers(containers, sort.spec);
                if let Some(container) = state.selected().and_then(|i| shown.get(i)) {
                    shell_request = Some((namespace.clone(), pod.clone(), container.name.clone()));
                }
            }
            KeyCode::Char('X') => {
                let shown = sorted_containers(containers, sort.spec);
                if let Some(container) = state.selected().and_then(|i| shown.get(i)) {
                    debug_request = Some((namespace.clone(), pod.clone(), container.name.clone()));
                }
            }
            // Logs of the selected container; `p` reads the previous run's.
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char('p') => {
                let shown = sorted_containers(containers, sort.spec);
                if let Some(container) = state.selected().and_then(|i| shown.get(i)) {
                    let previous = key.code == KeyCode::Char('p');
                    let no_previous = previous && container.restarts == 0;
                    let snapshot = Mode::Containers {
                        title: title.clone(),
                        namespace: namespace.clone(),
                        pod: pod.clone(),
                        containers: containers.clone(),
                        state: *state,
                        sort: *sort,
                        back: std::mem::replace(back, Box::new(Mode::List)),
                    };
                    st.mode = if no_previous {
                        Mode::Notice { text: format!("{} has not restarted, so there is no previous run to show", container.name), tone: NoticeTone::Info, back: Box::new(snapshot) }
                    } else {
                        logs_mode(cx, namespace, pod, &container.name, previous, snapshot)
                    };
                }
            }
            _ => {}
        },
        (Event::Key(key), Mode::NodeDetail { search, editing: editing @ true, state, .. }) => {
            if super::edit_line(key.code, search, editing) {
                state.select(Some(0));
            }
        }
        (Event::Key(key), Mode::NodeDetail { name, state, sort, search, editing, back }) => match key.code {
            KeyCode::Char('/') | KeyCode::Char('f') => *editing = true,
            KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Char('d') => {
                if let Some(node) = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str())) {
                    let title = name.clone();
                    open_spec(&mut st.mode, title, node.as_ref());
                }
            }
            KeyCode::Char('e') if read_only => {
                st.refuse_if_read_only();
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
                    super::edit::start(st, cx, &manifest, *back);
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
        (Event::Key(key), Mode::Logs { filter, filter_editing: filter_editing @ true, .. }) => {
            super::edit_line(key.code, filter, filter_editing);
        }
        (Event::Key(key), Mode::Logs { title, lines, filter, scroll, follow, timestamp_format, order, filter_editing, back, .. }) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                st.mode = std::mem::replace(&mut **back, Mode::List);
            }
            KeyCode::Char('j') | KeyCode::Down => ui::logs_scroll_down(frame_area, lines, filter, *timestamp_format, *order, follow, scroll),
            KeyCode::Char('k') | KeyCode::Up => ui::logs_scroll_up(frame_area, lines, filter, *timestamp_format, *order, follow, scroll),
            // A page (PageDown, Ctrl-f) or half of one (Ctrl-d), a line at a time.
            code @ (KeyCode::PageDown | KeyCode::PageUp | KeyCode::Char('f' | 'b' | 'd' | 'u'))
                if matches!(code, KeyCode::PageDown | KeyCode::PageUp) || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let page = usize::from(frame_area.height.saturating_sub(6)).max(1);
                let (lines_to_move, down) = match code {
                    KeyCode::PageDown | KeyCode::Char('f') => (page, true),
                    KeyCode::PageUp | KeyCode::Char('b') => (page, false),
                    KeyCode::Char('d') => ((page / 2).max(1), true),
                    _ => ((page / 2).max(1), false),
                };
                for _ in 0..lines_to_move {
                    if down {
                        ui::logs_scroll_down(frame_area, lines, filter, *timestamp_format, *order, follow, scroll);
                    } else {
                        ui::logs_scroll_up(frame_area, lines, filter, *timestamp_format, *order, follow, scroll);
                    }
                }
            }
            KeyCode::Char('G') => *follow = true,
            KeyCode::Char('/') => *filter_editing = true,
            KeyCode::Char('t') => {
                *timestamp_format = timestamp_format.toggled();
            }
            KeyCode::Char('o') => {
                *order = order.toggled();
                *follow = true;
            }
            // `w` saves the lines shown (the filter and order applied) to a file.
            KeyCode::Char('w') => {
                let text = ui::logs_text(lines, filter, *order);
                let count = text.lines().count();
                let filtered = if filter.is_empty() { "" } else { " matching the filter" };
                let (text, tone) = match crate::ops::logfile::save(&cx.config.logs.save_dir, title, &text) {
                    Ok(path) => (format!("Saved {count} lines{filtered} to {}", crate::ops::logfile::shown(&path)), NoticeTone::Done),
                    Err(e) => (format!("{e:#}"), NoticeTone::Failed),
                };
                let back = std::mem::replace(&mut st.mode, Mode::List);
                st.mode = Mode::Notice { text, tone, back: Box::new(back) };
            }
            // `c` copies the lines shown (the filter and order applied) to the clipboard.
            KeyCode::Char('c') => {
                let text = ui::logs_text(lines, filter, *order);
                let count = text.lines().count();
                let outcome = match clipboard::copy(&text) {
                    Ok(how) => crate::ops::Outcome { text: format!("Copied {count} log lines{}", clipboard::how_note(how)), tone: NoticeTone::Done },
                    Err(e) => crate::ops::Outcome { text: format!("{e:#}"), tone: NoticeTone::Failed },
                };
                let back = std::mem::replace(&mut st.mode, Mode::List);
                st.mode = Mode::Notice { text: outcome.text, tone: outcome.tone, back: Box::new(back) };
            }
            _ => {}
        },
        (Event::Mouse(mouse), Mode::Containers { containers, state, .. }) if matches!(mouse.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) => {
            wheel_select(mouse.kind, state, containers.len());
        }
        (Event::Mouse(mouse), Mode::NodeDetail { state, .. }) if matches!(mouse.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) => {
            wheel_select(mouse.kind, state, node_detail_rows.len());
        }
        (Event::Mouse(mouse), Mode::Logs { lines, filter, scroll, follow, order, timestamp_format, .. }) => match mouse.kind {
            MouseEventKind::ScrollDown => ui::logs_scroll_down(frame_area, lines, filter, *timestamp_format, *order, follow, scroll),
            MouseEventKind::ScrollUp => ui::logs_scroll_up(frame_area, lines, filter, *timestamp_format, *order, follow, scroll),
            _ => {}
        },
        _ => {}
    }
    if let Some((namespace, pod, container)) = shell_request {
        open_shell(st, cx, &namespace, &pod, &container);
    }
    if let Some((namespace, name, container)) = debug_request
        && let Some(found) = cx.d.pods.iter().find(|p| p.metadata.namespace.as_deref() == Some(namespace.as_str()) && p.metadata.name.as_deref() == Some(name.as_str()))
    {
        super::ask_debug(st, &k8s::manifest_value(found.as_ref()), &container);
    }
    Ok(None)
}
