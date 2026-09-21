//! The theme picker: moving through the list previews each theme on the
//! whole interface; Enter keeps it (saving it to the config), Esc restores
//! what was there.

use super::super::*;
use super::Cx;

/// Shows the theme at the cursor, with the config's own colour overrides on top.
fn preview(entries: &[ThemeEntry], state: &TableState, config: &Config) {
    if let Some(entry) = state.selected().and_then(|i| entries.get(i)) {
        let (theme, _) = crate::theme::build(&entry.name, &config.theme.colors);
        crate::theme::set_theme(theme);
    }
}

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let mut saved: Option<Config> = None;
    let mut notice: Option<String> = None;
    let mut close = false;
    if let Mode::ThemePicker { entries, state, back, .. } = &mut st.mode {
        let len = entries.len();
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => select_next(state, len),
                KeyCode::Char('k') | KeyCode::Up => select_prev(state, len),
                KeyCode::Char('g') | KeyCode::Home => state.select(Some(0)),
                KeyCode::Char('G') | KeyCode::End => state.select(Some(len.saturating_sub(1))),
                KeyCode::PageDown => (0..10).for_each(|_| select_next(state, len)),
                KeyCode::PageUp => (0..10).for_each(|_| select_prev(state, len)),
                KeyCode::Enter => {
                    if let Some(entry) = state.selected().and_then(|i| entries.get(i)) {
                        match crate::settings::save(&Config::path(), "theme.preset", Some(toml_edit::Value::from(entry.name.as_str()))) {
                            Ok(config) => saved = Some(config),
                            Err(e) => notice = Some(format!("{e:#}")),
                        }
                    }
                    close = true;
                }
                KeyCode::Char('q') | KeyCode::Esc => close = true,
                _ => {}
            },
            Event::Mouse(mouse) => {
                if !wheel_select(mouse.kind, state, len)
                    && matches!(mouse.kind, MouseEventKind::Down(_))
                    && let Some(index) = ui::theme_row_at(cx.frame_area, len, state.offset(), mouse.row)
                {
                    state.select(Some(index));
                }
            }
            _ => {}
        }
        if !close {
            preview(entries, state, cx.config);
        } else {
            let back = std::mem::replace(back, Box::new(Mode::List));
            st.mode = match notice.take() {
                Some(text) => Mode::Notice { text, error: true, back },
                None => *back,
            };
        }
    }
    if close {
        // Kept: the saved config is the new one. Cancelled or failed: back to the old.
        if let Some(config) = saved {
            st.config = config;
        }
        crate::settings::apply(&st.config);
    }
    Ok(None)
}

/// Opens the picker on the theme in use.
pub(super) fn open(st: &mut State, config: &Config) {
    let (entries, at) = theme_entries(&config.theme.preset);
    let back = std::mem::replace(&mut st.mode, Mode::List);
    st.mode = Mode::ThemePicker { entries, state: TableState::default().with_selected(at), back: Box::new(back) };
}
