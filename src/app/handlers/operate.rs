//! Answering the questions that come before an action: yes/no for the
//! destructive ones, a number for scale.

use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let Event::Key(key) = event else { return Ok(None) };
    match &mut st.mode {
        Mode::Confirm { targets, action, back, .. } => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let outcome = actions::run_many(cx.client, targets, *action);
                if !outcome.error {
                    st.marked.clear();
                }
                let back = std::mem::replace(back, Box::new(Mode::List));
                st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back };
            }
            KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            _ => {}
        },
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
                    let outcome = actions::run_many(cx.client, targets, Action::Scale(replicas));
                    if !outcome.error {
                        st.marked.clear();
                    }
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
                let back = std::mem::replace(back, Box::new(Mode::List));
                st.mode = match result {
                    Ok(forward) => {
                        let mut text = format!("Forwarding {} (:pf to stop)", forward.label());
                        let url = forward.url();
                        st.forwards.push(forward);
                        if !cx.config.portforward.open_browser {
                            // Not opening by itself: ask.
                            text.push_str(&format!("\nOpen {url} in the browser?"));
                            Mode::OpenUrl { text, url, back }
                        } else {
                            if let Err(e) = portforward::open_in_browser(&url) {
                                text.push_str(&format!("\n{e:#}"));
                            }
                            Mode::Notice { text, error: false, back }
                        }
                    }
                    Err(e) => Mode::Notice { text: format!("{e:#}"), error: true, back },
                };
            }
            _ => {}
        },
        _ => {}
    }
    Ok(None)
}
