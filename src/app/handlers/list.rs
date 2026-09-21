//! Input on the main list (and the overview): sorting, namespaces, drill-down, opening details.

use super::super::*;
use super::{Cx, logs_mode, open_shell};
use crate::app::derive::Derived;

/// Handles one input event for these modes; `Some` ends the session.
pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let Derived { pods, pod_rows, deployments, sorted_nodes, overview, generic_rows_full, generic_visible, generic_columns, generic_rows, crd_rows, .. } = cx.d;
    let catalog = &mut *cx.catalog;
    let client = cx.client;
    let frame_area = cx.frame_area;
    let row_count = cx.row_count;
    let mut open = false;
    let mut to_owner = false;
    match (event, &mut st.mode) {
        // Ctrl combinations: `Ctrl-z` lists only rows that need a look,
        // `Ctrl-w` adds the wide columns. Any other Ctrl key does nothing
        // (rather than acting as its plain letter).
        (Event::Key(key), Mode::List) if key.modifiers.contains(KeyModifiers::CONTROL) => match key.code {
            KeyCode::Char('z') => {
                st.faults_only = !st.faults_only;
                st.table_state.select(Some(0));
            }
            KeyCode::Char('w') => {
                st.wide = !st.wide;
                st.hscroll = 0;
            }
            // Scroll the info panel.
            KeyCode::Char('d') if st.info_panel => st.info_scroll += 8,
            KeyCode::Char('u') if st.info_panel => st.info_scroll = st.info_scroll.saturating_sub(8),
            _ => {}
        },
        // Sort mode (`s`): headers show column numbers and a digit sorts by that column.
        // The same digit again flips ascending, descending, off. It stays on until
        // `s`, Esc or `q`; other keys work as usual.
        (Event::Key(key), Mode::List)
            if st.sort_choosing && matches!(key.code, KeyCode::Char('0'..='9' | 's' | 'q') | KeyCode::Esc) =>
        {
            match key.code {
                KeyCode::Char(c @ '0'..='9') => {
                    // 1-9 are columns 1-9; 0 is the tenth.
                    let column = (c as usize + 9 - '0' as usize) % 10;
                    if column < column_count(st.current_kind, *generic_columns, st.wide) {
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
                            return Ok(None);
                        }
                        MouseEventKind::ScrollUp if over_panel => {
                            st.info_scroll = st.info_scroll.saturating_sub(3);
                            return Ok(None);
                        }
                        MouseEventKind::Down(_) if over_panel => {
                            st.info_focus = true;
                            return Ok(None);
                        }
                        _ if over_panel => return Ok(None),
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
        }
        // The Port-forwards list: open one in the browser, stop one.
        (Event::Key(key), Mode::List) if st.current_kind == ResourceKind::PortForwards && matches!(key.code, KeyCode::Enter | KeyCode::Char('o' | 'D' | 'x') | KeyCode::Delete) => {
            let real = st.table_state.selected().and_then(|i| generic_visible.get(i).copied()).filter(|&i| i < st.forwards.len());
            if let Some(real) = real {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char('o')) {
                    if let Err(e) = portforward::open_in_browser(&st.forwards[real].url()) {
                        st.mode = Mode::Notice { text: format!("{e:#}"), error: true, back: Box::new(Mode::List) };
                    }
                } else {
                    st.forwards.remove(real);
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
                KeyCode::Char('m') => {
                    st.mode = Mode::Menu { selected: menu_position_for(st.current_kind, &catalog.crds) };
                }
                KeyCode::Char('T') => super::themes::open(st, cx.config),
                KeyCode::Char(',') => super::settings::open(st),
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
            // `q` and Esc go back one level everywhere except the Overview: to the Overview,
            // or to the CRD group a custom resource came from. `:q` quits.
            KeyCode::Esc if !st.marked.is_empty() => st.marked.clear(),
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
                    ResourceKind::Api(..) => ResourceKind::ApiResources,
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
                            namespace: (!row.namespace.is_empty()).then(|| row.namespace.clone()),
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
            KeyCode::Left => st.hscroll = st.hscroll.saturating_sub(1),
            KeyCode::Right => st.hscroll += 1,
            // `s` sorts: the column numbers in the header light up and the
            // next digit picks one.
            KeyCode::Char('s') if column_count(st.current_kind, *generic_columns, st.wide) > 0 => st.sort_choosing = true,
            // `n` gives a namespace one of the keys 1-9. On the Namespaces list it acts on
            // the highlighted row; elsewhere it shows the namespaces to choose from.
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
                // Indexes into the filtered `sorted_nodes`, not `generic_rows`, which
                // re-snapshot unfiltered and would misalign while a search is active.
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
            // Edit the selected resource in `$EDITOR` (see `edit`),
            // the same manifest `d` shows, for every kind that has
            // a selectable row.
            KeyCode::Char('e') => {
                if let Some(manifest) = selected_manifest(st, cx.d, catalog, client) {
                    let outcome = edit::edit_resource(cx.terminal, &client, &manifest);
                    st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back: Box::new(Mode::List) };
                }
            }
            // Actions on the selected object (see `actions`).
            // Forward a port of a pod, service or deployment.
            KeyCode::Char('F') => {
                if let Some(target) = selected_manifest(st, cx.d, catalog, client).as_ref().and_then(Target::from_manifest)
                    && target.forward_resource().is_some()
                {
                    let form = portforward::PortForm::new(target.ports());
                    st.mode = Mode::Ports { target, form, back: Box::new(Mode::List) };
                }
            }
            // The theme picker, and the settings.
            KeyCode::Char('T') => super::themes::open(st, cx.config),
            KeyCode::Char(',') => super::settings::open(st),
            // History: back, forward, and the view before this one.
            KeyCode::Char('[') => st.history_back(),
            KeyCode::Char(']') => st.history_forward(),
            KeyCode::Char('-') => st.toggle_last_view(),
            // Jump to what owns the selected object (a pod's ReplicaSet, a
            // ReplicaSet's Deployment); Esc comes back.
            KeyCode::Char('O') => {
                let owner = selected_manifest(st, cx.d, catalog, client).and_then(|m| {
                    let first = m.get("metadata")?.get("ownerReferences")?.as_sequence()?.first()?.clone();
                    Some((first.get("kind")?.as_str()?.to_string(), first.get("name")?.as_str()?.to_string()))
                });
                match owner {
                    Some((kind, name)) => match ResourceKind::from_owner_kind(&kind) {
                        Some(target) => {
                            let namespace = selected_manifest(st, cx.d, catalog, client).and_then(|m| m.get("metadata")?.get("namespace")?.as_str().map(String::from));
                            st.jump_to_object(target, namespace.as_deref(), &name);
                        }
                        None => st.mode = Mode::Notice { text: format!("Owned by a {kind} ({name}), which has no list here"), error: false, back: Box::new(Mode::List) },
                    },
                    None => st.mode = Mode::Notice { text: "No owner".into(), error: false, back: Box::new(Mode::List) },
                }
            }
            // The manifest as plain YAML text.
            KeyCode::Char('y') => {
                if let Some(manifest) = selected_manifest(st, cx.d, catalog, client)
                    && let Some(target) = Target::from_manifest(&manifest)
                {
                    let title = format!("{}/{}", target.namespace.as_deref().unwrap_or("-"), target.name);
                    let text = serde_yaml::to_string(&manifest).unwrap_or_default();
                    let back = std::mem::replace(&mut st.mode, Mode::List);
                    st.mode = Mode::Yaml { title, text, scroll: 0, back: Box::new(back) };
                }
            }
            // A readable summary of the selected object.
            KeyCode::Char('i') if frame_area.width >= ui::SIDE_PANEL_MIN_WIDTH => {
                st.info_panel = !st.info_panel;
                st.info_focus = false;
                st.info_scroll = 0;
            }
            KeyCode::Char('i') => {
                if let Some(manifest) = selected_manifest(st, cx.d, catalog, client) {
                    let sections = k8s::details::details(&manifest, &cx.d.overview.events);
                    let back = std::mem::replace(&mut st.mode, Mode::List);
                    st.mode = Mode::Details { manifest, sections, scroll: 0, hscroll: 0, back: Box::new(back) };
                }
            }
            // What the selected object relates to.
            KeyCode::Char('R') => {
                if let Some(manifest) = selected_manifest(st, cx.d, catalog, client) {
                    let all = surrounding_manifests(cx.pod_store, cx.dep_store, catalog, &manifest);
                    let graph = k8s::relations::graph(&manifest, &k8s::relations::relations(&manifest, &all));
                    let back = std::mem::replace(&mut st.mode, Mode::List);
                    st.mode = Mode::Relations { target: manifest, all, graph, selected: 0, previous: Vec::new(), back: Box::new(back) };
                }
            }
            // Copy the row's name (`namespace/name`) to the clipboard.
            KeyCode::Char('Y') => {
                if let Some(target) = selected_manifest(st, cx.d, catalog, client).as_ref().and_then(Target::from_manifest) {
                    let name = target.namespace.as_deref().map(|ns| format!("{ns}/{}", target.name)).unwrap_or_else(|| target.name.clone());
                    let outcome = match clipboard::copy(&name) {
                        Ok(how) => actions::Outcome { text: format!("Copied {name} with {how}"), error: false },
                        Err(e) => actions::Outcome { text: format!("{e:#}"), error: true },
                    };
                    st.mode = Mode::Notice { text: outcome.text, error: outcome.error, back: Box::new(Mode::List) };
                }
            }
            // A Secret's values, decoded.
            KeyCode::Char('x') if st.current_kind == ResourceKind::Secrets => {
                if let Some(manifest) = selected_manifest(st, cx.d, catalog, client)
                    && let Some(target) = Target::from_manifest(&manifest)
                {
                    let title = format!("{}/{} (decoded)", target.namespace.as_deref().unwrap_or("-"), target.name);
                    open_spec_value(&mut st.mode, title, actions::decode_secret(&manifest));
                }
            }
            // Logs of a pod's container (`p`: the previous run's).
            KeyCode::Char(c @ ('l' | 'p')) if st.current_kind == ResourceKind::Pods => {
                if let Some(target) = selected_manifest(st, cx.d, catalog, client).as_ref().and_then(Target::from_manifest) {
                    open_pod(st, cx, &target, PodView::Logs { previous: c == 'p' });
                }
            }
            KeyCode::Char(c @ ('D' | 'S' | 'r' | 'c' | 'u' | 't')) => {
                // Delete, restart and scale act on every marked row when
                // there are marks; everything else on the cursor row.
                let bulk = matches!(c, 'D' | 'S' | 'r') && !st.marked.is_empty();
                let targets: Vec<Target> = if bulk {
                    marked_targets(st, cx.d, catalog, client)
                } else {
                    selected_manifest(st, cx.d, catalog, client).as_ref().and_then(Target::from_manifest).into_iter().collect()
                };
                // An API resource is a kind, not an object: nothing here acts on it.
                if let Some(target) = targets.first().cloned().filter(|t| t.kind != "APIResource") {
                    let action = match c {
                        'D' => Some(Action::Delete),
                        'r' if target.restartable() => Some(Action::Restart),
                        'c' => target.cordon_action(),
                        'u' => target.suspend_action(),
                        't' if target.kind == "CronJob" => Some(Action::Trigger),
                        _ => None,
                    };
                    // `S` scales what scales and opens a shell in a pod.
                    if c == 'S' && target.scalable() {
                        st.mode = Mode::Scale { input: target.replicas().to_string(), targets, back: Box::new(Mode::List) };
                    } else if c == 'S' && target.kind == "Pod" && !bulk {
                        open_pod(st, cx, &target, PodView::Shell);
                    } else if let Some(action) = action {
                        if let Some(spec) = actions::confirm_spec(action, &targets) {
                            st.mode = Mode::Confirm { spec, targets, action, back: Box::new(Mode::List) };
                        } else {
                            crate::app::jobs::run_action(st, client, targets, action, Box::new(Mode::List));
                        }
                    }
                }
            }
            // Space marks the row (and moves down) for bulk actions.
            KeyCode::Char(' ') => {
                if let Some(manifest) = selected_manifest(st, cx.d, catalog, client)
                    && let Some(target) = Target::from_manifest(&manifest)
                {
                    let key = ui::mark_key(target.namespace.as_deref().unwrap_or("-"), &target.name);
                    if !st.marked.remove(&key) {
                        st.marked.insert(key);
                    }
                    select_next(&mut st.table_state, row_count);
                }
            }
            // Open the resource type under the cursor as a list of its own.
            KeyCode::Enter if st.current_kind == ResourceKind::ApiResources => {
                let api = st.table_state.selected().and_then(|i| generic_visible.get(i).copied()).and_then(|real| catalog.apis.get(real).map(|a| (real, a.plural)));
                if let Some((index, plural)) = api {
                    st.switch_kind(ResourceKind::Api(index, plural));
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
    if to_owner {
        return handle(Event::Key(KeyCode::Char('O').into()), st, cx);
    }
    Ok(None)
}

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
pub(crate) fn selected_manifest(st: &State, d: &Derived, catalog: &mut Catalog, client: &Client) -> Option<serde_yaml::Value> {
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

/// The manifests around `target` (its namespace, plus everything cluster-wide it may
/// point at), with ConfigMap and Secret payloads dropped.
fn surrounding_manifests(pod_store: &Store<Pod>, dep_store: &Store<Deployment>, catalog: &mut Catalog, target: &serde_yaml::Value) -> Vec<serde_yaml::Value> {
    use kube::ResourceExt;
    let kind = target.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    let namespace = target.get("metadata").and_then(|m| m.get("namespace")).and_then(|n| n.as_str()).map(String::from);
    // A cluster-scoped target (a Node, a PV) can be used from any namespace.
    let filter = if matches!(kind, "Node" | "PersistentVolume" | "StorageClass") { None } else { namespace.as_deref() };
    let mut all: Vec<serde_yaml::Value> = Vec::new();
    all.extend(k8s::snapshot(pod_store).iter().filter(|p| filter.is_none() || p.namespace().as_deref() == filter).map(|p| k8s::manifest_value(p.as_ref())));
    all.extend(k8s::snapshot_generic(dep_store).iter().filter(|d| filter.is_none() || d.namespace().as_deref() == filter).map(|d| k8s::manifest_value(d.as_ref())));
    for kind in [
        ResourceKind::Nodes,
        ResourceKind::ReplicaSets,
        ResourceKind::StatefulSets,
        ResourceKind::DaemonSets,
        ResourceKind::Jobs,
        ResourceKind::CronJobs,
        ResourceKind::ConfigMaps,
        ResourceKind::Secrets,
        ResourceKind::Hpas,
        ResourceKind::Services,
        ResourceKind::Ingresses,
        ResourceKind::Pvcs,
        ResourceKind::Pvs,
        ResourceKind::StorageClasses,
        ResourceKind::ServiceAccounts,
    ] {
        if let Some(k) = catalog.get(kind) {
            all.extend(k.manifests(filter));
        }
    }
    all.into_iter().map(k8s::relations::slim).collect()
}

/// What to do with a pod's container.
enum PodView {
    Shell,
    Logs { previous: bool },
}

/// Opens a shell or the logs in a pod's container: straight away when the
/// pod has just one, else the container list to choose from.
fn open_pod(st: &mut State, cx: &mut Cx, target: &Target, view: PodView) {
    let Ok(pod) = serde_yaml::from_value::<k8s_openapi::api::core::v1::Pod>(target.manifest.clone()) else { return };
    let containers = k8s::containers_for(&pod);
    let namespace = target.namespace.clone().unwrap_or_default();
    // The previous run only exists for a container that restarted.
    if let PodView::Logs { previous: true } = view {
        let restarted: Vec<&k8s::ContainerInfo> = containers.iter().filter(|c| c.restarts > 0).collect();
        match restarted.as_slice() {
            [] => {
                st.mode = Mode::Notice { text: format!("{} has not restarted, so there is no previous run to show", target.name), error: false, back: Box::new(Mode::List) };
                return;
            }
            [one] => {
                let name = one.name.clone();
                st.mode = logs_mode(cx, &namespace, &target.name, &name, true, Mode::List);
                return;
            }
            _ => {}
        }
    }
    match (containers.as_slice(), view) {
        ([only], PodView::Shell) => {
            let name = only.name.clone();
            open_shell(st, cx, &namespace, &target.name, &name);
        }
        ([only], PodView::Logs { previous }) => {
            st.mode = logs_mode(cx, &namespace, &target.name, &only.name, previous, Mode::List);
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

/// Every row of the current list, in order, with its manifest.
fn visible_manifests(st: &State, d: &Derived, catalog: &mut Catalog, client: &Client) -> Vec<serde_yaml::Value> {
    match st.current_kind {
        ResourceKind::Overview | ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => Vec::new(),
        ResourceKind::Pods => d.pods.iter().map(|p| k8s::manifest_value(p.as_ref())).collect(),
        ResourceKind::Deployments => d.deployments.iter().map(|x| k8s::manifest_value(x.as_ref())).collect(),
        ResourceKind::Nodes => d.sorted_nodes.iter().map(|n| k8s::manifest_value(n.as_ref())).collect(),
        kind => match catalog.resolve(kind, client) {
            Some(k) => d.generic_visible.iter().filter_map(|&real| k.spec_at(real)).collect(),
            None => Vec::new(),
        },
    }
}

/// The marked rows of the current list as action targets.
fn marked_targets(st: &State, d: &Derived, catalog: &mut Catalog, client: &Client) -> Vec<Target> {
    visible_manifests(st, d, catalog, client)
        .iter()
        .filter_map(Target::from_manifest)
        .filter(|t| st.marked.contains(&ui::mark_key(t.namespace.as_deref().unwrap_or("-"), &t.name)))
        .collect()
}
