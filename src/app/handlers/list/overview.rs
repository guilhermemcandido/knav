//! Keys on the Overview: moving between the tiles and opening one.

use super::*;

pub(super) fn keys(key: crossterm::event::KeyEvent, st: &mut State, cx: &mut Cx) {
    let overview = &cx.d.overview;
    let (catalog, frame_area, client) = (&mut *cx.catalog, cx.frame_area, cx.client);
        match key.code {
            KeyCode::Char('n') => {
                let names: Vec<String> =
                    catalog.resolve(ResourceKind::Namespaces, client).map(|k| k.rows()).unwrap_or_default().into_iter().map(|r| r.name.clone()).collect();
                open_namespace_picker(&mut st.mode, names);
            }
            // Esc and `q` do nothing here: nothing is further back, and quitting takes `:q`.
            KeyCode::Char('j') | KeyCode::Down => {
                st.overview_selection = ui::move_overview_selection(overview, st.overview_selection, ui::Direction::Down);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                st.overview_selection = ui::move_overview_selection(overview, st.overview_selection, ui::Direction::Up);
            }
            KeyCode::Char('h') | KeyCode::Left => {
                st.overview_selection = ui::move_overview_selection(overview, st.overview_selection, ui::Direction::Left);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                st.overview_selection = ui::move_overview_selection(overview, st.overview_selection, ui::Direction::Right);
            }
            // `f` switches Home between the pinned kinds and every category; `*` pins the tile.
            KeyCode::Char('f') => {
                let all = crate::config::pinned::home(&st.config.home, st.home_all).is_some();
                st.home_all = Some(all);
                ui::show_toast(if all { "Home: all kinds" } else { "Home: favorites" }.into(), true);
                // The cards are different ones now.
                st.overview_selection = ui::OverviewSelection::Resources;
            }
            KeyCode::Char('*') => {
                if let ui::OverviewSelection::Item(col, item) = st.overview_selection
                    && let Some((label, _)) = overview.catalog.get(col).and_then(|(_, items)| items.get(item))
                {
                    pin_toast(st, label);
                    if overview.favorites {
                        // That card is gone, so the selection would point past the end.
                        st.overview_selection = ui::OverviewSelection::Resources;
                    }
                }
            }
            // `[` and `]` move the selected favorite earlier or later.
            KeyCode::Char(c @ ('[' | ']')) if overview.favorites => {
                if let ui::OverviewSelection::Item(col, item) = st.overview_selection
                    && let Some((label, _)) = overview.catalog.get(col).and_then(|(_, items)| items.get(item))
                    && *label != k8s::ADD_FAVORITE
                {
                    let back = c == '[';
                    let at = col * 3 + item;
                    if let Some(moved) = crate::config::pinned::nudged(&st.config.home.favorites, label, back) {
                        save_favorites(st, moved);
                        let to = if back { at - 1 } else { at + 1 };
                        st.overview_selection = ui::OverviewSelection::Item(to / 3, to % 3);
                    }
                }
            }
            KeyCode::Char('T') => crate::app::handlers::themes::open(st, cx.config),
            KeyCode::Char(',') => crate::app::handlers::settings::open(st),
            KeyCode::Enter => match st.overview_selection {
                ui::OverviewSelection::Resources => {
                    st.mode = Mode::ResourcesDetail;
                }
                ui::OverviewSelection::Events(_) => {
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
                        && *label == k8s::ADD_FAVORITE
                    {
                        // Pinning happens on the full Home, where `*` marks a kind.
                        st.home_all = Some(true);
                        st.overview_selection = ui::OverviewSelection::Resources;
                        ui::show_toast("Press * on a kind to pin it, f to come back".into(), true);
                    } else if let Some((_, items)) = overview.catalog.get(col)
                        && let Some((label, _)) = items.get(item)
                    {
                        if overview.absent.iter().any(|a| a == label) {
                            ui::show_toast(format!("{label} is not in this cluster"), false);
                        } else if let Some(kind) = catalog.kind_for_tile_label(label) {
                            st.switch_kind(kind);
                        }
                    }
                }
            },
            _ => {}
        }
        keep_overview_selection_visible(st, overview, frame_area);
}

/// Pins or unpins a kind's tile, and says which.
pub(super) fn pin_toast(st: &mut State, label: &str) {
    let (labels, pinned) = crate::config::pinned::toggled(&st.config.home.favorites, label);
    if save_favorites(st, labels) {
        ui::show_toast(if pinned { format!("Pinned {label} to Home") } else { format!("Unpinned {label}") }, true);
    }
}

/// Writes the favorites to `config.toml`, which keeps them for the next start.
pub(super) fn save_favorites(st: &mut State, labels: Vec<String>) -> bool {
    let mut array = toml_edit::Array::new();
    for label in &labels {
        array.push(label.as_str());
    }
    match crate::config::edit::save(&crate::config::Config::path(), "home.favorites", Some(toml_edit::Value::Array(array))) {
        Ok(saved) => {
            st.config.home = saved.home;
            true
        }
        Err(e) => {
            ui::show_toast(format!("Could not save the favorites: {e}"), false);
            false
        }
    }
}
