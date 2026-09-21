//! The settings screen: change a value with the arrows or Enter (numbers and
//! colours can be typed), reset one with `r`. Each change is saved to
//! `config.toml` and applied at once.

use super::super::*;
use super::Cx;
use crate::config::settings::{self, Kind, Setting};

/// What to do to the config file once the screen's own state is released.
enum Change {
    Set(String, String),
    Reset(String),
}

/// The next value for a setting stepped by `direction` (-1, +1; 10x for `big`).
fn stepped(setting: &Setting, current: &str, direction: i64, big: bool) -> Option<String> {
    match &setting.kind {
        Kind::Choice(options) => {
            let at = options.iter().position(|o| o == current).unwrap_or(0) as i64;
            let next = (at + direction).rem_euclid(options.len() as i64) as usize;
            Some(options[next].clone())
        }
        Kind::Bool => Some(if current == "true" { "false" } else { "true" }.to_string()),
        Kind::Number { min, max } => {
            let value: i64 = current.parse().ok()?;
            let step = if big { 10 } else { 1 };
            Some((value + direction * step).clamp(*min, *max).to_string())
        }
        Kind::Color | Kind::Keys => None,
    }
}

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let mut change: Option<Change> = None;
    let mut close = false;
    let current_theme = crate::theme::theme();
    if let Mode::Settings { settings, state, editing, capture, error, back } = &mut st.mode {
        let len = settings.len();
        let setting = state.selected().and_then(|i| settings.get(i)).cloned();
        let current = setting.as_ref().map(|s| settings::current(cx.config, &current_theme, s)).unwrap_or_default();
        match event {
            // Picking a key: the first key pressed is the candidate, then it is
            // confirmed, added to the current keys, re-picked or dropped.
            Event::Key(key) if capture.is_some() => {
                let picking = capture.as_mut().expect("checked above");
                match picking.pressed.clone() {
                    None => picking.pressed = Some(crate::input::keymap::format_key(crate::input::keymap::KeySpec::of(&key))),
                    Some(pressed) => match key.code {
                        KeyCode::Esc | KeyCode::Char('n') => *capture = None,
                        KeyCode::Backspace => *picking = KeyCapture::default(),
                        KeyCode::Enter | KeyCode::Char('y' | 'a') => {
                            let new = if pressed == "," { "comma".to_string() } else { pressed };
                            let text = if key.code == KeyCode::Char('a') && !current.split(", ").any(|k| k == new) { format!("{current}, {new}") } else { new };
                            match setting.as_ref().map(|s| (s, settings::typed_value(cx.config, s, &text))) {
                                Some((s, Ok(_))) => {
                                    change = Some(Change::Set(s.path.clone(), text));
                                    *capture = None;
                                }
                                Some((_, Err(e))) => picking.problem = Some(format!("{e:#}")),
                                None => {}
                            }
                        }
                        _ => {}
                    },
                }
            }
            Event::Mouse(_) if capture.is_some() => {}
            Event::Key(key) if editing.is_some() => {
                let input = editing.as_mut().expect("checked above");
                match key.code {
                    KeyCode::Esc => {
                        *editing = None;
                        *error = None;
                        // Drop the live preview of what was being typed.
                        settings::apply(cx.config);
                    }
                    KeyCode::Enter => match setting.as_ref().map(|s| (s, settings::typed_value(cx.config, s, input))) {
                        Some((s, Ok(_))) => {
                            change = Some(Change::Set(s.path.clone(), input.trim().to_string()));
                            *editing = None;
                            *error = None;
                        }
                        Some((_, Err(e))) => *error = Some(format!("{e:#}")),
                        None => {}
                    },
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Char(c) => input.push(c),
                    _ => {}
                }
                // A colour being typed shows up right away, before it is saved.
                if let (Some(s), Some(input)) = (&setting, editing.as_ref())
                    && matches!(s.kind, Kind::Color)
                    && let Some(color) = crate::theme::parse_color(input)
                {
                    let mut theme = crate::theme::theme();
                    theme.set(s.path.trim_start_matches("theme.colors."), color);
                    crate::theme::set_theme(theme);
                }
            }
            Event::Key(key) => {
                *error = None;
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => close = true,
                    KeyCode::Char('j') | KeyCode::Down => select_next(state, len),
                    KeyCode::Char('k') | KeyCode::Up => select_prev(state, len),
                    KeyCode::Char('g') | KeyCode::Home => state.select(Some(0)),
                    KeyCode::Char('G') | KeyCode::End => state.select(Some(len.saturating_sub(1))),
                    KeyCode::PageDown => (0..10).for_each(|_| select_next(state, len)),
                    KeyCode::PageUp => (0..10).for_each(|_| select_prev(state, len)),
                    KeyCode::Left | KeyCode::Char('h') | KeyCode::Right | KeyCode::Char('l') => {
                        let direction = if matches!(key.code, KeyCode::Left | KeyCode::Char('h')) { -1 } else { 1 };
                        if let Some(s) = &setting
                            && let Some(next) = stepped(s, &current, direction, shift)
                        {
                            change = Some(Change::Set(s.path.clone(), next));
                        }
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        if let Some(s) = &setting {
                            match &s.kind {
                                Kind::Keys => *capture = Some(KeyCapture::default()),
                                Kind::Number { .. } | Kind::Color => *editing = Some(current.clone()),
                                _ => {
                                    if let Some(next) = stepped(s, &current, 1, false) {
                                        change = Some(Change::Set(s.path.clone(), next));
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char('r') | KeyCode::Delete | KeyCode::Backspace => {
                        if let Some(s) = &setting {
                            change = Some(Change::Reset(s.path.clone()));
                        }
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse) => {
                if !wheel_select(mouse.kind, state, len)
                    && matches!(mouse.kind, MouseEventKind::Down(_))
                    && let Some(index) = ui::settings_row_at(cx.frame_area, len, state.offset(), mouse.row)
                {
                    state.select(Some(index));
                }
            }
            _ => {}
        }
        if close {
            let back = std::mem::replace(back, Box::new(Mode::List));
            st.mode = *back;
        }
    }
    if let Some(change) = change {
        let config_before = st.config.clone();
        let result = match &change {
            Change::Set(path, text) => {
                let setting = settings::registry().into_iter().find(|s| &s.path == path);
                match setting.map(|s| settings::typed_value(&config_before, &s, text)) {
                    Some(Ok(value)) => settings::save(&Config::path(), path, Some(value)),
                    Some(Err(e)) => Err(e),
                    None => Err(anyhow::anyhow!("unknown setting {path}")),
                }
            }
            Change::Reset(path) => settings::save(&Config::path(), path, None),
        };
        match result {
            Ok(config) => st.reload(config),
            Err(e) => {
                if let Mode::Settings { error, .. } = &mut st.mode {
                    *error = Some(format!("{e:#}"));
                }
            }
        }
    }
    Ok(None)
}

/// Opens the settings screen over whatever is showing.
pub(super) fn open(st: &mut State) {
    let back = std::mem::replace(&mut st.mode, Mode::List);
    st.mode = Mode::Settings { settings: settings::registry(), state: TableState::default().with_selected(0), editing: None, capture: None, error: None, back: Box::new(back) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(path: &str) -> Setting {
        settings::registry().into_iter().chain([Setting { path: "theme.colors.ok".into(), section: "Colours", label: "ok".into(), kind: Kind::Color, restart: false }]).find(|s| s.path == path).unwrap()
    }

    #[test]
    fn choices_cycle_and_wrap_in_both_directions() {
        let border = find("logs.order");
        assert_eq!(stepped(&border, "oldest_first", 1, false).as_deref(), Some("newest_first"));
        assert_eq!(stepped(&border, "newest_first", 1, false).as_deref(), Some("oldest_first"));
        assert_eq!(stepped(&border, "oldest_first", -1, false).as_deref(), Some("newest_first"));
    }

    #[test]
    fn numbers_step_by_one_or_ten_and_stay_in_range() {
        let wheel = find("mouse.wheel_rows");
        assert_eq!(stepped(&wheel, "3", 1, false).as_deref(), Some("4"));
        assert_eq!(stepped(&wheel, "3", 1, true).as_deref(), Some("13"));
        assert_eq!(stepped(&wheel, "18", 1, true).as_deref(), Some("20"), "clamped to the maximum");
        assert_eq!(stepped(&wheel, "1", -1, false).as_deref(), Some("1"), "and the minimum");
    }

    #[test]
    fn booleans_toggle_and_colours_are_typed_not_stepped() {
        assert_eq!(stepped(&find("portforward.open_browser"), "true", 1, false).as_deref(), Some("false"));
        assert_eq!(stepped(&find("portforward.open_browser"), "false", -1, false).as_deref(), Some("true"));
        assert_eq!(stepped(&find("theme.colors.ok"), "#00ff00", 1, false), None);
    }
}
