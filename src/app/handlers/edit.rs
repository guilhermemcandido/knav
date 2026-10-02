//! `e`: edit in `$EDITOR`, then review what changed before it is applied.

use crate::ops::{NoticeTone, edit};
use crate::app::mode::EditDraft;
use super::super::*;
use super::Cx;

/// Opens the editor on `manifest`, then shows the changes over `back`. Nothing is
/// applied until the review says so.
pub(super) fn start(st: &mut State, cx: &mut Cx, manifest: &serde_yaml::Value, back: Mode) {
    if st.refuse_if_read_only() {
        return;
    }
    let original = match serde_yaml::to_string(manifest) {
        Ok(text) => text,
        Err(e) => {
            st.mode = Mode::Notice { text: format!("Can't render the manifest: {e}"), tone: NoticeTone::Failed, back: Box::new(back) };
            return;
        }
    };
    let draft = EditDraft { title: crate::app::mode::object_title(manifest), edited: original.clone(), original, error: None };
    open(st, cx, draft, back, None);
}

/// Runs the editor on the draft. `review` is the screen to return to when the editor
/// is aborted, if one is already up.
fn open(st: &mut State, cx: &mut Cx, mut draft: EditDraft, back: Mode, review: Option<(usize, usize)>) {
    let restore = |draft: EditDraft, back: Mode, (scroll, focus): (usize, usize)| {
        let diff = edit::diff(&draft.original, &draft.edited);
        Mode::EditReview { draft, diff, scroll, focus, back: Box::new(back) }
    };
    st.mode = match edit::open_editor(cx.terminal, &draft.edited, draft.error.as_deref()) {
        Err(e) => Mode::Notice { text: format!("{e:#}"), tone: NoticeTone::Failed, back: Box::new(back) },
        Ok(None) => match review {
            Some(at) => restore(draft, back, at),
            None => back,
        },
        Ok(Some(text)) if text.trim() == draft.original.trim() => Mode::Notice { text: "No changes".into(), tone: NoticeTone::Info, back: Box::new(back) },
        Ok(Some(text)) => {
            draft.edited = text;
            draft.error = None;
            restore(draft, back, (0, 0))
        }
    };
}

/// The review has three buttons: Apply, Edit again and Cancel.
const BUTTONS: usize = 3;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let Mode::EditReview { diff, scroll, focus, .. } = &mut st.mode else { return Ok(None) };
    let last = diff.len().saturating_sub(1);
    let page = usize::from(cx.frame_area.height.saturating_sub(10)).max(1);
    let pressed = match event {
        Event::Key(key) => match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                *scroll = (*scroll + 1).min(last);
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                *scroll = scroll.saturating_sub(1);
                None
            }
            KeyCode::PageDown => {
                *scroll = (*scroll + page).min(last);
                None
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                *scroll = (*scroll + (page / 2).max(1)).min(last);
                None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                *scroll = scroll.saturating_sub((page / 2).max(1));
                None
            }
            KeyCode::PageUp => {
                *scroll = scroll.saturating_sub(page);
                None
            }
            KeyCode::Char('g') | KeyCode::Home => {
                *scroll = 0;
                None
            }
            KeyCode::Char('G') | KeyCode::End => {
                *scroll = last;
                None
            }
            KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => {
                *focus = (*focus + BUTTONS - 1) % BUTTONS;
                None
            }
            KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => {
                *focus = (*focus + 1) % BUTTONS;
                None
            }
            KeyCode::Enter => Some(*focus),
            KeyCode::Char('a') => Some(0),
            KeyCode::Char('e') => Some(1),
            KeyCode::Char('q') | KeyCode::Esc => Some(2),
            _ => None,
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollDown => {
                *scroll = (*scroll + 3).min(last);
                None
            }
            MouseEventKind::ScrollUp => {
                *scroll = scroll.saturating_sub(3);
                None
            }
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                let pos = ratatui::layout::Position { x: mouse.column, y: mouse.row };
                ui::edit_review_buttons(cx.frame_area).iter().position(|r| r.contains(pos))
            }
            _ => None,
        },
        _ => None,
    };
    match pressed {
        Some(0) => crate::app::jobs::apply_edit(st, cx.client),
        Some(1) => {
            let Mode::EditReview { draft, scroll, focus, back, .. } = std::mem::replace(&mut st.mode, Mode::List) else { return Ok(None) };
            open(st, cx, draft, *back, Some((scroll, focus)));
        }
        Some(_) => {
            if let Mode::EditReview { back, .. } = std::mem::replace(&mut st.mode, Mode::List) {
                st.mode = *back;
            }
        }
        None => {}
    }
    Ok(None)
}
