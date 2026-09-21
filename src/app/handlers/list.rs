//! Input on the main list (and the overview): sorting, namespaces, drill-down, opening details.

use super::super::*;
use super::Cx;
use crate::app::derive::Derived;

/// Handles one input event for these modes; `Some` ends the session.
pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let Derived { pods, pod_rows, deployments, sorted_nodes, overview, generic_rows_full, generic_visible, generic_columns, generic_rows, crd_rows, .. } = cx.d;
    let catalog = &mut *cx.catalog;
    let client = cx.client;
    let frame_area = cx.frame_area;
    let row_count = cx.row_count;
    let mut open = false;
    match (event, &mut st.mode) {
        // Sort mode (`s`): the headers show their column numbers and a
        // digit sorts by that column — the same one again flips
        // ascending, descending, off. It stays on until `s`, Esc or
        // `q`; every other key still works as usual meanwhile.
        (Event::Key(key), Mode::List)
            if st.sort_choosing && matches!(key.code, KeyCode::Char('0'..='9' | 's' | 'q') | KeyCode::Esc) =>
        {
            match key.code {
                KeyCode::Char(c @ '0'..='9') => {
                    // 1-9 are columns 1-9; 0 is the tenth.
                    let column = (c as usize + 9 - '0' as usize) % 10;
                    if column < column_count(st.current_kind, *generic_columns) {
                        st.sort = Some(SortSpec::pressed(st.sort, column));
                        st.table_state.select(Some(0));
                    }
                }
                _ => st.sort_choosing = false,
            }
        }
        (Event::Mouse(mouse), Mode::List) => {
            if st.current_kind == ResourceKind::Overview {
                let active_col = match st.overview_selection {
                    ui::OverviewSelection::Header(c) | ui::OverviewSelection::Item(c, _) => c,
                    ui::OverviewSelection::Resources | ui::OverviewSelection::Events => usize::MAX,
                };
                match mouse.kind {
                    MouseEventKind::Down(_) => {
                        if let Some(hit) = ui::column_hit(ui::body_area(frame_area, false), overview, st.overview_col_scroll, active_col, st.overview_item_scroll, mouse.column, mouse.row) {
                            st.overview_selection = hit;
                        }
                    }
                    MouseEventKind::ScrollDown => st.overview_selection = ui::move_overview_selection(overview, st.overview_selection, ui::Direction::Down),
                    MouseEventKind::ScrollUp => st.overview_selection = ui::move_overview_selection(overview, st.overview_selection, ui::Direction::Up),
                    _ => {}
                }
                keep_overview_selection_visible(st, overview, frame_area);
            } else {
                let table = ui::body_area(frame_area, true);
                match mouse.kind {
                    MouseEventKind::Moved => {
                        st.hovered = ui::row_at(table, pod_rows, st.hscroll, &st.table_state, row_count, mouse.column, mouse.row)
                            .map(|row| ui::Hover { row, column: mouse.column, row_on_screen: mouse.row });
                    }
                    // A click selects the row; a second click on it soon after opens it.
                    MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                        st.hovered = ui::row_at(table, pod_rows, st.hscroll, &st.table_state, row_count, mouse.column, mouse.row)
                            .map(|row| ui::Hover { row, column: mouse.column, row_on_screen: mouse.row });
                        if let Some(index) = ui::list_row_at(table, st.table_state.offset(), row_count, mouse.row) {
                            st.table_state.select(Some(index));
                            let now = std::time::Instant::now();
                            let again = st.last_click.is_some_and(|(at, row)| row == index && now.duration_since(at) < DOUBLE_CLICK);
                            st.last_click = if again { None } else { Some((now, index)) };
                            open = again;
                        }
                    }
                    kind => {
                        wheel_select(kind, &mut st.table_state, row_count);
                    }
                }
            }
        }
        // Number keys pick the active namespace: 0 is all, 1-9 are the
        // ones reserved with `s`. Reachable from any list.
        (Event::Key(key), Mode::List) if matches!(key.code, KeyCode::Char('0'..='9')) => {
            if let KeyCode::Char(c) = key.code {
                let n = c.to_digit(10).unwrap_or(0) as usize;
                if n == 0 {
                    st.namespace = None;
                    st.table_state.select(Some(0));
                } else if let Some(ns) = st.favorites.get(n) {
                    st.namespace = Some(ns.to_string());
                    st.table_state.select(Some(0));
                }
            }
        }
        (Event::Key(key), Mode::List) if st.current_kind == ResourceKind::Overview => {
            match key.code {
                KeyCode::Char('n') => {
                    let names: Vec<String> =
                        catalog.resolve(ResourceKind::Namespaces, &client).map(|k| k.rows()).unwrap_or_default().into_iter().map(|r| r.name).collect();
                    open_namespace_picker(&mut st.mode, names);
                }
                // Esc and `q` are no-ops here — there's nowhere
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
                KeyCode::Char('m') => {
                    st.mode = Mode::Menu { selected: menu_position_for(st.current_kind, &catalog.crds) };
                }
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
        (Event::Key(key), Mode::List) => match if key.code == KeyCode::Enter && st.current_kind.opens_spec_on_enter() {
            KeyCode::Char('d')
        } else {
            key.code
        } {
            // `q` and Esc do the same thing everywhere except the
            // main Overview screen: back out one level — to
            // Overview from any top-level kind, or to the specific
            // CRD-group picker (or the flat list, if discovery
            // somehow can't find it) one specific CRD kind's
            // instances came from, mirroring how you got there.
            // Quitting from in here is still reachable via `:q`.
            KeyCode::Char('q') | KeyCode::Esc if !st.nav_stack.is_empty() => {
                // Back out of a drill-down to the list it came from.
                if let Some((kind, previous_scope, selected)) = st.nav_stack.pop() {
                    st.current_kind = kind;
                    st.scope = previous_scope;
                    st.sort = None;
                    st.hscroll = 0;
                    st.table_state.select(Some(selected));
                }
                st.search.clear();
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                st.current_kind = match st.current_kind {
                    ResourceKind::CustomResource(index, _) => catalog
                        .crds
                        .get(index)
                        .map(|c| ResourceKind::CustomResourceGroup(c.group))
                        .unwrap_or(ResourceKind::CustomResourceList),
                    _ => ResourceKind::Overview,
                };
                st.table_state.select(Some(0));
                st.search.clear();
            }
            // Enter drills into what a row owns or selects: a
            // Deployment's ReplicaSets, a ReplicaSet's Pods, a
            // Service's Pods, a CronJob's Jobs, a Namespace's Pods.
            KeyCode::Enter if st.current_kind.drill_target().is_some() => {
                let target = st.current_kind.drill_target().expect("guarded above");
                let selected = st.table_state.selected().unwrap_or(0);
                let new_scope = match st.current_kind {
                    ResourceKind::Deployments => deployments.get(selected).map(|d| Scope::Owner {
                        uid: d.metadata.uid.clone().unwrap_or_default(),
                        kind: "Deployment".into(),
                        name: d.metadata.name.clone().unwrap_or_default(),
                    }),
                    ResourceKind::Namespaces => generic_rows.get(selected).map(|r| Scope::Namespace { name: r.name.clone() }),
                    ResourceKind::Services => {
                        let manifest = generic_visible
                            .get(selected)
                            .and_then(|&real| catalog.resolve(st.current_kind, &client).and_then(|k| k.spec_at(real)));
                        generic_rows.get(selected).zip(manifest).map(|(row, manifest)| Scope::Selector {
                            labels: service_selector(&manifest),
                            kind: "Service".into(),
                            name: row.name.clone(),
                        })
                    }
                    _ => generic_rows.get(selected).map(|row| Scope::Owner {
                        uid: row.uid.clone(),
                        kind: singular(st.current_kind),
                        name: row.name.clone(),
                    }),
                };
                if new_scope.is_some() {
                    st.nav_stack.push((st.current_kind, st.scope.take(), selected));
                    st.scope = new_scope;
                    st.sort = None;
                    st.hscroll = 0;
                    st.current_kind = target;
                    st.table_state.select(Some(0));
                    st.search.clear();
                }
            }
            // ←/→ (or h/l) scroll a table sideways when its columns don't
            // all fit; the title shows `‹ ›` for what's out of view.
            KeyCode::Left | KeyCode::Char('h') => st.hscroll = st.hscroll.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => st.hscroll += 1,
            // `s` sorts: the column numbers in the header light up and the
            // next digit picks one.
            KeyCode::Char('s') if column_count(st.current_kind, *generic_columns) > 0 => st.sort_choosing = true,
            // `n` gives a namespace one of the number keys 1-9. On the
            // Namespaces list it acts on the highlighted row right
            // away; from every other view it first shows the
            // namespaces to choose from.
            KeyCode::Char('n') => {
                if st.current_kind == ResourceKind::Namespaces {
                    if let Some(name) = st.table_state.selected().and_then(|i| generic_rows.get(i)).map(|r| r.name.clone()) {
                        st.mode = key_picker(name, &st.favorites);
                    }
                } else {
                    let names: Vec<String> =
                        catalog.resolve(ResourceKind::Namespaces, &client).map(|k| k.rows()).unwrap_or_default().into_iter().map(|r| r.name).collect();
                    open_namespace_picker(&mut st.mode, names);
                }
            }
            KeyCode::Char('j') | KeyCode::Down => select_next(&mut st.table_state, row_count),
            KeyCode::Char('k') | KeyCode::Up => select_prev(&mut st.table_state, row_count),
            KeyCode::Char('m') => {
                st.mode = Mode::Menu { selected: menu_position_for(st.current_kind, &catalog.crds) };
            }
            KeyCode::Char('d') => match st.current_kind {
                ResourceKind::Overview => unreachable!("handled in the Overview-specific arm above"),
                ResourceKind::Pods => {
                    if let Some(pod) = st.table_state.selected().and_then(|i| pods.get(i)) {
                        open_spec(&mut st.mode, title_for(pod.metadata.namespace.as_deref(), pod.metadata.name.as_deref()), pod.as_ref());
                    }
                }
                ResourceKind::Deployments => {
                    if let Some(dep) = st.table_state.selected().and_then(|i| deployments.get(i)) {
                        open_spec(&mut st.mode, title_for(dep.metadata.namespace.as_deref(), dep.metadata.name.as_deref()), dep.as_ref());
                    }
                }
                // Indexes straight into the (already filtered)
                // `sorted_nodes`, not through `generic_rows`/
                // `catalog.resolve` — those re-snapshot unfiltered,
                // which would misalign with what's actually
                // displayed whenever a search is active.
                ResourceKind::Nodes => {
                    if let Some(node) = st.table_state.selected().and_then(|i| sorted_nodes.get(i)) {
                        open_spec(&mut st.mode, node.metadata.name.clone().unwrap_or_default(), node.as_ref());
                    }
                }
                _ => {
                    // `table_state.selected()` is a position in the
                    // *filtered* display; `generic_visible` maps it
                    // back to `spec_at`'s real index.
                    if let Some(display_index) = st.table_state.selected()
                        && let Some(&real_index) = generic_visible.get(display_index)
                        && let Some(row) = generic_rows_full.get(real_index)
                        && let Some(value) = catalog.resolve(st.current_kind, &client).and_then(|k| k.spec_at(real_index))
                    {
                        let title = format!("{}/{}", row.namespace, row.name);
                        open_spec_value(&mut st.mode, title, value);
                    }
                }
            },
            // Edit the selected resource in `$EDITOR` (see `edit`) —
            // the same manifest `d` shows, for every kind that has
            // a selectable row.
            KeyCode::Char('e') => {
                if let Some(manifest) = selected_manifest(st, cx.d, catalog, client) {
                    let outcome = edit::edit_resource(cx.terminal, &client, st.mouse_capture_enabled, &manifest);
                    st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back: Box::new(Mode::List) };
                }
            }
            // Actions on the selected object (see `actions`).
            KeyCode::Char(c @ ('D' | 'S' | 'r' | 'o' | 'u' | 't' | 'a')) => {
                if let Some(target) = selected_manifest(st, cx.d, catalog, client).as_ref().and_then(Target::from_manifest) {
                    let action = match c {
                        'D' => Some(Action::Delete),
                        'r' if target.restartable() => Some(Action::Restart),
                        'o' => target.cordon_action(),
                        'u' => target.suspend_action(),
                        't' if target.kind == "CronJob" => Some(Action::Trigger),
                        _ => None,
                    };
                    if c == 'S' && target.scalable() {
                        st.mode = Mode::Scale { input: target.replicas().to_string(), target, back: Box::new(Mode::List) };
                    } else if c == 'a' && target.kind == "Pod" {
                        open_shell(st, cx, &target);
                    } else if let Some(action) = action {
                        if let Some(text) = action.confirmation(&target) {
                            st.mode = Mode::Confirm { text, target, action, back: Box::new(Mode::List) };
                        } else {
                            let outcome = actions::run(client, &target, action);
                            st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back: Box::new(Mode::List) };
                        }
                    }
                }
            }
            KeyCode::Enter if matches!(st.current_kind, ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_)) => {
                if let Some(index) = st.table_state.selected()
                    && let Some((real_index, crd)) = crd_rows.get(index)
                {
                    st.switch_kind(ResourceKind::CustomResource(*real_index, crd.kind));
                }
            }
            KeyCode::Enter if st.current_kind == ResourceKind::Pods => {
                if let Some(pod) = st.table_state.selected().and_then(|i| pods.get(i)) {
                    let title = title_for(pod.metadata.namespace.as_deref(), pod.metadata.name.as_deref());
                    let pod_namespace = pod.metadata.namespace.clone().unwrap_or_default();
                    let name = pod.metadata.name.clone().unwrap_or_default();
                    let containers = k8s::containers_for(pod);
                    st.mode = Mode::Containers {
                        title,
                        namespace: pod_namespace,
                        pod: name,
                        containers,
                        state: TableState::default().with_selected(0),
                        sort: ListSort::default(),
                        back: Box::new(Mode::List),
                    };
                }
            }
            // Freelens-style node drill-down: what's actually running
            // on this node, plus its own CPU/Memory/Pods gauges.
            KeyCode::Enter if st.current_kind == ResourceKind::Nodes => {
                if let Some(node) = st.table_state.selected().and_then(|i| sorted_nodes.get(i)) {
                    let name = node.metadata.name.clone().unwrap_or_default();
                    st.mode = Mode::NodeDetail { name, state: TableState::default().with_selected(0), sort: ListSort::default(), search: String::new(), editing: false, back: Box::new(Mode::List) };
                }
            }
            KeyCode::Char('/') | KeyCode::Char('f') => {
                st.mode = Mode::Search;
            }
            _ => {}
        },
        _ => {}
    }
    if open {
        return handle(Event::Key(KeyCode::Enter.into()), st, cx);
    }
    Ok(None)
}

