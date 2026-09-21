//! The interactive loop: draws the current `Mode` and handles keyboard/mouse input.

mod derive;
mod draw;
mod state;

use state::State;

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    terminal: &mut ratatui::DefaultTerminal,
    pod_store: &Store<Pod>,
    dep_store: &Store<Deployment>,
    node_store: &Store<Node>,
    event_store: &Store<k8s_openapi::api::core::v1::Event>,
    node_metrics_rx: &watch::Receiver<Option<metrics::ClusterUsage>>,
    catalog: &mut Catalog,
    client: Client,
    config: &Config,
    active_context: &str,
    header: &ui::HeaderInfo,
) -> Result<Outcome> {
    let mut st = State::new(active_context);

    loop {
        let src = derive::Sources { pod_store, dep_store, node_store, event_store, node_metrics_rx, client: &client };
        let query = derive::Query { current_kind: st.current_kind, namespace: st.namespace.as_deref(), scope: st.scope.as_ref(), search: &st.search, sort: st.sort };
        let derive::Derived { pods, pod_rows, deployments, dep_rows, nodes, usage, node_detail_pods, node_detail_rows, sorted_nodes, node_rows, overview, generic_headers, generic_rows_full, generic_visible, generic_columns, generic_rows, crd_rows } = derive::derive(&src, catalog, &st.mode, &query);

        let row_count = match st.current_kind {
            ResourceKind::Overview => overview.events.len(),
            ResourceKind::Pods => pod_rows.len(),
            ResourceKind::Deployments => dep_rows.len(),
            ResourceKind::Nodes => node_rows.len(),
            ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => crd_rows.len(),
            _ => generic_rows.len(),
        };
        // Selection can't outrun the list as pods/deployments come and go
        // underneath it. Overview has no selectable row — it scrolls
        // instead (see `overview_scroll`) — so this only matters for
        // Pods/Deployments.
        if st.current_kind != ResourceKind::Overview && row_count > 0 {
            let clamped = st.table_state.selected().unwrap_or(0).min(row_count - 1);
            st.table_state.select(Some(clamped));
        }

        // Logs keep arriving in the background regardless of what key was
        // last pressed — drain whatever's ready before every redraw.
        if let Mode::Logs { lines, rx, .. } = &mut st.mode {
            while let Ok(line) = rx.try_recv() {
                lines.push(line);
            }
        }

        let rows_view = || match st.current_kind {
            ResourceKind::Overview => ui::Rows::Overview(&overview, st.overview_selection, st.overview_col_scroll, st.overview_item_scroll),
            ResourceKind::Pods => ui::Rows::Pods(&pod_rows),
            ResourceKind::Deployments => ui::Rows::Deployments(&dep_rows),
            ResourceKind::Nodes => ui::Rows::Nodes(&node_rows),
            ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => ui::Rows::CrdList(&crd_rows, st.current_kind.label()),
            _ => ui::Rows::Generic(&generic_rows, st.current_kind.label(), &generic_headers),
        };

        let header_now = ui::HeaderInfo {
            namespace: st.namespace.clone().unwrap_or_else(|| "all".into()),
            scope: st.scope.as_ref().map(Scope::label).unwrap_or_default(),
            namespace_slots: st.favorites.slots.clone(),
            ..header.clone()
        };
        let sort_view = ui::SortState { column: st.sort.map(|s| s.column), descending: st.sort.is_some_and(|s| s.descending), choosing: st.sort_choosing };
        let breadcrumb_segments = breadcrumb(&st.mode, location(st.current_kind, &st.nav_stack, st.scope.as_ref()));
        let mut hints = hints_for(&st.mode, st.current_kind);
        if !hints.is_empty() {
            hints.push(("c", if st.mouse_capture_enabled { "mouse off" } else { "mouse on" }));
        }
        let view = draw::View {
            rows: &rows_view,
            overview: &overview,
            nodes: &nodes,
            usage: usage.as_ref(),
            node_detail_rows: &node_detail_rows,
            crds: &catalog.crds,
            favorites: &st.favorites,
            hints: &hints,
            show_hints_panel: st.show_hints_panel,
            breadcrumb: &breadcrumb_segments,
            header_now: &header_now,
            search: &st.search,
            sort_view,
        };
        let frame_area = draw::draw_mode(terminal, &mut st.mode, &view, &mut st.table_state, st.hovered, &mut st.icons, &mut st.hscroll)?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        // Handle every event already queued before looping back to
        // redraw — not just the one that just arrived. A trackpad
        // "flick" scroll can queue up dozens of mouse-wheel events at
        // once; without this, each one triggered its own full redraw,
        // and a keypress typed right after (like `q`) sat behind that
        // whole backlog instead of being handled almost immediately.
        loop {
            let event = event::read()?;
            // `s` and the digits sort a popup's table when one has focus.
            let sort_key_used = matches!(&event, Event::Key(key) if popup_sort_key(&mut st.mode, key.code));
            match (event, &mut st.mode) {
                (Event::Key(_), _) if sort_key_used => {}
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
                            if column < column_count(st.current_kind, generic_columns) {
                                st.sort = Some(SortSpec::pressed(st.sort, column));
                                st.table_state.select(Some(0));
                            }
                        }
                        _ => st.sort_choosing = false,
                    }
                }
                // Any key (or click) closes a notice — checked before the
                // global keys below so they don't also fire on that press.
                (Event::Key(key), Mode::NamespacePick { filter, editing: editing @ true, state, .. }) => match key.code {
                    KeyCode::Esc => {
                        filter.clear();
                        *editing = false;
                    }
                    KeyCode::Enter => *editing = false,
                    KeyCode::Backspace => {
                        filter.pop();
                        state.select(Some(0));
                    }
                    KeyCode::Char(c) => {
                        filter.push(c);
                        state.select(Some(0));
                    }
                    _ => {}
                },
                (Event::Key(key), Mode::NamespacePick { names, filter, editing, state, sort, back }) => {
                    let mut chosen: Option<String> = None;
                    let mut close = false;
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => close = true,
                        KeyCode::Char('/') | KeyCode::Char('f') => *editing = true,
                        KeyCode::Char('j') | KeyCode::Down => select_next(state, filtered_names(names, filter, *sort, &st.favorites).len()),
                        KeyCode::Char('k') | KeyCode::Up => select_prev(state, filtered_names(names, filter, *sort, &st.favorites).len()),
                        KeyCode::Enter => chosen = state.selected().and_then(|i| filtered_names(names, filter, *sort, &st.favorites).get(i).map(|n| (*n).clone())),
                        _ => {}
                    }
                    if let Some(name) = chosen {
                        // Straight on to choosing its key; Esc from there goes
                        // back to the view this was opened from.
                        let back = std::mem::replace(&mut **back, Mode::List);
                        let mut next = key_picker(name, &st.favorites);
                        if let Mode::Slots { back: slot_back, .. } = &mut next {
                            *slot_back = Box::new(back);
                        }
                        st.mode = next;
                    } else if close {
                        st.mode = std::mem::replace(&mut **back, Mode::List);
                    }
                }
                (Event::Mouse(mouse), Mode::NamespacePick { names, filter, state, sort, back, .. }) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                    let matches = filtered_names(names, filter, *sort, &st.favorites);
                    if let Some(idx) = ui::event_row_at(frame_area, matches.len(), state.offset(), mouse.row) {
                        let name = matches[idx].clone();
                        let back = std::mem::replace(&mut **back, Mode::List);
                        let mut next = key_picker(name, &st.favorites);
                        if let Mode::Slots { back: slot_back, .. } = &mut next {
                            *slot_back = Box::new(back);
                        }
                        st.mode = next;
                    }
                }
                (Event::Key(key), Mode::Slots { namespace, selected, back }) => {
                    let mut assign_to = None;
                    let mut close = false;
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => close = true,
                        KeyCode::Char('j') | KeyCode::Down => *selected = (*selected + 1).min(favorites::SLOTS - 1),
                        KeyCode::Char('k') | KeyCode::Up => *selected = selected.saturating_sub(1),
                        KeyCode::Char(c @ '1'..='9') => assign_to = c.to_digit(10).map(|d| d as usize),
                        KeyCode::Enter => assign_to = Some(*selected + 1),
                        KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
                            st.favorites.clear(*selected + 1);
                            st.favorites.save(active_context);
                        }
                        _ => {}
                    }
                    if let Some(key_number) = assign_to {
                        st.favorites.assign(key_number, namespace);
                        st.favorites.save(active_context);
                        close = true;
                    }
                    if close {
                        st.mode = std::mem::replace(&mut **back, Mode::List);
                    }
                }
                (Event::Key(_), Mode::Notice { back, .. }) => st.mode = std::mem::replace(&mut **back, Mode::List),
                (Event::Mouse(m), Mode::Notice { back, .. }) if matches!(m.kind, MouseEventKind::Down(_)) => {
                    st.mode = std::mem::replace(&mut **back, Mode::List)
                }
                // Toggling mouse reporting off hands click-drag text
                // selection (and therefore copy) back to the terminal
                // itself — the only thing enabling it took away. Skipped
                // while typing a command/search, where `c` is just a
                // character to type, not this toggle.
                (Event::Key(key), current_mode)
                    if key.code == KeyCode::Char('c') && !is_typing(current_mode) =>
                {
                    st.mouse_capture_enabled = !st.mouse_capture_enabled;
                    if st.mouse_capture_enabled {
                        execute!(stdout(), EnableMouseCapture)?;
                    } else {
                        execute!(stdout(), DisableMouseCapture)?;
                    }
                }
                // `?` toggles the commands panel — reachable from any
                // screen (except while typing), same as `c`.
                (Event::Key(key), current_mode)
                    if key.code == KeyCode::Char('?') && !is_typing(current_mode) =>
                {
                    st.show_hints_panel = !st.show_hints_panel;
                }
                // `:` opens the command line from anywhere — captures
                // whatever mode was actually active as `back`, so Esc (or
                // Enter on a command that doesn't switch kind) returns to
                // exactly where this was opened from, not always `List`.
                (Event::Key(key), current_mode)
                    if key.code == KeyCode::Char(':') && !is_typing(current_mode) =>
                {
                    let back = Box::new(std::mem::replace(current_mode, Mode::List));
                    *current_mode = Mode::Command { input: String::new(), selected: 0, back };
                }
                // `C` opens the context switcher from anywhere, same as `:ctx`.
                (Event::Key(key), current_mode)
                    if key.code == KeyCode::Char('C') && !is_typing(current_mode) =>
                {
                    open_context_switcher(current_mode, active_context);
                }
                (Event::Mouse(mouse), Mode::List) if mouse.kind == MouseEventKind::Moved || matches!(mouse.kind, MouseEventKind::Down(_)) => {
                    if st.current_kind == ResourceKind::Overview {
                        let active_col = match st.overview_selection {
                            ui::OverviewSelection::Header(c) | ui::OverviewSelection::Item(c, _) => c,
                            ui::OverviewSelection::Resources | ui::OverviewSelection::Events => usize::MAX,
                        };
                        if matches!(mouse.kind, MouseEventKind::Down(_))
                            && let Some(hit) = ui::column_hit(
                                ui::body_area(frame_area, false),
                                &overview,
                                st.overview_col_scroll,
                                active_col,
                                st.overview_item_scroll,
                                mouse.column,
                                mouse.row,
                            )
                        {
                            st.overview_selection = hit;
                        }
                    } else {
                        st.hovered = ui::row_at(ui::body_area(frame_area, true), &pod_rows, st.hscroll, &st.table_state, row_count, mouse.column, mouse.row).map(|row| {
                            ui::Hover { row, column: mouse.column, row_on_screen: mouse.row }
                        });
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
                    let columns_area = ui::columns_area(ui::body_area(frame_area, false), &overview);
                    let cols_visible = ui::visible_columns(columns_area.width, overview.catalog.len());
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
                    if let ui::OverviewSelection::Header(c) | ui::OverviewSelection::Item(c, _) = st.overview_selection {
                        st.overview_col_scroll = ui::scroll_columns_to_show(st.overview_col_scroll, cols_visible, c);
                        let target_item = match st.overview_selection {
                            ui::OverviewSelection::Item(_, i) => i,
                            _ => 0,
                        };
                        let items_visible = ui::visible_items_per_column(columns_area.height, ui::column_item_height(&overview, c));
                        st.overview_item_scroll = ui::scroll_columns_to_show(st.overview_item_scroll, items_visible, target_item);
                    }
                }
                (Event::Key(key), Mode::Events { search, editing: editing @ true, state, .. }) => match key.code {
                    // Esc clears the search and leaves typing; Enter keeps
                    // it applied and goes back to browsing the matches.
                    KeyCode::Esc => {
                        search.clear();
                        *editing = false;
                    }
                    KeyCode::Enter => *editing = false,
                    KeyCode::Backspace => {
                        search.pop();
                        state.select(Some(0));
                    }
                    KeyCode::Char(c) => {
                        search.push(c);
                        state.select(Some(0));
                    }
                    _ => {}
                },
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
                    let cols = ui::column_detail_cols(frame_area);
                    // The scroll recompute has to happen inside each
                    // navigation branch, not after the whole match — the
                    // Enter/Esc branches below reassign `mode` itself, which
                    // would leave `selected`/`row_scroll` dangling if used
                    // afterward.
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
                    KeyCode::Char('s') if column_count(st.current_kind, generic_columns) > 0 => st.sort_choosing = true,
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
                        let manifest: Option<serde_yaml::Value> = match st.current_kind {
                            ResourceKind::Overview | ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => None,
                            ResourceKind::Pods => st.table_state.selected().and_then(|i| pods.get(i)).map(|p| k8s::manifest_value(p.as_ref())),
                            ResourceKind::Deployments => {
                                st.table_state.selected().and_then(|i| deployments.get(i)).map(|d| k8s::manifest_value(d.as_ref()))
                            }
                            ResourceKind::Nodes => st.table_state.selected().and_then(|i| sorted_nodes.get(i)).map(|n| k8s::manifest_value(n.as_ref())),
                            _ => st.table_state
                                .selected()
                                .and_then(|i| generic_visible.get(i).copied())
                                .and_then(|real| catalog.resolve(st.current_kind, &client).and_then(|k| k.spec_at(real))),
                        };
                        if let Some(manifest) = manifest {
                            let outcome = edit::edit_resource(terminal, &client, st.mouse_capture_enabled, &manifest);
                            st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back: Box::new(Mode::List) };
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
                (Event::Key(key), Mode::Command { input, selected, back }) => match key.code {
                    KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => {
                        let len = command_suggestions(input, &catalog.crds).len();
                        *selected = (*selected + 1).min(len.saturating_sub(1));
                    }
                    KeyCode::Enter => {
                        let cmd = input.trim().to_lowercase();
                        // The highlighted autocomplete suggestion wins
                        // when there is one; `from_command` is only the
                        // fallback for an exact alias that didn't happen
                        // to fuzzy-score into the visible list.
                        let suggestions = command_suggestions(input, &catalog.crds);
                        let highlighted = suggestions.get(*selected).map(|s| s.cmd);
                        if matches!(highlighted, Some(Cmd::Quit)) || matches!(cmd.as_str(), "q" | "quit" | "exit") {
                            return Ok(Outcome::Quit);
                        }
                        if matches!(highlighted, Some(Cmd::Events)) {
                            st.mode = Mode::Events { filter: k8s::EventFilter::All, search: String::new(), editing: false, state: TableState::default().with_selected(0), sort: ListSort::default() };
                        } else if is_context_command(&cmd) || matches!(highlighted, Some(Cmd::Context)) {
                            let mut opened = std::mem::replace(&mut **back, Mode::List);
                            open_context_switcher(&mut opened, active_context);
                            st.mode = opened;
                        } else if let Some(kind) = match highlighted {
                            Some(Cmd::Kind(k)) => Some(k),
                            _ => k8s::ResourceKind::from_command(&cmd),
                        } {
                            st.switch_kind(kind);
                            st.mode = Mode::List;
                        } else {
                            st.mode = std::mem::replace(&mut **back, Mode::List);
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
                (Event::Key(key), Mode::Context { filter, editing: true, state, error, .. }) => match key.code {
                    KeyCode::Esc => {
                        filter.clear();
                        if let Mode::Context { editing, .. } = &mut st.mode {
                            *editing = false;
                        }
                    }
                    KeyCode::Enter => {
                        if let Mode::Context { editing, .. } = &mut st.mode {
                            *editing = false;
                        }
                    }
                    KeyCode::Backspace => {
                        filter.pop();
                        state.select(Some(0));
                        *error = None;
                    }
                    KeyCode::Char(c) => {
                        filter.push(c);
                        state.select(Some(0));
                        *error = None;
                    }
                    _ => {}
                },
                (Event::Key(key), Mode::Context { contexts, filter, editing, state, error, sort, back }) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
                    KeyCode::Char('/') | KeyCode::Char('f') => *editing = true,
                    KeyCode::Char('j') | KeyCode::Down => select_next(state, filtered_contexts(contexts, filter, *sort).len()),
                    KeyCode::Char('k') | KeyCode::Up => select_prev(state, filtered_contexts(contexts, filter, *sort).len()),
                    KeyCode::Enter => {
                        let name = state.selected().and_then(|i| filtered_contexts(contexts, filter, *sort).get(i).map(|c| c.name.clone()));
                        if let Some(name) = name {
                            match switch_target(&name, active_context) {
                                Ok(true) => return Ok(Outcome::SwitchContext(name)),
                                Ok(false) => st.mode = std::mem::replace(&mut **back, Mode::List),
                                Err(msg) => *error = Some(msg),
                            }
                        }
                    }
                    _ => {}
                },
                (Event::Mouse(mouse), Mode::Context { contexts, filter, state, error, sort, back, .. }) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                    let matches = filtered_contexts(contexts, filter, *sort);
                    if let Some(idx) = ui::event_row_at(frame_area, matches.len(), state.offset(), mouse.row) {
                        state.select(Some(idx));
                        let name = matches[idx].name.clone();
                        match switch_target(&name, active_context) {
                            Ok(true) => return Ok(Outcome::SwitchContext(name)),
                            Ok(false) => st.mode = std::mem::replace(&mut **back, Mode::List),
                            Err(msg) => *error = Some(msg),
                        }
                    }
                }
                (Event::Key(key), Mode::Search) => match key.code {
                    KeyCode::Esc => {
                        st.search.clear();
                        st.mode = Mode::List;
                    }
                    KeyCode::Enter => st.mode = Mode::List,
                    KeyCode::Backspace => {
                        st.search.pop();
                    }
                    KeyCode::Char(c) => st.search.push(c),
                    _ => {}
                },
                (Event::Key(key), Mode::Menu { selected }) => {
                    let sections = menu_sections(&catalog.crds);
                    let cols = ui::menu_cols(frame_area);
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => st.mode = Mode::List,
                        KeyCode::Char('h') | KeyCode::Left => {
                            *selected = ui::move_menu_selection(&sections, cols, *selected, ui::Direction::Left);
                        }
                        KeyCode::Char('l') | KeyCode::Right => {
                            *selected = ui::move_menu_selection(&sections, cols, *selected, ui::Direction::Right);
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            *selected = ui::move_menu_selection(&sections, cols, *selected, ui::Direction::Up);
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            *selected = ui::move_menu_selection(&sections, cols, *selected, ui::Direction::Down);
                        }
                        KeyCode::Enter => {
                            if let Some(kind) = sections.get(selected.0).and_then(|s| s.tiles.get(selected.1)) {
                                st.switch_kind(*kind);
                                st.mode = Mode::List;
                            }
                        }
                        _ => {}
                    }
                }
                (Event::Key(key), Mode::Spec { viewing: viewing @ Some(_), .. }) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => *viewing = None,
                    _ => {}
                },
                (Event::Key(key), Mode::Spec { items, state, expanded_all, leaf_values, viewing, back, .. }) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
                    KeyCode::Char('j') | KeyCode::Down => {
                        state.key_down();
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        state.key_up();
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        state.toggle_selected();
                    }
                    // Toggles between "everything open" and "everything
                    // closed" — `TreeState` only gives us the latter as a
                    // single call, so expanding needs walking every
                    // identifier ourselves.
                    KeyCode::Char('a') => {
                        if *expanded_all {
                            state.close_all();
                        } else {
                            let mut ids = Vec::new();
                            all_tree_identifiers(items, &mut Vec::new(), &mut ids);
                            for id in ids {
                                state.open(id);
                            }
                        }
                        *expanded_all = !*expanded_all;
                    }
                    // Shows the selected leaf's full value, untruncated —
                    // a no-op on a branch node (nothing in `leaf_values`
                    // for it).
                    KeyCode::Char('v') => {
                        if let Some(id) = state.selected().last()
                            && let Some((label, value)) = leaf_values.get(id)
                        {
                            *viewing = Some((label.clone(), value.clone()));
                        }
                    }
                    _ => {}
                },
                (Event::Mouse(mouse), Mode::Spec { state, .. }) => match mouse.kind {
                    MouseEventKind::Down(_) => ui::click_tree(state, mouse.column, mouse.row),
                    MouseEventKind::ScrollDown => {
                        state.scroll_down(1);
                    }
                    MouseEventKind::ScrollUp => {
                        state.scroll_up(1);
                    }
                    _ => {}
                },
                (Event::Key(key), Mode::Containers { title, namespace, pod, containers, state, sort, back }) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        st.mode = std::mem::replace(&mut **back, Mode::List);
                    }
                    KeyCode::Char('j') | KeyCode::Down => select_next(state, containers.len()),
                    KeyCode::Char('k') | KeyCode::Up => select_prev(state, containers.len()),
                    KeyCode::Enter => {
                        let shown = sorted_containers(containers, *sort);
                        if let Some(container) = state.selected().and_then(|i| shown.get(i)) {
                            let log_title = format!("{namespace}/{pod}/{}", container.name);
                            let (rx, handle) =
                                k8s::stream_logs(client.clone(), namespace.clone(), pod.clone(), container.name.clone());
                            let containers_snapshot = Mode::Containers {
                                title: title.clone(),
                                namespace: namespace.clone(),
                                pod: pod.clone(),
                                containers: containers.clone(),
                                state: *state,
                                sort: *sort,
                                back: std::mem::replace(back, Box::new(Mode::List)),
                            };
                            st.mode = Mode::Logs {
                                title: log_title,
                                lines: Vec::new(),
                                scroll: 0,
                                follow: true,
                                timestamp_format: config.logs.timestamp_format,
                                order: config.logs.order,
                                rx,
                                handle,
                                filter: String::new(),
                                filter_editing: false,
                                back: Box::new(containers_snapshot),
                            };
                        }
                    }
                    _ => {}
                },
                (Event::Key(key), Mode::NodeDetail { search, editing: editing @ true, state, .. }) => match key.code {
                    // Esc clears the search; Enter keeps it and goes back to
                    // browsing the matches.
                    KeyCode::Esc => {
                        search.clear();
                        *editing = false;
                    }
                    KeyCode::Enter => *editing = false,
                    KeyCode::Backspace => {
                        search.pop();
                        state.select(Some(0));
                    }
                    KeyCode::Char(c) => {
                        search.push(c);
                        state.select(Some(0));
                    }
                    _ => {}
                },
                (Event::Key(key), Mode::NodeDetail { name, state, sort, search, editing, back }) => match key.code {
                    KeyCode::Char('/') | KeyCode::Char('f') => *editing = true,
                    KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
                    KeyCode::Char('d') => {
                        if let Some(node) = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str())) {
                            let title = name.clone();
                            open_spec(&mut st.mode, title, node.as_ref());
                        }
                    }
                    KeyCode::Char('e') => {
                        if let Some(node) = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str())) {
                            let manifest = k8s::manifest_value(node.as_ref());
                            let back = Box::new(Mode::NodeDetail {
                                name: name.clone(),
                                state: *state,
                                sort: *sort,
                                search: search.clone(),
                                editing: false,
                                back: std::mem::replace(back, Box::new(Mode::List)),
                            });
                            let outcome = edit::edit_resource(terminal, &client, st.mouse_capture_enabled, &manifest);
                            st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back };
                        }
                    }
                    KeyCode::Char('j') | KeyCode::Down => select_next(state, node_detail_rows.len()),
                    KeyCode::Char('k') | KeyCode::Up => select_prev(state, node_detail_rows.len()),
                    KeyCode::Enter => {
                        if let Some(pod) = state.selected().and_then(|i| node_detail_pods.get(i)) {
                            let title = title_for(pod.metadata.namespace.as_deref(), pod.metadata.name.as_deref());
                            let pod_namespace = pod.metadata.namespace.clone().unwrap_or_default();
                            let pod_name = pod.metadata.name.clone().unwrap_or_default();
                            let containers = k8s::containers_for(pod);
                            let node_detail_snapshot = Mode::NodeDetail {
                                name: name.clone(),
                                state: *state,
                                sort: *sort,
                                search: search.clone(),
                                editing: false,
                                back: std::mem::replace(back, Box::new(Mode::List)),
                            };
                            st.mode = Mode::Containers {
                                title,
                                namespace: pod_namespace,
                                pod: pod_name,
                                containers,
                                state: TableState::default().with_selected(0),
                                sort: ListSort::default(),
                                back: Box::new(node_detail_snapshot),
                            };
                        }
                    }
                    _ => {}
                },
                (Event::Key(key), Mode::Logs { filter, filter_editing: filter_editing @ true, .. }) => match key.code {
                    // Esc while typing clears the filter rather than
                    // leaving; Enter confirms and goes back to normal
                    // scrolling with it applied — the filtered set is
                    // what you land back on, having "scrolled past"
                    // everything that didn't match.
                    KeyCode::Esc => {
                        filter.clear();
                        *filter_editing = false;
                    }
                    KeyCode::Enter => *filter_editing = false,
                    KeyCode::Backspace => {
                        filter.pop();
                    }
                    KeyCode::Char(c) => filter.push(c),
                    _ => {}
                },
                (Event::Key(key), Mode::Logs { lines, filter, scroll, follow, timestamp_format, order, handle, filter_editing, back, .. }) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        handle.abort();
                        st.mode = std::mem::replace(&mut **back, Mode::List);
                    }
                    KeyCode::Char('j') | KeyCode::Down => ui::logs_scroll_down(frame_area, lines, filter, *order, follow, scroll),
                    KeyCode::Char('k') | KeyCode::Up => ui::logs_scroll_up(frame_area, lines, filter, *order, follow, scroll),
                    KeyCode::Char('G') => *follow = true,
                    KeyCode::Char('/') => *filter_editing = true,
                    KeyCode::Char(c) if c == config.keybindings.logs.toggle_timestamp => {
                        *timestamp_format = timestamp_format.toggled();
                    }
                    KeyCode::Char(c) if c == config.keybindings.logs.toggle_order => {
                        *order = order.toggled();
                        *follow = true;
                    }
                    _ => {}
                },
                (Event::Mouse(mouse), Mode::Logs { lines, filter, scroll, follow, order, .. }) => match mouse.kind {
                    MouseEventKind::ScrollDown => ui::logs_scroll_down(frame_area, lines, filter, *order, follow, scroll),
                    MouseEventKind::ScrollUp => ui::logs_scroll_up(frame_area, lines, filter, *order, follow, scroll),
                    _ => {}
                },
                _ => {}
            }
            if !event::poll(Duration::from_millis(0))? {
                break;
            }
        }
    }
}
