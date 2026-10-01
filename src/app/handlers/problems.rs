//! `!`: everything in the cluster that needs a look. Enter opens the object's list
//! with it selected.

use super::super::*;
use super::Cx;
use crate::app::derive::Derived;

pub(super) fn open(st: &mut State) {
    let back = std::mem::replace(&mut st.mode, Mode::List);
    st.mode = Mode::Problems { state: TableState::default().with_selected(0), search: String::new(), editing: false, back: Box::new(back) };
}

/// The problems that match `search` (on kind, name, reason or detail), in order.
pub(in crate::app) fn matching<'a>(problems: &'a [std::sync::Arc<k8s::problems::Problem>], search: &str) -> Vec<&'a k8s::problems::Problem> {
    let search = search.to_lowercase();
    problems
        .iter()
        .map(|p| p.as_ref())
        .filter(|p| search.is_empty() || [p.kind.label(), &p.place(), &p.reason, &p.detail].iter().any(|t| t.to_lowercase().contains(&search)))
        .collect()
}

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let Derived { problems, .. } = cx.d;
    let Mode::Problems { state, search, editing, back } = &mut st.mode else { return Ok(None) };
    let shown = matching(problems, search);
    let mut open: Option<usize> = None;
    match event {
        Event::Key(key) if *editing => {
            if super::edit_line(key.code, search, editing) {
                state.select(Some(0));
            }
        }
        Event::Key(key) => match key.code {
            KeyCode::Esc if !search.is_empty() => {
                search.clear();
                state.select(Some(0));
            }
            KeyCode::Esc | KeyCode::Char('q') => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Char('/') | KeyCode::Char('f') => *editing = true,
            KeyCode::Down | KeyCode::Char('j') => select_next(state, shown.len()),
            KeyCode::Up | KeyCode::Char('k') => select_prev(state, shown.len()),
            KeyCode::Char('g') | KeyCode::Home => state.select(Some(0)),
            KeyCode::Char('G') | KeyCode::End => state.select(Some(shown.len().saturating_sub(1))),
            KeyCode::Enter => open = state.selected(),
            _ => {}
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollDown => select_next(state, shown.len()),
            MouseEventKind::ScrollUp => select_prev(state, shown.len()),
            // A click selects a row; a second click on it soon after opens it.
            MouseEventKind::Down(_) => {
                if let Some(index) = ui::event_row_at(cx.frame_area, shown.len(), state.offset(), mouse.row) {
                    let again = state::double_click(&mut st.last_click, 2000 + index);
                    state.select(Some(index));
                    if again {
                        open = Some(index);
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
    if let Some(problem) = open.and_then(|i| shown.get(i)) {
        // Problems is a step q or Esc undoes, like the diagram's open list.
        let (kind, namespace, name) = (problem.kind, problem.namespace.clone(), problem.name.clone());
        let snap = st.list_snapshot();
        st.back_stack.push(Step::Mode(Box::new(std::mem::replace(&mut st.mode, Mode::List)), snap));
        st.jump_to_object(kind, namespace.as_deref(), &name);
        st.back_stack.pop();
    }
    Ok(None)
}
