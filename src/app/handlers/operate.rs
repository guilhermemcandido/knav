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
                        let result = portforward::start(cx.active_context, namespace, &resource, &address, local, remote);
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
                }
            }
        }
        _ => {}
    }
    Ok(None)
}
