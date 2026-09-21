//! Keys on the Overview: moving between the tiles and opening one.

use super::*;

/// Handles one key on the Overview.
pub(super) fn keys(key: crossterm::event::KeyEvent, st: &mut State, cx: &mut Cx) {
    let overview = &cx.d.overview;
    let (catalog, frame_area, client) = (&mut *cx.catalog, cx.frame_area, cx.client);
        match key.code {
            KeyCode::Char('n') => {
                let names: Vec<String> =
                    catalog.resolve(ResourceKind::Namespaces, &client).map(|k| k.rows()).unwrap_or_default().into_iter().map(|r| r.name.clone()).collect();
                open_namespace_picker(&mut st.mode, names);
            }
            // Esc and `q` are no-ops here, there's nowhere
            // further "back" than the main screen, and quitting
            // takes a deliberate `:q` so a stray key can't do it.
            KeyCode::Char('j') | KeyCode::Down => {
                st.overview_selection = ui::move_overview_selection(&overview, st.overview_selection, ui::Direction::Down);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                st.overview_selection = ui::move_overview_selection(&overview, st.overview_selection, ui::Direction::Up);
            }
            KeyCode::Char('h') | KeyCode::Left => {
                st.overview_selection = ui::move_overview_selection(&overview, st.overview_selection, ui::Direction::Left);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                st.overview_selection = ui::move_overview_selection(&overview, st.overview_selection, ui::Direction::Right);
            }
            KeyCode::Char('T') => crate::app::handlers::themes::open(st, cx.config),
            KeyCode::Char(',') => crate::app::handlers::settings::open(st),
            KeyCode::Enter => match st.overview_selection {
                ui::OverviewSelection::Resources => {
                    st.mode = Mode::ResourcesDetail;
                }
                ui::OverviewSelection::Events => {
                    st.mode = Mode::Events {
                        filter: k8s::EventFilter::default(),
                        search: String::new(),
                        editing: false,
                        sort: ListSort::default(),
                        state: TableState::default().with_selected(if overview.events.is_empty() { None } else { Some(0) }),
                    };
                }
                ui::OverviewSelection::Header(col) => {
                    st.mode = Mode::ColumnDetail { col, selected: 0, row_scroll: 0 };
                }
                ui::OverviewSelection::Item(col, item) => {
                    if let Some((_, items)) = overview.catalog.get(col)
                        && let Some((label, _)) = items.get(item)
                        && let Some(kind) = catalog.kind_for_tile_label(label)
                    {
                        st.switch_kind(kind);
                    }
                }
            },
            _ => {}
        }
        keep_overview_selection_visible(st, overview, frame_area);
}
