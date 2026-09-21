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
        _ => {}
    }
    Ok(None)
}
