//! Answering the questions before an action: yes or no, or a number for scale.

use crate::ops::NoticeTone;
use super::super::*;
use super::Cx;

/// A key on a yes/no dialog: `Some(true)` for yes, `Some(false)` for no, `None` when
/// it only moved the focus between the buttons (or did nothing).
fn answer(code: KeyCode, yes: &mut bool) -> Option<bool> {
    match code {
        KeyCode::Char('y') => Some(true),
        KeyCode::Char('n' | 'q') | KeyCode::Esc => Some(false),
        KeyCode::Enter => Some(*yes),
        KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('h' | 'l') => {
            *yes = !*yes;
            None
        }
        _ => None,
    }
}

const MAX_DIGITS: usize = 5;

/// One up or down from `input`, never below 0 or past `MAX_DIGITS` digits.
fn step(input: &str, up: bool) -> i64 {
    let now = input.parse::<i64>().unwrap_or(0);
    if up { (now + 1).min(99_999) } else { (now - 1).max(0) }
}

/// A click on a dialog button, as the key that presses it.
fn clicked_button(st: &mut State, area: Rect, column: u16, row: u16) -> Option<KeyCode> {
    let at = |b: ui::DialogButtons| {
        let pos = ratatui::layout::Position { x: column, y: row };
        if b.yes.contains(pos) { Some(true) } else if b.no.contains(pos) { Some(false) } else { None }
    };
    let as_key = |yes: bool| if yes { KeyCode::Char('y') } else { KeyCode::Esc };
    match &mut st.mode {
        Mode::Confirm { spec, .. } => at(ui::confirm_buttons(area, spec)).map(as_key),
        Mode::OpenUrl { text, url, .. } => at(ui::confirm_buttons(area, &actions::open_url_spec(text, url))).map(as_key),
        Mode::Scale { targets, input, yes, .. } => {
            let hit = at(ui::scale_buttons(area, &crate::app::draw::scale_view(targets, input, *yes)))?;
            *yes = hit;
            Some(KeyCode::Enter)
        }
        _ => None,
    }
}

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    if let Event::Mouse(mouse) = event {
        if matches!(mouse.kind, MouseEventKind::Down(crossterm::event::MouseButton::Left))
            && let Some(code) = clicked_button(st, cx.frame_area, mouse.column, mouse.row)
        {
            return handle(Event::Key(code.into()), st, cx);
        }
        return Ok(None);
    }
    let Event::Key(key) = event else { return Ok(None) };
    match &mut st.mode {
        Mode::Confirm { targets, action, yes, back, .. } => match answer(key.code, yes) {
            Some(true) => {
                let (targets, action) = (std::mem::take(targets), *action);
                let back = std::mem::replace(back, Box::new(Mode::List));
                crate::app::jobs::run_action(st, cx.client, targets, action, back);
            }
            Some(false) => st.mode = std::mem::replace(&mut **back, Mode::List),
            None => {}
        },
        Mode::Working { job, back } => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                let note = job.cancel_note;
                let back = std::mem::replace(back, Box::new(Mode::List));
                st.mode = match note {
                    Some(text) => Mode::Notice { text: text.into(), tone: NoticeTone::Info, back },
                    None => *back,
                };
            }
        }
        Mode::OpenUrl { url, yes, back, .. } => match answer(key.code, yes) {
            Some(true) => {
                let result = portforward::open_in_browser(url);
                let back = std::mem::replace(back, Box::new(Mode::List));
                st.mode = match result {
                    Ok(()) => *back,
                    Err(e) => Mode::Notice { text: format!("{e:#}"), tone: NoticeTone::Failed, back },
                };
            }
            Some(false) => st.mode = std::mem::replace(&mut **back, Mode::List),
            None => {}
        },
        Mode::Scale { targets, input, fresh, yes, back } => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Tab | KeyCode::BackTab => *yes = !*yes,
            KeyCode::Enter if !*yes => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Up | KeyCode::Right | KeyCode::Down | KeyCode::Left | KeyCode::Char('+' | '-' | 'k' | 'j' | 'l' | 'h') => {
                let up = matches!(key.code, KeyCode::Up | KeyCode::Right | KeyCode::Char('+' | 'k' | 'l'));
                *input = step(input, up).to_string();
                *fresh = false;
            }
            KeyCode::Backspace => {
                input.pop();
                *fresh = false;
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if std::mem::take(fresh) {
                    input.clear();
                }
                if input.len() < MAX_DIGITS {
                    input.push(c);
                }
                // No leading zeros: `05` is 5.
                *input = input.parse::<i64>().map(|n| n.to_string()).unwrap_or_default();
            }
            KeyCode::Enter => {
                let current = targets.first().map(|t| t.replicas()).filter(|now| targets.iter().all(|t| t.replicas() == *now));
                match input.parse::<i32>() {
                    // Nothing to change: just close.
                    Ok(replicas) if current == Some(i64::from(replicas)) => st.mode = std::mem::replace(&mut **back, Mode::List),
                    Ok(replicas) => {
                        let targets = std::mem::take(targets);
                        let back = std::mem::replace(back, Box::new(Mode::List));
                        crate::app::jobs::run_action(st, cx.client, targets, Action::Scale(replicas), back);
                    }
                    Err(_) => {}
                }
            }
            _ => {}
        },
        Mode::Ports { target, form, back } => {
            use portforward::Field;
            let mut submit = false;
            match key.code {
                KeyCode::Esc => {
                    st.mode = std::mem::replace(&mut **back, Mode::List);
                    return Ok(None);
                }
                KeyCode::Tab | KeyCode::Down => form.next(),
                KeyCode::BackTab | KeyCode::Up => form.prev(),
                KeyCode::Left | KeyCode::Right if matches!(form.focus, Field::Ok | Field::Cancel) => {
                    form.focus = if form.focus == Field::Ok { Field::Cancel } else { Field::Ok };
                }
                KeyCode::Backspace => form.backspace(),
                KeyCode::Char(c) => form.type_char(c),
                KeyCode::Enter if form.focus == Field::Cancel => {
                    st.mode = std::mem::replace(&mut **back, Mode::List);
                    return Ok(None);
                }
                KeyCode::Enter => submit = true,
                _ => {}
            }
            if submit {
                match form.parse() {
                    Err(e) => form.error = Some(format!("{e:#}")),
                    Ok((local, remote, address)) => {
                        let resource = target.forward_resource().expect("only forwardable kinds open this dialog");
                        let namespace = target.namespace.as_deref().unwrap_or("default");
                        let request = portforward::ForwardRequest { context: cx.active_context.to_string(), namespace: namespace.to_string(), resource, address, local, remote };
                        let back = std::mem::replace(back, Box::new(Mode::List));
                        crate::app::jobs::start_forward(st, request, back);
                    }
                }
            }
        }
        _ => {}
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{answer, step};
    use crossterm::event::KeyCode;

    #[test]
    fn arrows_move_the_focus_and_enter_presses_it() {
        let mut yes = false;
        assert_eq!(answer(KeyCode::Enter, &mut yes), Some(false), "starts on Cancel");
        assert_eq!(answer(KeyCode::Left, &mut yes), None);
        assert_eq!(answer(KeyCode::Enter, &mut yes), Some(true));
        assert_eq!(answer(KeyCode::Char('n'), &mut yes), Some(false), "n cancels wherever the focus is");
        assert_eq!(answer(KeyCode::Tab, &mut yes), None);
        assert_eq!(answer(KeyCode::Char('y'), &mut yes), Some(true), "y confirms wherever the focus is");
    }

    #[test]
    fn steps_stay_between_zero_and_the_digit_limit() {
        assert_eq!(step("2", true), 3);
        assert_eq!(step("0", false), 0);
        assert_eq!(step("", true), 1);
        assert_eq!(step("99999", true), 99_999);
    }
}