/// Two clicks on one row this close together are a double-click.
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

/// Scrolls the overview's columns so the selected tile is on screen.
fn keep_overview_selection_visible(st: &mut State, overview: &k8s::Overview, frame_area: Rect) {
    let columns_area = ui::columns_area(ui::body_area(frame_area, false), overview);
    if let ui::OverviewSelection::Header(c) | ui::OverviewSelection::Item(c, _) = st.overview_selection {
        let cols_visible = ui::visible_columns(columns_area.width, overview.catalog.len());
        st.overview_col_scroll = ui::scroll_columns_to_show(st.overview_col_scroll, cols_visible, c);
        let target_item = match st.overview_selection {
            ui::OverviewSelection::Item(_, i) => i,
            _ => 0,
        };
        let items_visible = ui::visible_items_per_column(columns_area.height, ui::column_item_height(overview, c));
        st.overview_item_scroll = ui::scroll_columns_to_show(st.overview_item_scroll, items_visible, target_item);
    }
}

/// The manifest of the row the cursor is on, for every kind with rows.
fn selected_manifest(st: &State, d: &Derived, catalog: &mut Catalog, client: &Client) -> Option<serde_yaml::Value> {
    let selected = st.table_state.selected()?;
    match st.current_kind {
        ResourceKind::Overview | ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => None,
        ResourceKind::Pods => d.pods.get(selected).map(|p| k8s::manifest_value(p.as_ref())),
        ResourceKind::Deployments => d.deployments.get(selected).map(|x| k8s::manifest_value(x.as_ref())),
        ResourceKind::Nodes => d.sorted_nodes.get(selected).map(|n| k8s::manifest_value(n.as_ref())),
        kind => {
            let real = *d.generic_visible.get(selected)?;
            catalog.resolve(kind, client).and_then(|k| k.spec_at(real))
        }
    }
}

/// `a` on a pod: a shell in its container, or the container list when
/// there is more than one to choose from.
fn open_shell(st: &mut State, cx: &mut Cx, target: &Target) {
    let pod: k8s_openapi::api::core::v1::Pod = match serde_yaml::from_value(target.manifest.clone()) {
        Ok(pod) => pod,
        Err(_) => return,
    };
    let containers = k8s::containers_for(&pod);
    let namespace = target.namespace.clone().unwrap_or_default();
    match containers.as_slice() {
        [only] => {
            let name = only.name.clone();
            run_shell(st, cx, &namespace, &target.name, &name);
        }
        _ => {
            st.mode = Mode::Containers {
                title: title_for(Some(&namespace), Some(&target.name)),
                namespace,
                pod: target.name.clone(),
                containers,
                state: TableState::default().with_selected(0),
                sort: ListSort::default(),
                back: Box::new(Mode::List),
            };
        }
    }
}

pub(super) fn run_shell(st: &mut State, cx: &mut Cx, namespace: &str, pod: &str, container: &str) {
    if let Some(outcome) = actions::shell(cx.terminal, st.mouse_capture_enabled, cx.active_context, namespace, pod, container) {
        st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back: Box::new(Mode::List) };
    }
}
