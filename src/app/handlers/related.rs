//! The relations diagram: move between boxes, recentre on one, open one's list.

use crate::ops::NoticeTone;
use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let mut open: Option<(ResourceKind, Option<String>, String)> = None;
    let mut info: Option<serde_yaml::Value> = None;
    let mut copy_note: Option<crate::ops::Outcome> = None;
    // A box the diagram hadn't loaded is fetched in the background, then the key comes
    // back here with it in `pending`.
    let mut fetch: Option<(crate::app::jobs::BoxId, Option<serde_yaml::Value>, crossterm::event::KeyEvent)> = None;
    let mut missing: Option<String> = None;
    let mut pending = st.surroundings.take();
    if let Mode::Relations { target, all, graph, selected, previous, zoom, back } = &mut st.mode {
        // The box's object: in what the diagram holds, else in what was just fetched for it.
        let mut lookup = |node: &k8s::relations::GraphNode, all: &[serde_yaml::Value]| -> Result<serde_yaml::Value, Option<k8s::surroundings::Fetched>> {
            if let Some(found) = k8s::relations::find_manifest(all, &node.kind, node.namespace.as_deref(), &node.name) {
                return Ok(found);
            }
            let wanted = crate::app::jobs::key_of(&node.kind, node.namespace.as_deref(), &node.name);
            match pending.take().filter(|(key, _)| *key == wanted) {
                Some((_, fetched)) => k8s::relations::find_manifest(&fetched.manifests, &node.kind, node.namespace.as_deref(), &node.name).ok_or(Some(fetched)),
                None => Err(None),
            }
        };
        let id = |node: &k8s::relations::GraphNode| (node.kind.clone(), node.namespace.clone(), node.name.clone());
        let layout = ui::graph_layout(graph, *zoom);
        let go = |direction: ui::Move, selected: &mut usize| {
            if let Some(to) = ui::graph_neighbor(graph, &layout, *selected, direction, *zoom) {
                *selected = to;
            }
        };
        // Space moves to a box with what surrounds it, fetched for it.
        let mut follow: Option<(serde_yaml::Value, k8s::surroundings::Fetched)> = None;
        let mut close = false;
        let mut restore = false;
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Char('q') => close = true,
                // Esc steps back through what Space followed, like Backspace, and closes
                // once there is nothing left.
                KeyCode::Esc if !previous.is_empty() => restore = true,
                KeyCode::Esc => close = true,
                KeyCode::Left | KeyCode::Char('h') => go(ui::Move::Left, selected),
                KeyCode::Right | KeyCode::Char('l') => go(ui::Move::Right, selected),
                KeyCode::Up | KeyCode::Char('k') => go(ui::Move::Up, selected),
                KeyCode::Down | KeyCode::Char('j') => go(ui::Move::Down, selected),
                // Space follows the map: the diagram moves to that object, with what
                // surrounds it fetched the way R does.
                KeyCode::Char(' ') => {
                    if let Some(node) = graph.nodes.get(*selected).filter(|_| *selected != 0) {
                        let wanted = crate::app::jobs::key_of(&node.kind, node.namespace.as_deref(), &node.name);
                        match pending.take().filter(|(k, _)| *k == wanted) {
                            Some((_, fetched)) => match k8s::relations::find_manifest(&fetched.manifests, &node.kind, node.namespace.as_deref(), &node.name) {
                                Some(object) => follow = Some((object, fetched)),
                                None => missing = Some(format!("Couldn't load {} {}", node.kind, node.name)),
                            },
                            None => fetch = Some((id(node), k8s::relations::find_manifest(all, &node.kind, node.namespace.as_deref(), &node.name), key)),
                        }
                    }
                }
                KeyCode::Backspace => restore = true,
                // Zoom out (more boxes fit) or back in (more detail).
                KeyCode::Char('-') | KeyCode::Char('_') => *zoom = ui::zoom_out(*zoom),
                KeyCode::Char('+') | KeyCode::Char('=') => *zoom = ui::zoom_in(*zoom),
                // `m` copies the diagram as Mermaid text.
                KeyCode::Char('m') => copy_note = Some(match clipboard::copy(&k8s::relations::mermaid(graph)) {
                    Ok(how) => crate::ops::Outcome { text: format!("Copied the diagram as Mermaid{}", clipboard::how_note(how)), tone: NoticeTone::Done },
                    Err(e) => crate::ops::Outcome { text: format!("{e:#}"), tone: NoticeTone::Failed },
                }),
                // Enter shows the object's info over the diagram; Enter there goes to its list.
                KeyCode::Enter => {
                    if *selected == 0 {
                        info = Some(target.clone());
                    } else if let Some(node) = graph.nodes.get(*selected) {
                        match lookup(node, all) {
                            Ok(found) => info = Some(found),
                            Err(Some(_)) => missing = Some(format!("Couldn't load {} {}", node.kind, node.name)),
                            Err(None) => fetch = Some((id(node), None, key)),
                        }
                    }
                }
                // `o` goes straight to the object's list.
                KeyCode::Char('o') => {
                    if let Some(node) = graph.nodes.get(*selected)
                        && let Some(kind) = cx.catalog.list_for(&node.kind)
                    {
                        open = Some((kind, node.namespace.clone(), node.name.clone()));
                    }
                }
                _ => {}
            },
            // Ctrl and the wheel zoom (a trackpad pinch arrives this way); the plain wheel moves.
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
                        if again && hit == 0 {
                            info = Some(target.clone());
                        } else if again && let Some(node) = graph.nodes.get(hit) {
                            match lookup(node, all) {
                                Ok(found) => info = Some(found),
                                Err(Some(_)) => missing = Some(format!("Couldn't load {} {}", node.kind, node.name)),
                                Err(None) => fetch = Some((id(node), None, crossterm::event::KeyEvent::from(KeyCode::Enter))),
                            }
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        // Backspace goes back to the last diagram with the objects it held.
        let next = if restore {
            previous.pop()
        } else {
            follow.map(|(object, fetched)| {
                let around = super::list::surrounding_manifests(cx.pod_store, cx.dep_store, fetched.manifests, &object);
                previous.push((target.clone(), std::mem::take(all)));
                (object, around)
            })
        };
        if let Some((next, around)) = next {
            *graph = k8s::relations::graph(&next, &k8s::relations::relations(&next, &around));
            *target = next;
            *all = around;
            *selected = 0;
        }
        if close {
            let back = std::mem::replace(&mut **back, Mode::List);
            st.mode = back;
        }
    }
    if let Some((node, known, key)) = fetch {
        let api = cx.catalog.api_for_kind(&node.0);
        crate::app::jobs::fetch_related(st, cx.client, node, api, known, key);
        return Ok(None);
    }
    if let Some(text) = missing {
        let back = std::mem::replace(&mut st.mode, Mode::List);
        st.mode = Mode::Notice { text, tone: NoticeTone::Info, back: Box::new(back) };
        return Ok(None);
    }
    if let Some(outcome) = copy_note {
        let back = std::mem::replace(&mut st.mode, Mode::List);
        st.mode = Mode::Notice { text: outcome.text, tone: outcome.tone, back: Box::new(back) };
    }
    if let Some(manifest) = info {
        st.reveal = true;
        let view = cx.catalog.view_for(&cx.config.extensions.enabled, &manifest);
        let sections = k8s::details::details(&manifest, &cx.d.overview.events, crate::app::live_usage(cx.d.pod_usage.as_deref(), cx.d.usage.as_ref()), true, view);
        let back = std::mem::replace(&mut st.mode, Mode::List);
        st.mode = Mode::Details { manifest, sections, scroll: 0, hscroll: 0, back: Box::new(back) };
    }
    if let Some((kind, namespace, name)) = open {
        // The diagram is one of the steps q or Esc undoes, like a drill-down.
        let snap = st.list_snapshot();
        st.back_stack.push(Step::Mode(Box::new(std::mem::replace(&mut st.mode, Mode::List)), snap));
        st.jump_to_object(kind, namespace.as_deref(), &name);
        // `jump_to_object` pushed the list too, which the step above already covers.
        st.back_stack.pop();
    }
    Ok(None)
}
