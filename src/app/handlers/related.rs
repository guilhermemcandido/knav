//! The related-objects popup: move over what the selected object relates to and open one.

use super::super::*;
use super::Cx;

/// The entries that can be opened, as (group, entry) positions.
pub(crate) fn selectable(groups: &[k8s::RelationGroup]) -> Vec<(usize, usize)> {
    groups.iter().enumerate().flat_map(|(g, group)| group.entries.iter().enumerate().filter(|(_, e)| e.openable).map(move |(e, _)| (g, e))).collect()
}

pub(super) fn handle(event: Event, st: &mut State, _cx: &mut Cx) -> Result<Option<Outcome>> {
    let mut jump: Option<(ResourceKind, String, Option<String>)> = None;
    if let Mode::Relations { groups, selected, back, .. } = &mut st.mode {
        let all = selectable(groups);
        let last = all.len().saturating_sub(1);
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    let back = std::mem::replace(&mut **back, Mode::List);
                    st.mode = back;
                }
                KeyCode::Char('j') | KeyCode::Down => *selected = (*selected + 1).min(last),
                KeyCode::Char('k') | KeyCode::Up => *selected = selected.saturating_sub(1),
                KeyCode::Char('g') | KeyCode::Home => *selected = 0,
                KeyCode::Char('G') | KeyCode::End => *selected = last,
                KeyCode::Enter => {
                    if let Some((g, e)) = all.get(*selected) {
                        let entry = &groups[*g].entries[*e];
                        if let Some(kind) = ResourceKind::from_owner_kind(&entry.kind) {
                            jump = Some((kind, entry.name.clone(), entry.namespace.clone()));
                        }
                    }
                }
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => *selected = (*selected + 1).min(last),
                MouseEventKind::ScrollUp => *selected = selected.saturating_sub(1),
                _ => {}
            },
            _ => {}
        }
    }
    if let Some((kind, name, namespace)) = jump {
        // Back at the list the way the owner jump does, so Esc returns to where this began.
        st.mode = Mode::List;
        if let Some(namespace) = namespace
            && st.namespace.is_some()
        {
            st.namespace = Some(namespace);
        }
        st.jump_to(kind, name);
    }
    Ok(None)
}
