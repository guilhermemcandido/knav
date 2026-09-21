//! Clicks and the wheel on the main list and the Overview.

use super::*;

/// Handles a mouse event on the list; the flags say a double click asked to open the row
/// or to follow a pod's owner.
pub(super) fn handle(mouse: crossterm::event::MouseEvent, st: &mut State, cx: &mut Cx) -> (bool, bool) {
    let Derived { pod_rows, overview, .. } = cx.d;
    let (frame_area, row_count) = (cx.frame_area, cx.row_count);
    let (mut open, mut to_owner) = (false, false);
        if st.current_kind == ResourceKind::Overview {
            let active_col = match st.overview_selection {
                ui::OverviewSelection::Header(c) | ui::OverviewSelection::Item(c, _) => c,
                ui::OverviewSelection::Resources | ui::OverviewSelection::Events => usize::MAX,
            };
            match mouse.kind {
                MouseEventKind::Down(_) => {
                    if let Some(hit) = ui::column_hit(ui::beside_sidebar(frame_area, false), overview, st.overview_col_scroll, active_col, st.overview_item_scroll, mouse.column, mouse.row) {
                        st.overview_selection = hit;
                        // A second click on the same tile soon after opens it.
                        let id = match hit {
                            ui::OverviewSelection::Resources => 0,
                            ui::OverviewSelection::Events => 1,
                            ui::OverviewSelection::Header(c) => 10 + c * 100,
                            ui::OverviewSelection::Item(c, i) => 11 + c * 100 + i,
                        };
                        let now = std::time::Instant::now();
                        let again = st.last_click.is_some_and(|(at, prev)| prev == id && now.duration_since(at) < std::time::Duration::from_millis(crate::config::tunables::tunables().double_click_ms));
                        st.last_click = if again { None } else { Some((now, id)) };
                        open = again;
                    }
                }
                MouseEventKind::ScrollDown => st.overview_selection = ui::move_overview_selection(overview, st.overview_selection, ui::Direction::Down),
                MouseEventKind::ScrollUp => st.overview_selection = ui::move_overview_selection(overview, st.overview_selection, ui::Direction::Up),
                _ => {}
            }
            keep_overview_selection_visible(st, overview, frame_area);
        } else {
            let table = ui::list_body(frame_area);
            // Over the info panel: the wheel scrolls it, a click gives it the keys, and
            // nothing reaches the list underneath. A click on the list takes the keys back.
            if st.info_panel {
                let over_panel = mouse.column >= table.x + table.width;
                match mouse.kind {
                    MouseEventKind::ScrollDown if over_panel => {
                        st.info_scroll += 3;
                        return (false, false);
                    }
                    MouseEventKind::ScrollUp if over_panel => {
                        st.info_scroll = st.info_scroll.saturating_sub(3);
                        return (false, false);
                    }
                    MouseEventKind::Down(_) if over_panel => {
                        st.info_focus = true;
                        return (false, false);
                    }
                    _ if over_panel => return (false, false),
                    MouseEventKind::Down(_) => st.info_focus = false,
                    _ => {}
                }
            }
            match mouse.kind {
                MouseEventKind::Moved => {
                    st.hovered = ui::row_at(table, pod_rows, st.wide, st.hscroll, &st.table_state, row_count, mouse.column, mouse.row)
                        .map(|row| ui::Hover { row, column: mouse.column, row_on_screen: mouse.row });
                }
                // A click selects the row; a second click on it soon after opens it.
                MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                    st.hovered = ui::row_at(table, pod_rows, st.wide, st.hscroll, &st.table_state, row_count, mouse.column, mouse.row)
                        .map(|row| ui::Hover { row, column: mouse.column, row_on_screen: mouse.row });
                    if let Some(index) = ui::list_row_at(table, st.table_state.offset(), row_count, mouse.row) {
                        st.table_state.select(Some(index));
                        // Clicking a pod's CONTROLLER follows it to the owner.
                        let on_controller = st.current_kind == ResourceKind::Pods && ui::controller_at(table, pod_rows, st.wide, st.hscroll, mouse.column) && pod_rows.get(index).is_some_and(|p| p.controlled_by != "-");
                        if on_controller {
                            st.last_click = None;
                            to_owner = true;
                        } else {
                            let now = std::time::Instant::now();
                            let again = st.last_click.is_some_and(|(at, row)| row == index && now.duration_since(at) < std::time::Duration::from_millis(crate::config::tunables::tunables().double_click_ms));
                            st.last_click = if again { None } else { Some((now, index)) };
                            open = again;
                        }
                    }
                }
                kind => {
                    wheel_select(kind, &mut st.table_state, row_count);
                }
            }
        }
    (open, to_owner)
}
