//! Keys on the Overview: moving between the tiles and opening one.

use super::*;

pub(super) fn keys(key: crossterm::event::KeyEvent, st: &mut State, cx: &mut Cx) {
    let overview = &cx.d.overview;
    let (catalog, frame_area, client) = (&mut *cx.catalog, cx.frame_area, cx.client);
        match key.code {
            // While a favorite is picked up, the arrows carry it and enter, space or esc put it down.
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Esc if st.drag_favorite.is_some() => st.drag_favorite = None,
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Char('h' | 'j' | 'k' | 'l') if st.drag_favorite.is_some() => {
                if let Some(label) = st.drag_favorite.clone()
                    && let Some(at) = st.config.home.favorites.iter().position(|l| *l == label)
                {
                    // Columns hold three, so sideways is three places and vertical one.
                    let to = match key.code {
                        KeyCode::Left | KeyCode::Char('h') => at.saturating_sub(3),
                        KeyCode::Right | KeyCode::Char('l') => at + 3,
                        KeyCode::Up | KeyCode::Char('k') => at.saturating_sub(1),
                        _ => at + 1,
                    };
                    if let Some(moved) = crate::config::pinned::moved(&st.config.home.favorites, &label, to) {
                        save_favorites(st, moved);
                        let now = st.config.home.favorites.iter().position(|l| *l == label).unwrap_or(at);
                        st.overview_selection = ui::OverviewSelection::Item(now / 3, now % 3);
                    }
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Esc if st.home_lift.is_some() => st.home_lift = None,
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Char('h' | 'j' | 'k' | 'l') if st.home_lift.is_some() => {
                // A category moves sideways and a kind up or down, by one place among those shown.
                let sideways = st.home_lift.as_ref().is_some_and(|(_, kind)| kind.is_none());
                let (back, forward) = if sideways { (matches!(key.code, KeyCode::Left | KeyCode::Char('h')), matches!(key.code, KeyCode::Right | KeyCode::Char('l'))) } else { (matches!(key.code, KeyCode::Up | KeyCode::Char('k')), matches!(key.code, KeyCode::Down | KeyCode::Char('j'))) };
                if let ui::OverviewSelection::Header(at) | ui::OverviewSelection::Item(_, at) = st.overview_selection {
                    let at = if sideways { match st.overview_selection { ui::OverviewSelection::Header(c) => c, _ => at } } else { at };
                    if back && at > 0 {
                        move_lift(st, catalog, overview, at - 1);
                    } else if forward {
                        move_lift(st, catalog, overview, at + 1);
                    }
                }
            }
            // Space picks up the selected category or kind to move it.
            KeyCode::Char(' ') if !overview.favorites => {
                st.home_lift = match st.overview_selection {
                    ui::OverviewSelection::Header(col) => overview.catalog.get(col).map(|(title, _)| ((*title).to_string(), None)),
                    ui::OverviewSelection::Item(col, item) => overview.catalog.get(col).and_then(|(title, items)| items.get(item).map(|(label, _)| ((*title).to_string(), Some((*label).to_string())))),
                    _ => None,
                };
            }
            // Space picks the selected favorite up to move it.
            KeyCode::Char(' ') if overview.favorites => {
                if let ui::OverviewSelection::Item(col, item) = st.overview_selection
                    && let Some((label, _)) = overview.catalog.get(col).and_then(|(_, items)| items.get(item))
                    && *label != k8s::ADD_FAVORITE
                {
                    st.drag_favorite = Some((*label).to_string());
                }
            }
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
            // The Favorites box is one header, so sideways has nowhere to go from it.
            KeyCode::Char('h' | 'l') | KeyCode::Left | KeyCode::Right if overview.favorites && matches!(st.overview_selection, ui::OverviewSelection::Header(_)) => {}
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
                            ui::refuse(format!("{label} is not in this cluster"));
                        } else if let Some(kind) = catalog.kind_for_tile_label(label) {
                            st.switch_kind(kind);
                        } else {
                            ui::refuse(format!("{label} can't be opened"));
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

/// Writes a list of names to `config.toml` at `path` and takes the result in.
fn save_names(st: &mut State, path: &str, names: &[String]) -> bool {
    let mut array = toml_edit::Array::new();
    for name in names {
        array.push(name.as_str());
    }
    match crate::config::edit::save(&crate::config::Config::path(), path, Some(toml_edit::Value::Array(array))) {
        Ok(saved) => {
            st.config.home = saved.home;
            st.config.overview = saved.overview;
            true
        }
        Err(e) => {
            ui::refuse(format!("Could not save: {e}"));
            false
        }
    }
}

/// Writes the favorites to `config.toml`, which keeps them for the next start.
pub(super) fn save_favorites(st: &mut State, labels: Vec<String>) -> bool {
    save_names(st, "home.favorites", &labels)
}

/// Moves the picked-up category, or kind within its category, to place `to` among those
/// shown, and saves the order in `overview.sections` or `overview.items.<category>`.
pub(super) fn move_lift(st: &mut State, catalog: &Catalog, overview: &k8s::Overview, to: usize) {
    use crate::app::overview_layout as layout;
    let Some((category, kind)) = st.home_lift.clone() else { return };
    let mut sections = layout::resolve(&st.config.overview, &catalog.layout_names(&st.config.extensions.enabled));
    match kind {
        None => {
            let shown: Vec<&str> = overview.catalog.iter().map(|(title, _)| *title).collect();
            let to = to.min(shown.len().saturating_sub(1));
            let (Some(from), Some(dest)) = (sections.iter().position(|s| s.name == category), shown.get(to).and_then(|t| sections.iter().position(|s| s.name == *t))) else { return };
            if from == dest {
                return;
            }
            layout::move_section_to(&mut sections, from, dest);
            let names: Vec<String> = sections.iter().map(|s| s.name.clone()).collect();
            if save_names(st, "overview.sections", &names) {
                st.overview_selection = ui::OverviewSelection::Header(to);
            }
        }
        Some(kind) => {
            let Some(col) = overview.catalog.iter().position(|(title, _)| *title == category) else { return };
            let shown: Vec<&str> = overview.catalog[col].1.iter().map(|(label, _)| *label).collect();
            let to = to.min(shown.len().saturating_sub(1));
            let Some(section) = sections.iter_mut().find(|s| s.name == category) else { return };
            let (Some(from), Some(dest)) = (section.items.iter().position(|i| i.name == kind), shown.get(to).and_then(|t| section.items.iter().position(|i| i.name == *t))) else { return };
            if from == dest {
                return;
            }
            let item = section.items.remove(from);
            section.items.insert(dest, item);
            let names: Vec<String> = section.items.iter().map(|i| i.name.clone()).collect();
            if save_names(st, &format!("overview.items.{category}"), &names) {
                st.overview_selection = ui::OverviewSelection::Item(col, to);
            }
        }
    }
}
