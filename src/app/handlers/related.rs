//! The relations diagram: move between boxes, recentre on one, open one's list.

use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let mut open: Option<(ResourceKind, Option<String>, String)> = None;
    let mut info: Option<serde_yaml::Value> = None;
    let mut copy_note: Option<actions::Outcome> = None;
    if let Mode::Relations { target, all, graph, selected, previous, back } = &mut st.mode {
        let layout = ui::graph_layout(graph);
        let go = |direction: ui::Move, selected: &mut usize| {
            if let Some(to) = ui::graph_neighbor(graph, &layout, *selected, direction) {
                *selected = to;
            }
        };
        let mut recentre: Option<serde_yaml::Value> = None;
        let mut close = false;
        let mut restore = false;
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => close = true,
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
                // `m` copies the diagram as Mermaid text.
                KeyCode::Char('m') => copy_note = Some(match clipboard::copy(&k8s::relations::mermaid(graph)) {
                    Ok(how) => actions::Outcome { text: format!("Copied the diagram as Mermaid with {how}"), error: false },
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
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => go(ui::Move::Down, selected),
                MouseEventKind::ScrollUp => go(ui::Move::Up, selected),
                // A click selects a box; a second click on it soon after shows its info.
                MouseEventKind::Down(_) => {
                    if let Some(hit) = ui::graph_hit(ui::relations_inner(cx.frame_area), graph, *selected, mouse.column, mouse.row) {
                        let now = std::time::Instant::now();
                        let again = st.last_click.is_some_and(|(at, prev)| prev == 1000 + hit && now.duration_since(at) < std::time::Duration::from_millis(crate::config::tunables::tunables().double_click_ms));
                        st.last_click = if again { None } else { Some((now, 1000 + hit)) };
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
        let sections = k8s::details::details(&manifest, &cx.d.overview.events, true);
        let back = std::mem::replace(&mut st.mode, Mode::List);
        st.mode = Mode::Details { manifest, sections, scroll: 0, hscroll: 0, back: Box::new(back) };
    }
    if let Some((kind, namespace, name)) = open {
        // Back at the list the way the owner jump does, so Esc returns to where this began.
        st.mode = Mode::List;
        st.jump_to_object(kind, namespace.as_deref(), &name);
    }
    Ok(None)
}
