//! The relations diagram: move between boxes, recentre on one, open one's list.

use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let mut open: Option<(ResourceKind, Option<String>, String)> = None;
    let mut info: Option<serde_yaml::Value> = None;
    let mut copy_note: Option<actions::Outcome> = None;
    if let Mode::Relations { target, all, graph, selected, previous, zoom, back } = &mut st.mode {
        let layout = ui::graph_layout(graph, *zoom);
        let go = |direction: ui::Move, selected: &mut usize| {
            if let Some(to) = ui::graph_neighbor(graph, &layout, *selected, direction, *zoom) {
                *selected = to;
            }
        };
        let mut recentre: Option<serde_yaml::Value> = None;
        let mut close = false;
        let mut restore = false;
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Char('q') => close = true,
                // Esc steps back through what Space followed, same as Backspace; only
                // closes once there is nothing left to step back through.
                KeyCode::Esc if !previous.is_empty() => restore = true,
                KeyCode::Esc => close = true,
                KeyCode::Left | KeyCode::Char('h') => go(ui::Move::Left, selected),
                KeyCode::Right | KeyCode::Char('l') => go(ui::Move::Right, selected),
                KeyCode::Up | KeyCode::Char('k') => go(ui::Move::Up, selected),
                KeyCode::Down | KeyCode::Char('j') => go(ui::Move::Down, selected),
                // Space follows the map: the diagram moves to that object.
                KeyCode::Char(' ') => {
                    if let Some(node) = graph.nodes.get(*selected).filter(|_| *selected != 0) {
                        recentre = k8s::relations::find_manifest(all, &node.kind, node.namespace.as_deref(), &node.name);
                    }
                }
                KeyCode::Backspace => restore = true,
                // Zoom the boxes out (more fit on screen) or back in (more detail per box).
                KeyCode::Char('-') | KeyCode::Char('_') => *zoom = ui::zoom_out(*zoom),
                KeyCode::Char('+') | KeyCode::Char('=') => *zoom = ui::zoom_in(*zoom),
                // `m` copies the diagram as Mermaid text.
                KeyCode::Char('m') => copy_note = Some(match clipboard::copy(&k8s::relations::mermaid(graph)) {
                    Ok(how) => actions::Outcome { text: format!("Copied the diagram as Mermaid{}", clipboard::how_note(how)), error: false },
                    Err(e) => actions::Outcome { text: format!("{e:#}"), error: true },
                }),
                // Enter shows the object's info over the diagram; Enter there goes to its list.
                KeyCode::Enter => {
                    if let Some(node) = graph.nodes.get(*selected) {
                        info = if *selected == 0 { Some(target.clone()) } else { k8s::relations::find_manifest(all, &node.kind, node.namespace.as_deref(), &node.name) };
                    }
                }
                // `o` goes straight to the object's list.
                KeyCode::Char('o') => {
                    if let Some(node) = graph.nodes.get(*selected)
                        && let Some(kind) = ResourceKind::from_owner_kind(&node.kind)
                    {
                        open = Some((kind, node.namespace.clone(), node.name.clone()));
                    }
                }
                _ => {}
            },
            // Ctrl+wheel zooms (a pinch on a trackpad reaches the terminal this way too); a plain
            // wheel moves the selection.
            Event::Mouse(mouse) if mouse.modifiers.contains(KeyModifiers::CONTROL) => match mouse.kind {
                MouseEventKind::ScrollUp => *zoom = ui::zoom_in(*zoom),
                MouseEventKind::ScrollDown => *zoom = ui::zoom_out(*zoom),
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => go(ui::Move::Down, selected),
                MouseEventKind::ScrollUp => go(ui::Move::Up, selected),
                // A click selects a box; a second click on it soon after shows its info.
                MouseEventKind::Down(_) => {
                    if let Some(hit) = ui::graph_hit(ui::relations_inner(cx.frame_area), graph, *selected, mouse.column, mouse.row, *zoom) {
                        let again = state::double_click(&mut st.last_click, 1000 + hit);
                        *selected = hit;
                        if again && let Some(node) = graph.nodes.get(hit) {
                            info = if hit == 0 { Some(target.clone()) } else { k8s::relations::find_manifest(all, &node.kind, node.namespace.as_deref(), &node.name) };
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        let next = if restore { previous.pop() } else { recentre.inspect(|_| previous.push(target.clone())) };
        if let Some(next) = next {
            *graph = k8s::relations::graph(&next, &k8s::relations::relations(&next, all));
            *target = next;
            *selected = 0;
        }
        if close {
            let back = std::mem::replace(&mut **back, Mode::List);
            st.mode = back;
        }
    }
    if let Some(outcome) = copy_note {
        let back = std::mem::replace(&mut st.mode, Mode::List);
        st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back: Box::new(back) };
    }
    if let Some(manifest) = info {
        st.reveal = true;
        let view = cx.catalog.view_for(&cx.config.extensions.enabled, &manifest);
        let sections = k8s::details::details(&manifest, &cx.d.overview.events, true, view);
        let back = std::mem::replace(&mut st.mode, Mode::List);
        st.mode = Mode::Details { manifest, sections, scroll: 0, hscroll: 0, back: Box::new(back) };
    }
    if let Some((kind, namespace, name)) = open {
        // The diagram itself is one of the steps q/Esc undoes, same as any drill-down.
        let snap = st.list_snapshot();
        st.back_stack.push(Step::Mode(Box::new(std::mem::replace(&mut st.mode, Mode::List)), snap));
        st.jump_to_object(kind, namespace.as_deref(), &name);
        // `jump_to_object` also pushes the pre-Relations list; the step just pushed already
        // covers getting back there, so drop that redundant entry.
        st.back_stack.pop();
    }
    Ok(None)
}
