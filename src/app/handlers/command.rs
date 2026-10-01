//! The `:` command line and the `/` search bar.

use super::super::*;
use super::Cx;

/// Handles one input event for these modes; `Some` ends the session.
pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let catalog = &mut *cx.catalog;
    let active_context = cx.active_context;
    let names = command_names(st);
    match (event, &mut st.mode) {
        (Event::Key(key), Mode::Command { input, selected, back }) => match key.code {
            KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            // Tab completes the highlighted suggestion, or moves on to the next when it is
            // already typed out.
            KeyCode::Tab => {
                let suggestions = command_suggestions(input, &catalog.crds, &catalog.apis, &catalog.dashboard_categories(), &names);
                if let Some(chosen) = suggestions.get((*selected).min(suggestions.len().saturating_sub(1))) {
                    let name = chosen.primary_name();
                    if *input == name {
                        *selected = (*selected + 1) % suggestions.len();
                    } else {
                        *input = name;
                        *selected = 0;
                    }
                }
            }
            KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::Down => {
                let len = command_suggestions(input, &catalog.crds, &catalog.apis, &catalog.dashboard_categories(), &names).len();
                *selected = (*selected + 1).min(len.saturating_sub(1));
            }
            KeyCode::Enter => {
                let cmd = input.trim().to_lowercase();
                // The highlighted suggestion wins; what was typed is the fallback for an exact
                // alias that isn't in the visible list.
                let suggestions = command_suggestions(input, &catalog.crds, &catalog.apis, &catalog.dashboard_categories(), &names);
                let typed = || match cmd.as_str() {
                    "q" | "quit" | "exit" => Some(Cmd::Quit),
                    "config" | "settings" | "preferences" | "prefs" | "options" => Some(Cmd::Settings),
                    "theme" | "themes" | "skin" | "skins" => Some(Cmd::Theme),
                    c if is_context_command(c) => Some(Cmd::Context),
                    c => k8s::ResourceKind::from_command(c).map(Cmd::Kind),
                };
                match suggestions.get(*selected).map(|s| s.cmd).or_else(typed) {
                    Some(Cmd::Quit) => return Ok(Some(SessionEnd::Quit)),
                    Some(Cmd::Settings) => {
                        let mut opened = std::mem::replace(&mut **back, Mode::List);
                        std::mem::swap(&mut st.mode, &mut opened);
                        super::settings::open(st);
                    }
                    Some(Cmd::Theme) => {
                        let mut opened = std::mem::replace(&mut **back, Mode::List);
                        std::mem::swap(&mut st.mode, &mut opened);
                        super::themes::open(st, cx.config);
                    }
                    Some(Cmd::Custom(index)) => {
                        st.mode = std::mem::replace(&mut **back, Mode::List);
                        // It runs on the selected row, so only from a list.
                        if matches!(st.mode, Mode::List)
                            && let Some(target) = crate::app::handlers::list::selected_manifest(st, cx.d, catalog, cx.client).as_ref().and_then(crate::ops::actions::Target::from_manifest)
                        {
                            super::custom::run(st, cx, index, &target);
                        }
                    }
                    Some(Cmd::Problems) => {
                        let opened = std::mem::replace(&mut **back, Mode::List);
                        st.mode = opened;
                        super::problems::open(st);
                    }
                    Some(Cmd::Events) => {
                        st.mode = Mode::Events { filter: k8s::EventFilter::All, search: String::new(), editing: false, state: TableState::default().with_selected(0), sort: ListSort::default() };
                    }
                    Some(Cmd::Context) => {
                        let mut opened = std::mem::replace(&mut **back, Mode::List);
                        open_context_switcher(&mut opened, active_context);
                        st.mode = opened;
                    }
                    Some(Cmd::Kind(kind)) => {
                        st.switch_kind(kind);
                        st.mode = Mode::List;
                    }
                    Some(Cmd::Api(index, plural, _)) => {
                        st.switch_kind(ResourceKind::Api(index, plural));
                        st.mode = Mode::List;
                    }
                    None => st.mode = std::mem::replace(&mut **back, Mode::List),
                }
            }
            KeyCode::Backspace => {
                input.pop();
                *selected = 0;
            }
            KeyCode::Char(c) => {
                input.push(c);
                *selected = 0;
            }
            _ => {}
        },
        (Event::Key(key), Mode::Search) => match key.code {
            KeyCode::Esc => {
                st.search.clear();
                st.table_state.select(Some(0));
                st.mode = Mode::List;
            }
            KeyCode::Enter => st.mode = Mode::List,
            KeyCode::Backspace => {
                st.search.pop();
                st.table_state.select(Some(0));
            }
            KeyCode::Char(c) => {
                st.search.push(c);
                st.table_state.select(Some(0));
            }
            _ => {}
        },
        _ => {}
    }
    Ok(None)
}

/// The names of your own commands, for the suggestions.
fn command_names(st: &State) -> Vec<String> {
    st.config.commands.iter().map(|c| c.name.clone()).collect()
}
