//! The interactive loop: draws the current `Mode` and handles keyboard/mouse input.

mod draw;

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
    let mut table_state = TableState::default().with_selected(0);
    let mut mode = Mode::List;
    let mut hovered: Option<ui::Hover> = None;
    let mut current_kind = ResourceKind::Overview;
    // The namespace every namespaced list is narrowed to (`Enter` on a
    // namespace sets it, `0` clears it) — sticks across kind switches.
    let mut namespace: Option<String> = None;
    // What the current list is drilled into (a Deployment's ReplicaSets,
    // a Service's Pods, ...) and how to get back out, one level per
    // entry: the kind, scope and selected row we came from.
    let mut scope: Option<Scope> = None;
    // The list's sort column/direction (`s` then a column number), and
    // whether the next digit is choosing one. Reset when the kind changes.
    let mut sort: Option<SortSpec> = None;
    let mut sort_choosing = false;
    // How many columns the list is scrolled to the right (←/→ or h/l) when
    // its columns don't all fit the screen.
    let mut hscroll: usize = 0;
    // Namespaces reserved to number keys 1-9 (`s` on a namespace).
    let mut favorites = Favorites::load(active_context);
    let mut nav_stack: Vec<(ResourceKind, Option<Scope>, usize)> = Vec::new();
    // The active `/`/`f` filter — empty means "show everything." Persists
    // across `Mode::Search`/`Mode::List` so confirming a search (Enter)
    // keeps the list narrowed while you go on navigating it; switching
    // resource kind (`m`, `:`, Esc back to Overview) clears it, since a
    // filter meant for one kind's names rarely makes sense carried over
    // to a completely different kind's list.
    let mut search = String::new();
    // Mouse reporting is what makes hover/click work, but it's also
    // exactly what stops the terminal's own click-drag text selection
    // (and therefore copy) from working — see the `c` handler below.
    // Starts enabled, same as before this toggle existed.
    let mut mouse_capture_enabled = true;
    // Toggled by `?` — whether the commands panel (the current screen's
    // keybinding hints, off to the side) is currently open. Starts
    // closed so the screen starts clean; only the small "?: cmds"
    // indicator is always there (and only outside the main Overview).
    let mut show_hints_panel = false;
    // Queries the terminal's actual graphics capability (Kitty/Sixel/
    // iTerm2, falling back to halfblocks) — must happen after raw mode is
    // enabled and before the event-read loop below starts, so its own
    // terminal query doesn't race with crossterm's stdin reads.
    let mut icons = icons::IconCache::detect();
    let mut overview_selection = ui::OverviewSelection::Resources;
    // Horizontal scroll offset into the Overview's columns (Cluster,
    // Workloads, Config, ... — one per catalog category).
    let mut overview_col_scroll: usize = 0;
    // Vertical scroll offset into whichever column currently holds the
    // selection — item cards are tall enough now that a category like
    // Workloads can't always fit on screen at once.
    let mut overview_item_scroll: usize = 0;

    loop {
        // `search` only ever has an effect on whichever kind it was typed
        // against — it's cleared on every kind switch (see the `search.
        // clear()` calls alongside `current_kind = ...` below) — so
        // filtering every kind's source list by it unconditionally is
        // safe: for every kind other than the one actively being
        // searched, `search` is "" and `row_matches` always returns true.
        // The Overview stays cluster-wide even with a namespace set.
        let ns_filter: Option<&str> = if current_kind == ResourceKind::Overview { None } else { namespace.as_deref() };
        let in_namespace = |meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta| {
            ns_filter.is_none_or(|ns| meta.namespace.as_deref() == Some(ns))
        };
        let mut pods: Vec<std::sync::Arc<Pod>> = k8s::snapshot(pod_store)
            .into_iter()
            .filter(|p| in_namespace(&p.metadata))
            .filter(|p| current_kind != ResourceKind::Pods || scope.as_ref().is_none_or(|s| s.matches_meta(&p.metadata)))
            .filter(|p| row_matches(&search, &meta_search_text(&p.metadata)))
            .collect();
        if current_kind == ResourceKind::Pods {
            apply(&mut pods, sort, |p, column| pod_key(&k8s::row_for(p), column));
        }
        let pod_rows: Vec<k8s::PodRow> = pods.iter().map(|p| k8s::row_for(p)).collect();
        let mut deployments: Vec<std::sync::Arc<Deployment>> = k8s::snapshot_deployments(dep_store)
            .into_iter()
            .filter(|d| in_namespace(&d.metadata))
            .filter(|d| row_matches(&search, &meta_search_text(&d.metadata)))
            .collect();
        if current_kind == ResourceKind::Deployments {
            apply(&mut deployments, sort, |d, column| deployment_key(&k8s::row_for_deployment(d), column));
        }
        let dep_rows: Vec<k8s::DeploymentRow> = deployments.iter().map(|d| k8s::row_for_deployment(d)).collect();
        let nodes = node_store.state();
        let events = event_store.state();
        let usage = node_metrics_rx.borrow().clone();
        // Only populated while actually viewing a node's detail — which
        // pod, out of everything on the cluster, is scheduled on this
        // one node. Searches the whole back-chain, not just the top
        // mode, so it's still available when NodeDetail is a dimmed
        // background layer behind Containers/Spec/Logs rather than the
        // focused view itself.
        let mut node_detail_pods: Vec<std::sync::Arc<Pod>> = if let Some(name) = node_detail_name(&mode) {
            pods.iter().filter(|p| p.spec.as_ref().and_then(|s| s.node_name.as_deref()) == Some(name)).cloned().collect()
        } else {
            Vec::new()
        };
        let node_search = node_detail_search(&mode).to_string();
        node_detail_pods.retain(|p| row_matches(&node_search, &meta_search_text(&p.metadata)));
        apply(&mut node_detail_pods, node_detail_sort(&mode), |p, column| pod_key(&k8s::row_for(p), column));
        let node_detail_rows: Vec<k8s::PodRow> = node_detail_pods.iter().map(|p| k8s::row_for(p)).collect();
        // Nodes get their own specialized rows (CPU/Memory visible right
        // in the list) instead of the generic Namespace/Name/Age table.
        // Filtered directly here (not via the generic `catalog`/
        // `generic_rows` path other kinds use) so the 'd'/Enter handlers
        // below, which index straight into `sorted_nodes`, can't drift
        // out of alignment with what's actually displayed.
        // Every pod counts toward its node's PODS, whatever the list's search,
        // namespace or drill-down is currently narrowed to.
        let mut pods_per_node: HashMap<String, usize> = HashMap::new();
        for pod in k8s::snapshot(pod_store) {
            if let Some(node) = pod.spec.as_ref().and_then(|s| s.node_name.clone()) {
                *pods_per_node.entry(node).or_default() += 1;
            }
        }
        let mut node_pairs: Vec<(std::sync::Arc<Node>, k8s::NodeRow)> = k8s::snapshot_generic(node_store)
            .into_iter()
            .filter(|n| row_matches(&search, &n.metadata.name.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|n| {
                let name = n.metadata.name.clone().unwrap_or_default();
                let node_usage = usage.as_ref().and_then(|u| u.for_node(&name));
                let pod_count = pods_per_node.get(&name).copied().unwrap_or(0);
                let row = k8s::node_row(&n, node_usage, pod_count);
                (n, row)
            })
            .collect();
        if current_kind == ResourceKind::Nodes {
            apply(&mut node_pairs, sort, |(_, row), column| node_key(row, column));
        }
        let (sorted_nodes, node_rows): (Vec<std::sync::Arc<Node>>, Vec<k8s::NodeRow>) = node_pairs.into_iter().unzip();
        let catalog_sections = catalog.sections(pod_rows.len(), dep_rows.len());
        let overview = k8s::overview(&nodes, &events, usage.as_ref(), catalog_sections);
        // Only ever populated for whatever kind is currently on screen —
        // computed unconditionally so every match arm below can just read
        // it, same as `pod_rows`/`dep_rows` are always computed too.
        // `resolve` also lazily starts a CRD's watch the first time it's
        // the current kind — "watch on open", not for every installed CRD.
        // `generic_visible` maps a filtered display position back to its
        // real index in `generic_rows_full`/the catalog's own live
        // snapshot — needed because `CatalogKind::spec_at` (the 'd' key)
        // takes that real index, not the display one.
        let generic_headers: Vec<&'static str> = catalog.resolve(current_kind, &client).map(|k| k.headers()).unwrap_or_default();
        let generic_rows_full: Vec<k8s::GenericRow> = catalog.resolve(current_kind, &client).map(|k| k.rows()).unwrap_or_default();
        let mut generic_visible: Vec<usize> = (0..generic_rows_full.len())
            .filter(|&i| {
                let row = &generic_rows_full[i];
                // Cluster-scoped rows (namespace "-") are never hidden by a namespace.
                ns_filter.is_none_or(|ns| row.namespace == "-" || row.namespace == ns)
                    && scope.as_ref().is_none_or(|s| s.matches_row(row))
            })
            .filter(|&i| row_matches(&search, &meta_search_text_generic(&generic_rows_full[i])))
            .collect();
        // Whether the table will show a namespace column — decides which
        // sort column is which.
        let generic_has_namespace = generic_visible.iter().any(|&i| generic_rows_full[i].namespace != "-");
        // The generic table's width: namespace (if shown), name, the kind's own
        // columns, age — what sort digits can reach.
        let generic_columns =
            usize::from(generic_has_namespace) + 1 + generic_headers.len() + 1;
        apply(&mut generic_visible, sort, |&i, column| generic_key(&generic_rows_full[i], column, generic_has_namespace));
        let generic_rows: Vec<k8s::GenericRow> = generic_visible.iter().map(|&i| generic_rows_full[i].clone()).collect();
        // The CRD picker, unfiltered or scoped to one API group — each
        // entry keeps its real index into `catalog.crds` (needed to open
        // the right one on Enter even though this may be a filtered
        // subset of the full list).
        let mut crd_rows: Vec<(usize, k8s::CrdInfo)> = match current_kind {
            ResourceKind::CustomResourceList => catalog
                .crds
                .iter()
                .cloned()
                .enumerate()
                .filter(|(_, c)| row_matches(&search, &format!("{} {}", c.group, c.kind)))
                .collect(),
            ResourceKind::CustomResourceGroup(group) => catalog
                .crds
                .iter()
                .cloned()
                .enumerate()
                .filter(|(_, c)| c.group == group && row_matches(&search, &format!("{} {}", c.group, c.kind)))
                .collect(),
            _ => Vec::new(),
        };
        apply(&mut crd_rows, sort, |(_, crd), column| crd_key(crd, column));

        let row_count = match current_kind {
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
        if current_kind != ResourceKind::Overview && row_count > 0 {
            let clamped = table_state.selected().unwrap_or(0).min(row_count - 1);
            table_state.select(Some(clamped));
        }

        // Logs keep arriving in the background regardless of what key was
        // last pressed — drain whatever's ready before every redraw.
        if let Mode::Logs { lines, rx, .. } = &mut mode {
            while let Ok(line) = rx.try_recv() {
                lines.push(line);
            }
        }

        let rows_view = || match current_kind {
            ResourceKind::Overview => ui::Rows::Overview(&overview, overview_selection, overview_col_scroll, overview_item_scroll),
            ResourceKind::Pods => ui::Rows::Pods(&pod_rows),
            ResourceKind::Deployments => ui::Rows::Deployments(&dep_rows),
            ResourceKind::Nodes => ui::Rows::Nodes(&node_rows),
            ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => ui::Rows::CrdList(&crd_rows, current_kind.label()),
            _ => ui::Rows::Generic(&generic_rows, current_kind.label(), &generic_headers),
        };

        let header_now = ui::HeaderInfo {
            namespace: namespace.clone().unwrap_or_else(|| "all".into()),
            scope: scope.as_ref().map(Scope::label).unwrap_or_default(),
            namespace_slots: favorites.slots.clone(),
            ..header.clone()
        };
        let sort_view = ui::SortState { column: sort.map(|s| s.column), descending: sort.is_some_and(|s| s.descending), choosing: sort_choosing };
        let breadcrumb_segments = breadcrumb(&mode, location(current_kind, &nav_stack, scope.as_ref()));
        let mut hints = hints_for(&mode, current_kind);
        if !hints.is_empty() {
            hints.push(("c", if mouse_capture_enabled { "mouse off" } else { "mouse on" }));
        }
        let view = draw::View {
            rows: &rows_view,
            overview: &overview,
            nodes: &nodes,
            usage: usage.as_ref(),
            node_detail_rows: &node_detail_rows,
            crds: &catalog.crds,
            favorites: &favorites,
            hints: &hints,
            show_hints_panel,
            breadcrumb: &breadcrumb_segments,
            header_now: &header_now,
            search: &search,
            sort_view,
        };
        let frame_area = draw::draw_mode(terminal, &mut mode, &view, &mut table_state, hovered, &mut icons, &mut hscroll)?;

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
            let sort_key_used = matches!(&event, Event::Key(key) if popup_sort_key(&mut mode, key.code));
            match (event, &mut mode) {
                (Event::Key(_), _) if sort_key_used => {}
                // Sort mode (`s`): the headers show their column numbers and a
                // digit sorts by that column — the same one again flips
                // ascending, descending, off. It stays on until `s`, Esc or
                // `q`; every other key still works as usual meanwhile.
                (Event::Key(key), Mode::List)
                    if sort_choosing && matches!(key.code, KeyCode::Char('0'..='9' | 's' | 'q') | KeyCode::Esc) =>
                {
                    match key.code {
                        KeyCode::Char(c @ '0'..='9') => {
                            // 1-9 are columns 1-9; 0 is the tenth.
                            let column = (c as usize + 9 - '0' as usize) % 10;
                            if column < column_count(current_kind, generic_columns) {
                                sort = Some(SortSpec::pressed(sort, column));
                                table_state.select(Some(0));
                            }
                        }
                        _ => sort_choosing = false,
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
                        KeyCode::Char('j') | KeyCode::Down => select_next(state, filtered_names(names, filter, *sort, &favorites).len()),
                        KeyCode::Char('k') | KeyCode::Up => select_prev(state, filtered_names(names, filter, *sort, &favorites).len()),
                        KeyCode::Enter => chosen = state.selected().and_then(|i| filtered_names(names, filter, *sort, &favorites).get(i).map(|n| (*n).clone())),
                        _ => {}
                    }
                    if let Some(name) = chosen {
                        // Straight on to choosing its key; Esc from there goes
                        // back to the view this was opened from.
                        let back = std::mem::replace(&mut **back, Mode::List);
                        let mut next = key_picker(name, &favorites);
                        if let Mode::Slots { back: slot_back, .. } = &mut next {
                            *slot_back = Box::new(back);
                        }
                        mode = next;
                    } else if close {
                        mode = std::mem::replace(&mut **back, Mode::List);
                    }
                }
                (Event::Mouse(mouse), Mode::NamespacePick { names, filter, state, sort, back, .. }) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                    let matches = filtered_names(names, filter, *sort, &favorites);
                    if let Some(idx) = ui::event_row_at(frame_area, matches.len(), state.offset(), mouse.row) {
                        let name = matches[idx].clone();
                        let back = std::mem::replace(&mut **back, Mode::List);
                        let mut next = key_picker(name, &favorites);
                        if let Mode::Slots { back: slot_back, .. } = &mut next {
                            *slot_back = Box::new(back);
                        }
                        mode = next;
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
                            favorites.clear(*selected + 1);
                            favorites.save(active_context);
                        }
                        _ => {}
                    }
                    if let Some(key_number) = assign_to {
                        favorites.assign(key_number, namespace);
                        favorites.save(active_context);
                        close = true;
                    }
                    if close {
                        mode = std::mem::replace(&mut **back, Mode::List);
                    }
                }
                (Event::Key(_), Mode::Notice { back, .. }) => mode = std::mem::replace(&mut **back, Mode::List),
                (Event::Mouse(m), Mode::Notice { back, .. }) if matches!(m.kind, MouseEventKind::Down(_)) => {
                    mode = std::mem::replace(&mut **back, Mode::List)
                }
                // Toggling mouse reporting off hands click-drag text
                // selection (and therefore copy) back to the terminal
                // itself — the only thing enabling it took away. Skipped
                // while typing a command/search, where `c` is just a
                // character to type, not this toggle.
                (Event::Key(key), current_mode)
                    if key.code == KeyCode::Char('c') && !is_typing(current_mode) =>
                {
                    mouse_capture_enabled = !mouse_capture_enabled;
                    if mouse_capture_enabled {
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
                    show_hints_panel = !show_hints_panel;
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
                    if current_kind == ResourceKind::Overview {
                        let active_col = match overview_selection {
                            ui::OverviewSelection::Header(c) | ui::OverviewSelection::Item(c, _) => c,
                            ui::OverviewSelection::Resources | ui::OverviewSelection::Events => usize::MAX,
                        };
                        if matches!(mouse.kind, MouseEventKind::Down(_))
                            && let Some(hit) = ui::column_hit(
                                ui::body_area(frame_area, false),
                                &overview,
                                overview_col_scroll,
                                active_col,
                                overview_item_scroll,
                                mouse.column,
                                mouse.row,
                            )
                        {
                            overview_selection = hit;
                        }
                    } else {
                        hovered = ui::row_at(ui::body_area(frame_area, true), &pod_rows, hscroll, &table_state, row_count, mouse.column, mouse.row).map(|row| {
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
                            namespace = None;
                            table_state.select(Some(0));
                        } else if let Some(ns) = favorites.get(n) {
                            namespace = Some(ns.to_string());
                            table_state.select(Some(0));
                        }
                    }
                }
                (Event::Key(key), Mode::List) if current_kind == ResourceKind::Overview => {
                    let columns_area = ui::columns_area(ui::body_area(frame_area, false), &overview);
                    let cols_visible = ui::visible_columns(columns_area.width, overview.catalog.len());
                    match key.code {
                        KeyCode::Char('n') => {
                            let names: Vec<String> =
                                catalog.resolve(ResourceKind::Namespaces, &client).map(|k| k.rows()).unwrap_or_default().into_iter().map(|r| r.name).collect();
                            open_namespace_picker(&mut mode, names);
                        }
                        // Esc and `q` are no-ops here — there's nowhere
                        // further "back" than the main screen, and quitting
                        // takes a deliberate `:q` so a stray key can't do it.
                        KeyCode::Char('j') | KeyCode::Down => {
                            overview_selection = ui::move_overview_selection(&overview, overview_selection, ui::Direction::Down);
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            overview_selection = ui::move_overview_selection(&overview, overview_selection, ui::Direction::Up);
                        }
                        KeyCode::Char('h') | KeyCode::Left => {
                            overview_selection = ui::move_overview_selection(&overview, overview_selection, ui::Direction::Left);
                        }
                        KeyCode::Char('l') | KeyCode::Right => {
                            overview_selection = ui::move_overview_selection(&overview, overview_selection, ui::Direction::Right);
                        }
                        KeyCode::Char('m') => {
                            mode = Mode::Menu { selected: menu_position_for(current_kind, &catalog.crds) };
                        }
                        KeyCode::Enter => match overview_selection {
                            ui::OverviewSelection::Resources => {
                                mode = Mode::ResourcesDetail;
                            }
                            ui::OverviewSelection::Events => {
                                mode = Mode::Events {
                                    filter: k8s::EventFilter::default(),
                                    search: String::new(),
                                    editing: false,
                                    sort: ListSort::default(),
                                    state: TableState::default().with_selected(if overview.events.is_empty() { None } else { Some(0) }),
                                };
                            }
                            ui::OverviewSelection::Header(col) => {
                                mode = Mode::ColumnDetail { col, selected: 0, row_scroll: 0 };
                            }
                            ui::OverviewSelection::Item(col, item) => {
                                if let Some((_, items)) = overview.catalog.get(col)
                                    && let Some((label, _)) = items.get(item)
                                    && let Some(kind) = catalog.kind_for_tile_label(label)
                                {
                                    current_kind = kind;
                                    scope = None;
                                    nav_stack.clear();
                                    sort = None;
                                    hscroll = 0;
                                    table_state.select(Some(0));
                                    search.clear();
                                }
                            }
                        },
                        _ => {}
                    }
                    if let ui::OverviewSelection::Header(c) | ui::OverviewSelection::Item(c, _) = overview_selection {
                        overview_col_scroll = ui::scroll_columns_to_show(overview_col_scroll, cols_visible, c);
                        let target_item = match overview_selection {
                            ui::OverviewSelection::Item(_, i) => i,
                            _ => 0,
                        };
                        let items_visible = ui::visible_items_per_column(columns_area.height, ui::column_item_height(&overview, c));
                        overview_item_scroll = ui::scroll_columns_to_show(overview_item_scroll, items_visible, target_item);
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
                    KeyCode::Char('q') | KeyCode::Esc => mode = Mode::List,
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
                            mode = Mode::EventDetail { entry, back };
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
                        mode = Mode::EventDetail { entry, back };
                    }
                }
                (Event::Key(key), Mode::EventDetail { back, .. }) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => mode = std::mem::replace(&mut **back, Mode::List),
                    _ => {}
                },
                (Event::Key(key), Mode::ResourcesDetail) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => mode = Mode::List,
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
                        KeyCode::Char('q') | KeyCode::Esc => mode = Mode::List,
                        KeyCode::Char('j') | KeyCode::Down => move_and_rescroll!(ui::Direction::Down),
                        KeyCode::Char('k') | KeyCode::Up => move_and_rescroll!(ui::Direction::Up),
                        KeyCode::Char('h') | KeyCode::Left => move_and_rescroll!(ui::Direction::Left),
                        KeyCode::Char('l') | KeyCode::Right => move_and_rescroll!(ui::Direction::Right),
                        KeyCode::Enter => {
                            if let Some((_, items)) = overview.catalog.get(*col)
                                && let Some((label, _)) = items.get(*selected)
                                && let Some(kind) = catalog.kind_for_tile_label(label)
                            {
                                current_kind = kind;
                                scope = None;
                                nav_stack.clear();
                                sort = None;
                                hscroll = 0;
                                table_state.select(Some(0));
                                search.clear();
                                mode = Mode::List;
                            }
                        }
                        _ => {}
                    }
                }
                (Event::Key(key), Mode::List) => match if key.code == KeyCode::Enter && current_kind.opens_spec_on_enter() {
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
                    KeyCode::Char('q') | KeyCode::Esc if !nav_stack.is_empty() => {
                        // Back out of a drill-down to the list it came from.
                        if let Some((kind, previous_scope, selected)) = nav_stack.pop() {
                            current_kind = kind;
                            scope = previous_scope;
                            sort = None;
                            hscroll = 0;
                            table_state.select(Some(selected));
                        }
                        search.clear();
                    }
                    KeyCode::Char('q') | KeyCode::Esc => {
                        current_kind = match current_kind {
                            ResourceKind::CustomResource(index, _) => catalog
                                .crds
                                .get(index)
                                .map(|c| ResourceKind::CustomResourceGroup(c.group))
                                .unwrap_or(ResourceKind::CustomResourceList),
                            _ => ResourceKind::Overview,
                        };
                        table_state.select(Some(0));
                        search.clear();
                    }
                    // Enter drills into what a row owns or selects: a
                    // Deployment's ReplicaSets, a ReplicaSet's Pods, a
                    // Service's Pods, a CronJob's Jobs, a Namespace's Pods.
                    KeyCode::Enter if current_kind.drill_target().is_some() => {
                        let target = current_kind.drill_target().expect("guarded above");
                        let selected = table_state.selected().unwrap_or(0);
                        let new_scope = match current_kind {
                            ResourceKind::Deployments => deployments.get(selected).map(|d| Scope::Owner {
                                uid: d.metadata.uid.clone().unwrap_or_default(),
                                kind: "Deployment".into(),
                                name: d.metadata.name.clone().unwrap_or_default(),
                            }),
                            ResourceKind::Namespaces => generic_rows.get(selected).map(|r| Scope::Namespace { name: r.name.clone() }),
                            ResourceKind::Services => {
                                let manifest = generic_visible
                                    .get(selected)
                                    .and_then(|&real| catalog.resolve(current_kind, &client).and_then(|k| k.spec_at(real)));
                                generic_rows.get(selected).zip(manifest).map(|(row, manifest)| Scope::Selector {
                                    labels: service_selector(&manifest),
                                    kind: "Service".into(),
                                    name: row.name.clone(),
                                })
                            }
                            _ => generic_rows.get(selected).map(|row| Scope::Owner {
                                uid: row.uid.clone(),
                                kind: singular(current_kind),
                                name: row.name.clone(),
                            }),
                        };
                        if new_scope.is_some() {
                            nav_stack.push((current_kind, scope.take(), selected));
                            scope = new_scope;
                            sort = None;
                            hscroll = 0;
                            current_kind = target;
                            table_state.select(Some(0));
                            search.clear();
                        }
                    }
                    // ←/→ (or h/l) scroll a table sideways when its columns don't
                    // all fit; the title shows `‹ ›` for what's out of view.
                    KeyCode::Left | KeyCode::Char('h') => hscroll = hscroll.saturating_sub(1),
                    KeyCode::Right | KeyCode::Char('l') => hscroll += 1,
                    // `s` sorts: the column numbers in the header light up and the
                    // next digit picks one.
                    KeyCode::Char('s') if column_count(current_kind, generic_columns) > 0 => sort_choosing = true,
                    // `n` gives a namespace one of the number keys 1-9. On the
                    // Namespaces list it acts on the highlighted row right
                    // away; from every other view it first shows the
                    // namespaces to choose from.
                    KeyCode::Char('n') => {
                        if current_kind == ResourceKind::Namespaces {
                            if let Some(name) = table_state.selected().and_then(|i| generic_rows.get(i)).map(|r| r.name.clone()) {
                                mode = key_picker(name, &favorites);
                            }
                        } else {
                            let names: Vec<String> =
                                catalog.resolve(ResourceKind::Namespaces, &client).map(|k| k.rows()).unwrap_or_default().into_iter().map(|r| r.name).collect();
                            open_namespace_picker(&mut mode, names);
                        }
                    }
                    KeyCode::Char('j') | KeyCode::Down => select_next(&mut table_state, row_count),
                    KeyCode::Char('k') | KeyCode::Up => select_prev(&mut table_state, row_count),
                    KeyCode::Char('m') => {
                        mode = Mode::Menu { selected: menu_position_for(current_kind, &catalog.crds) };
                    }
                    KeyCode::Char('d') => match current_kind {
                        ResourceKind::Overview => unreachable!("handled in the Overview-specific arm above"),
                        ResourceKind::Pods => {
                            if let Some(pod) = table_state.selected().and_then(|i| pods.get(i)) {
                                open_spec(&mut mode, title_for(pod.metadata.namespace.as_deref(), pod.metadata.name.as_deref()), pod.as_ref());
                            }
                        }
                        ResourceKind::Deployments => {
                            if let Some(dep) = table_state.selected().and_then(|i| deployments.get(i)) {
                                open_spec(&mut mode, title_for(dep.metadata.namespace.as_deref(), dep.metadata.name.as_deref()), dep.as_ref());
                            }
                        }
                        // Indexes straight into the (already filtered)
                        // `sorted_nodes`, not through `generic_rows`/
                        // `catalog.resolve` — those re-snapshot unfiltered,
                        // which would misalign with what's actually
                        // displayed whenever a search is active.
                        ResourceKind::Nodes => {
                            if let Some(node) = table_state.selected().and_then(|i| sorted_nodes.get(i)) {
                                open_spec(&mut mode, node.metadata.name.clone().unwrap_or_default(), node.as_ref());
                            }
                        }
                        _ => {
                            // `table_state.selected()` is a position in the
                            // *filtered* display; `generic_visible` maps it
                            // back to `spec_at`'s real index.
                            if let Some(display_index) = table_state.selected()
                                && let Some(&real_index) = generic_visible.get(display_index)
                                && let Some(row) = generic_rows_full.get(real_index)
                                && let Some(value) = catalog.resolve(current_kind, &client).and_then(|k| k.spec_at(real_index))
                            {
                                let title = format!("{}/{}", row.namespace, row.name);
                                open_spec_value(&mut mode, title, value);
                            }
                        }
                    },
                    // Edit the selected resource in `$EDITOR` (see `edit`) —
                    // the same manifest `d` shows, for every kind that has
                    // a selectable row.
                    KeyCode::Char('e') => {
                        let manifest: Option<serde_yaml::Value> = match current_kind {
                            ResourceKind::Overview | ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => None,
                            ResourceKind::Pods => table_state.selected().and_then(|i| pods.get(i)).map(|p| k8s::manifest_value(p.as_ref())),
                            ResourceKind::Deployments => {
                                table_state.selected().and_then(|i| deployments.get(i)).map(|d| k8s::manifest_value(d.as_ref()))
                            }
                            ResourceKind::Nodes => table_state.selected().and_then(|i| sorted_nodes.get(i)).map(|n| k8s::manifest_value(n.as_ref())),
                            _ => table_state
                                .selected()
                                .and_then(|i| generic_visible.get(i).copied())
                                .and_then(|real| catalog.resolve(current_kind, &client).and_then(|k| k.spec_at(real))),
                        };
                        if let Some(manifest) = manifest {
                            let outcome = edit::edit_resource(terminal, &client, mouse_capture_enabled, &manifest);
                            mode = Mode::Notice { text: outcome.text, error: outcome.error, back: Box::new(Mode::List) };
                        }
                    }
                    KeyCode::Enter if matches!(current_kind, ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_)) => {
                        if let Some(index) = table_state.selected()
                            && let Some((real_index, crd)) = crd_rows.get(index)
                        {
                            current_kind = ResourceKind::CustomResource(*real_index, crd.kind);
                            scope = None;
                            nav_stack.clear();
                            sort = None;
                            hscroll = 0;
                            table_state.select(Some(0));
                            search.clear();
                        }
                    }
                    KeyCode::Enter if current_kind == ResourceKind::Pods => {
                        if let Some(pod) = table_state.selected().and_then(|i| pods.get(i)) {
                            let title = title_for(pod.metadata.namespace.as_deref(), pod.metadata.name.as_deref());
                            let namespace = pod.metadata.namespace.clone().unwrap_or_default();
                            let name = pod.metadata.name.clone().unwrap_or_default();
                            let containers = k8s::containers_for(pod);
                            mode = Mode::Containers {
                                title,
                                namespace,
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
                    KeyCode::Enter if current_kind == ResourceKind::Nodes => {
                        if let Some(node) = table_state.selected().and_then(|i| sorted_nodes.get(i)) {
                            let name = node.metadata.name.clone().unwrap_or_default();
                            mode = Mode::NodeDetail { name, state: TableState::default().with_selected(0), sort: ListSort::default(), search: String::new(), editing: false, back: Box::new(Mode::List) };
                        }
                    }
                    KeyCode::Char('/') | KeyCode::Char('f') => {
                        mode = Mode::Search;
                    }
                    _ => {}
                },
                (Event::Key(key), Mode::Command { input, selected, back }) => match key.code {
                    KeyCode::Esc => mode = std::mem::replace(&mut **back, Mode::List),
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
                            mode = Mode::Events { filter: k8s::EventFilter::All, search: String::new(), editing: false, state: TableState::default().with_selected(0), sort: ListSort::default() };
                        } else if is_context_command(&cmd) || matches!(highlighted, Some(Cmd::Context)) {
                            let mut opened = std::mem::replace(&mut **back, Mode::List);
                            open_context_switcher(&mut opened, active_context);
                            mode = opened;
                        } else if let Some(kind) = match highlighted {
                            Some(Cmd::Kind(k)) => Some(k),
                            _ => k8s::ResourceKind::from_command(&cmd),
                        } {
                            current_kind = kind;
                            scope = None;
                            nav_stack.clear();
                            sort = None;
                            hscroll = 0;
                            table_state.select(Some(0));
                            search.clear();
                            mode = Mode::List;
                        } else {
                            mode = std::mem::replace(&mut **back, Mode::List);
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
                        if let Mode::Context { editing, .. } = &mut mode {
                            *editing = false;
                        }
                    }
                    KeyCode::Enter => {
                        if let Mode::Context { editing, .. } = &mut mode {
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
                    KeyCode::Char('q') | KeyCode::Esc => mode = std::mem::replace(&mut **back, Mode::List),
                    KeyCode::Char('/') | KeyCode::Char('f') => *editing = true,
                    KeyCode::Char('j') | KeyCode::Down => select_next(state, filtered_contexts(contexts, filter, *sort).len()),
                    KeyCode::Char('k') | KeyCode::Up => select_prev(state, filtered_contexts(contexts, filter, *sort).len()),
                    KeyCode::Enter => {
                        let name = state.selected().and_then(|i| filtered_contexts(contexts, filter, *sort).get(i).map(|c| c.name.clone()));
                        if let Some(name) = name {
                            match switch_target(&name, active_context) {
                                Ok(true) => return Ok(Outcome::SwitchContext(name)),
                                Ok(false) => mode = std::mem::replace(&mut **back, Mode::List),
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
                            Ok(false) => mode = std::mem::replace(&mut **back, Mode::List),
                            Err(msg) => *error = Some(msg),
                        }
                    }
                }
                (Event::Key(key), Mode::Search) => match key.code {
                    KeyCode::Esc => {
                        search.clear();
                        mode = Mode::List;
                    }
                    KeyCode::Enter => mode = Mode::List,
                    KeyCode::Backspace => {
                        search.pop();
                    }
                    KeyCode::Char(c) => search.push(c),
                    _ => {}
                },
                (Event::Key(key), Mode::Menu { selected }) => {
                    let sections = menu_sections(&catalog.crds);
                    let cols = ui::menu_cols(frame_area);
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => mode = Mode::List,
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
                                current_kind = *kind;
                                scope = None;
                                nav_stack.clear();
                                sort = None;
                                hscroll = 0;
                                table_state.select(Some(0));
                                search.clear();
                                mode = Mode::List;
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
                    KeyCode::Char('q') | KeyCode::Esc => mode = std::mem::replace(&mut **back, Mode::List),
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
                        mode = std::mem::replace(&mut **back, Mode::List);
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
                            mode = Mode::Logs {
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
                    KeyCode::Char('q') | KeyCode::Esc => mode = std::mem::replace(&mut **back, Mode::List),
                    KeyCode::Char('d') => {
                        if let Some(node) = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str())) {
                            let title = name.clone();
                            open_spec(&mut mode, title, node.as_ref());
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
                            let outcome = edit::edit_resource(terminal, &client, mouse_capture_enabled, &manifest);
                            mode = Mode::Notice { text: outcome.text, error: outcome.error, back };
                        }
                    }
                    KeyCode::Char('j') | KeyCode::Down => select_next(state, node_detail_rows.len()),
                    KeyCode::Char('k') | KeyCode::Up => select_prev(state, node_detail_rows.len()),
                    KeyCode::Enter => {
                        if let Some(pod) = state.selected().and_then(|i| node_detail_pods.get(i)) {
                            let title = title_for(pod.metadata.namespace.as_deref(), pod.metadata.name.as_deref());
                            let namespace = pod.metadata.namespace.clone().unwrap_or_default();
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
                            mode = Mode::Containers {
                                title,
                                namespace,
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
                        mode = std::mem::replace(&mut **back, Mode::List);
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
