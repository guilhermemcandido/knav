//! `v` on a Deployment: its revisions, what rolling back to one would change, and
//! the rollback itself, after a confirm.

use crate::ops::{NoticeTone, actions};
use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let read_only = st.read_only();
    let Mode::History { target, revisions, cursor, scroll, back } = &mut st.mode else { return Ok(None) };
    let last = revisions.len().saturating_sub(1);
    let page = usize::from(cx.frame_area.height / 2).max(1);
    match event {
        Event::Key(key) => {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => st.mode = std::mem::replace(&mut **back, Mode::List),
                KeyCode::Down | KeyCode::Char('j') => {
                    *cursor = (*cursor + 1).min(last);
                    *scroll = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    *cursor = cursor.saturating_sub(1);
                    *scroll = 0;
                }
                KeyCode::Char('d') if ctrl => *scroll += page,
                KeyCode::Char('u') if ctrl => *scroll = scroll.saturating_sub(page),
                KeyCode::PageDown => *scroll += page,
                KeyCode::PageUp => *scroll = scroll.saturating_sub(page),
                KeyCode::Enter => {
                    let Some(revision) = revisions.get(*cursor) else { return Ok(None) };
                    if revision.current {
                        let text = format!("Revision {} is the one running", revision.number);
                        let here = std::mem::replace(&mut st.mode, Mode::List);
                        st.mode = Mode::Notice { text, tone: NoticeTone::Info, back: Box::new(here) };
                    } else if read_only {
                        st.refuse_if_read_only();
                    } else {
                        let action = actions::Action::Rollback(revision.number);
                        let targets = vec![target.clone()];
                        if let Some(spec) = actions::confirm_spec(action, &targets) {
                            let here = std::mem::replace(&mut st.mode, Mode::List);
                            st.mode = Mode::Confirm { spec, targets, action, yes: true, back: Box::new(here) };
                        }
                    }
                }
                _ => {}
            }
        }
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollDown => *scroll += 3,
            MouseEventKind::ScrollUp => *scroll = scroll.saturating_sub(3),
            _ => {}
        },
        _ => {}
    }
    Ok(None)
}
