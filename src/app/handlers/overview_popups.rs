//! Overview popups: the events list and its detail, resources, category columns.

use super::super::*;
use super::Cx;
use crate::app::derive::Derived;

/// Handles one input event for these modes; `Some` ends the session.
pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let Derived { overview, .. } = cx.d;
    let catalog = &mut *cx.catalog;
    let frame_area = cx.frame_area;
    match (event, &mut st.mode) {
        (Event::Key(key), Mode::Events { search, editing: editing @ true, state, .. }) => {
            if super::edit_line(key.code, search, editing) {
                state.select(Some(0));
            }
        }
        (Event::Key(key), Mode::Events { filter, search, editing, state, sort }) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => st.mode = Mode::List,
            KeyCode::Char('/') | KeyCode::Char('f') => *editing = true,
            KeyCode::Char('a') => *filter = k8s::EventFilter::All,
            KeyCode::Char('w') => *filter = k8s::EventFilter::Warnings,
            KeyCode::Char('n') => *filter = k8s::EventFilter::Normal,
            KeyCode::Char('j') | KeyCode::Down => select_next(state, k8s::filter_events(&overview.events, *filter, search, sort.spec).len()),
            KeyCode::Char('k') | KeyCode::Up => select_prev(state, k8s::filter_events(&overview.events, *filter, search, sort.spec).len()),
            KeyCode::Enter => {
                let entry = state.selected().and_then(|i| k8s::filter_events(&overview.events, *filter, search, sort.spec).get(i).map(|e| (*e).clone()));
                if let Some(entry) = entry {
                    let back = Box::new(Mode::Events { filter: *filter, search: search.clone(), editing: false, state: *state, sort: *sort });
                    st.mode = Mode::EventDetail { entry, back };
                }
            }
            _ => {}
        },
        (Event::Mouse(mouse), Mode::Events { filter, search, state, sort, .. }) if matches!(mouse.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) => {
            wheel_select(mouse.kind, state, k8s::filter_events(&overview.events, *filter, search, sort.spec).len());
        }
        (Event::Mouse(mouse), Mode::Events { filter, search, state, sort, .. }) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
            let filtered = k8s::filter_events(&overview.events, *filter, search, sort.spec);
            if let Some(idx) = ui::event_row_at(frame_area, filtered.len(), state.offset(), mouse.row) {
                state.select(Some(idx));
                let entry = filtered[idx].clone();
                let back = Box::new(Mode::Events { filter: *filter, search: search.clone(), editing: false, state: *state, sort: *sort });
                st.mode = Mode::EventDetail { entry, back };
            }
        }
        (Event::Key(key), Mode::EventDetail { back, .. }) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
            _ => {}
        },
        (Event::Key(key), Mode::ResourcesDetail) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => st.mode = Mode::List,
            _ => {}
        },
        (Event::Key(key), Mode::ColumnDetail { col, selected, row_scroll }) => {
            let items_len = overview.catalog.get(*col).map(|(_, items)| items.len()).unwrap_or(0);
            let cols = ui::column_detail_cols(frame_area, overview.catalog.get(*col).map(|(_, items)| items.as_slice()).unwrap_or(&[]));
            // The scroll recompute happens inside each navigation branch: the Enter/Esc
            // branches reassign `mode`, which would leave `selected`/`row_scroll` dangling.
            macro_rules! move_and_rescroll {
                ($dir:expr) => {{
                    *selected = ui::move_column_detail_selection(items_len, cols, *selected, $dir);
                    let visible_rows = ui::column_detail_visible_rows(frame_area, overview.catalog.get(*col).map(|(_, items)| items.as_slice()).unwrap_or(&[]));
                    let selected_row = if cols > 0 { *selected / cols } else { 0 };
                    *row_scroll = ui::scroll_columns_to_show(*row_scroll, visible_rows, selected_row);
                }};
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => st.mode = Mode::List,
                KeyCode::Char('j') | KeyCode::Down => move_and_rescroll!(ui::Direction::Down),
                KeyCode::Char('k') | KeyCode::Up => move_and_rescroll!(ui::Direction::Up),
                KeyCode::Char('h') | KeyCode::Left => move_and_rescroll!(ui::Direction::Left),
                KeyCode::Char('l') | KeyCode::Right => move_and_rescroll!(ui::Direction::Right),
                KeyCode::Enter => {
                    if let Some((_, items)) = overview.catalog.get(*col)
                        && let Some((label, _)) = items.get(*selected)
                        && let Some(kind) = catalog.kind_for_tile_label(label)
                    {
                        st.switch_kind(kind);
                        st.mode = Mode::List;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(None)
}
