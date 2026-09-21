//! Answering the questions that come before an action: yes/no for the
//! destructive ones, a number for scale.

use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
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
                    Some(text) => Mode::Notice { text: text.into(), error: false, back },
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
                    Err(e) => Mode::Notice { text: format!("{e:#}"), error: true, back },
                };
            }
            KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            _ => {}
        },
        Mode::Scale { targets, input, back } => match key.code {
            KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() && input.len() < 5 => input.push(c),
            KeyCode::Enter => {
                if let Ok(replicas) = input.parse::<i32>() {
                    let targets = std::mem::take(targets);
                    let back = std::mem::replace(back, Box::new(Mode::List));
                    crate::app::jobs::run_action(st, cx.client, targets, Action::Scale(replicas), back);
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
                        let (context, namespace, resource, back) = (cx.active_context.to_string(), namespace.to_string(), resource, std::mem::replace(back, Box::new(Mode::List)));
                        crate::app::jobs::start_forward(st, &context, &namespace, &resource, &address, local, remote, back);
                    }
                }
            }
        }
        _ => {}
    }
    Ok(None)
}
