//! `v` on a Deployment, StatefulSet or DaemonSet: its revisions, what rolling back to
//! one would change, and the rollback itself, after a confirm.

use crate::ops::{NoticeTone, actions};
use super::super::*;
use super::Cx;

/// Columns a sideways step moves.
const SIDEWAYS: usize = 6;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let read_only = st.read_only();
    let Mode::History { target, revisions, cursor, scroll, hscroll, on_diff, back } = &mut st.mode else { return Ok(None) };
    let last = revisions.len().saturating_sub(1);
    let page = usize::from(cx.frame_area.height / 2).max(1);
    // The changes' length, to stop scrolling at their end.
    let running = revisions.iter().find(|r| r.current).map(|r| r.template.as_str()).unwrap_or_default();
    let diff = revisions.get(*cursor).map(|r| crate::ops::edit::diff(running, &r.template)).unwrap_or_default();
    let bottom = diff.len().saturating_sub(1);
    // Far enough sideways that the longest line's end still shows.
    let right_edge = diff.iter().map(|(_, text)| text.chars().count()).max().unwrap_or(0).saturating_sub(SIDEWAYS);
    match event {
        Event::Key(key) => {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => st.mode = std::mem::replace(&mut **back, Mode::List),
                // Tab, or the arrow toward a pane, moves between the revisions and the changes.
                KeyCode::Tab | KeyCode::BackTab => *on_diff = !*on_diff,
                // On the changes, the arrows scroll sideways; past the left edge they go back.
                KeyCode::Right | KeyCode::Char('l') if *on_diff => *hscroll = (*hscroll + SIDEWAYS).min(right_edge),
                KeyCode::Left | KeyCode::Char('h') if *on_diff && *hscroll > 0 => *hscroll = hscroll.saturating_sub(SIDEWAYS),
                KeyCode::Right | KeyCode::Char('l') => *on_diff = true,
                KeyCode::Left | KeyCode::Char('h') => *on_diff = false,
                KeyCode::Down | KeyCode::Char('j') if *on_diff => *scroll = (*scroll + 1).min(bottom),
                KeyCode::Up | KeyCode::Char('k') if *on_diff => *scroll = scroll.saturating_sub(1),
                KeyCode::Char('g') | KeyCode::Home if *on_diff => *scroll = 0,
                KeyCode::Char('G') | KeyCode::End if *on_diff => *scroll = bottom,
                KeyCode::Down | KeyCode::Char('j') => {
                    *cursor = (*cursor + 1).min(last);
                    (*scroll, *hscroll) = (0, 0);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    *cursor = cursor.saturating_sub(1);
                    (*scroll, *hscroll) = (0, 0);
                }
                KeyCode::Char('d') if ctrl => *scroll = (*scroll + page).min(bottom),
                KeyCode::Char('u') if ctrl => *scroll = scroll.saturating_sub(page),
                KeyCode::PageDown => *scroll = (*scroll + page).min(bottom),
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
            _ if crate::app::nav::sideways(&mouse) == Some(true) => *hscroll = (*hscroll + SIDEWAYS).min(right_edge),
            _ if crate::app::nav::sideways(&mouse) == Some(false) => *hscroll = hscroll.saturating_sub(SIDEWAYS),
            MouseEventKind::ScrollDown => *scroll = (*scroll + 3).min(bottom),
            MouseEventKind::ScrollUp => *scroll = scroll.saturating_sub(3),
            _ => {}
        },
        _ => {}
    }
    Ok(None)
}
