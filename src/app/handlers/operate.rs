//! Answering the questions that come before an action: yes/no for the
//! destructive ones, a number for scale.

use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let Event::Key(key) = event else { return Ok(None) };
    match &mut st.mode {
        Mode::Confirm { target, action, back, .. } => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let outcome = actions::run(cx.client, target, *action);
                let back = std::mem::replace(back, Box::new(Mode::List));
                st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back };
            }
            KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            _ => {}
        },
        Mode::Scale { target, input, back } => match key.code {
            KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() && input.len() < 5 => input.push(c),
            KeyCode::Enter => {
                if let Ok(replicas) = input.parse::<i32>() {
                    let outcome = actions::run(cx.client, target, Action::Scale(replicas));
                    let back = std::mem::replace(back, Box::new(Mode::List));
                    st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back };
                }
            }
            _ => {}
        },
        Mode::Ports { target, input, back } => match key.code {
            KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) if (c.is_ascii_digit() || c == ':') && input.len() < 11 => input.push(c),
            KeyCode::Enter => {
                let result = portforward::parse_ports(input).and_then(|(local, remote)| {
                    let resource = target.forward_resource().expect("only forwardable kinds open this prompt");
                    portforward::start(cx.active_context, target.namespace.as_deref().unwrap_or("default"), &resource, local, remote)
                });
                let outcome = match result {
                    Ok(forward) => {
                        let text = format!("Forwarding {} (:pf to stop)", forward.label());
                        st.forwards.push(forward);
                        actions::Outcome { text, error: false }
                    }
                    Err(e) => actions::Outcome { text: format!("{e:#}"), error: true },
                };
                let back = std::mem::replace(back, Box::new(Mode::List));
                st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back };
            }
            _ => {}
        },
        Mode::Forwards { state, back } => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            KeyCode::Char('j') | KeyCode::Down => select_next(state, st.forwards.len()),
            KeyCode::Char('k') | KeyCode::Up => select_prev(state, st.forwards.len()),
            KeyCode::Char('D') | KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(i) = state.selected().filter(|i| *i < st.forwards.len()) {
                    st.forwards.remove(i);
                    state.select(Some(i.saturating_sub(usize::from(i >= st.forwards.len()))));
                }
            }
            _ => {}
        },
        _ => {}
    }
    Ok(None)
}
