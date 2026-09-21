//! The relations diagram: move between boxes, recentre on one, open one's list.

use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let mut open: Option<(ResourceKind, Option<String>, String)> = None;
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
                KeyCode::Enter => {
                    if let Some(node) = graph.nodes.get(*selected).filter(|_| *selected != 0) {
                        recentre = k8s::relations::find_manifest(all, &node.kind, node.namespace.as_deref(), &node.name);
                    }
                }
                KeyCode::Backspace => restore = true,
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
    let _ = cx;
    if let Some((kind, namespace, name)) = open {
        // Back at the list the way the owner jump does, so Esc returns to where this began.
        st.mode = Mode::List;
        st.jump_to_object(kind, namespace.as_deref(), &name);
    }
    Ok(None)
}
