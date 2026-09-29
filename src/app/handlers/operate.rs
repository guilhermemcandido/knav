//! Answering the questions before an action: yes or no, or a number for scale.

use crate::ops::NoticeTone;
use super::super::*;
use super::Cx;

const MAX_DIGITS: usize = 5;

/// One up or down from `input`, never below 0 or past `MAX_DIGITS` digits.
fn step(input: &str, up: bool) -> i64 {
    let now = input.parse::<i64>().unwrap_or(0);
    if up { (now + 1).min(99_999) } else { (now - 1).max(0) }
}

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let Event::Key(key) = event else { return Ok(None) };
    match &mut st.mode {
        Mode::Confirm { spec, targets, action, back } => match key.code {
            // A destructive action needs an explicit `y`; Enter only confirms the mild ones.
            KeyCode::Char('y') | KeyCode::Enter if key.code == KeyCode::Char('y') || !spec.danger => {
                let (targets, action) = (std::mem::take(targets), *action);
                let back = std::mem::replace(back, Box::new(Mode::List));
                crate::app::jobs::run_action(st, cx.client, targets, action, back);
            }
            KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            _ => {}
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
        Mode::OpenUrl { url, back, .. } => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let result = portforward::open_in_browser(url);
                let back = std::mem::replace(back, Box::new(Mode::List));
                st.mode = match result {
                    Ok(()) => *back,
                    Err(e) => Mode::Notice { text: format!("{e:#}"), tone: NoticeTone::Failed, back },
                };
            }
            KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            _ => {}
        },
        Mode::Scale { targets, input, fresh, back } => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => st.mode = std::mem::replace(&mut **back, Mode::List),
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
    use super::step;

    #[test]
    fn steps_stay_between_zero_and_the_digit_limit() {
        assert_eq!(step("2", true), 3);
        assert_eq!(step("0", false), 0);
        assert_eq!(step("", true), 1);
        assert_eq!(step("99999", true), 99_999);
    }
}
