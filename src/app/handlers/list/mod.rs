//! Input on the main list (and the overview): sorting, namespaces, drill-down, opening details.

use crate::ops::NoticeTone;
use super::super::*;
use super::{Cx, logs_mode, open_shell};
use crate::app::derive::Derived;

mod dashboard;
mod mouse;
mod overview;
mod selection;

pub(crate) use selection::selected_manifest;
use selection::{PodView, RELATED_KINDS, keep_overview_selection_visible, marked_targets, open_pod, surrounding_manifests};

/// Handles one input event for these modes; `Some` ends the session.
pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let Derived { pods, deployments, sorted_nodes, generic_rows_full, generic_visible, generic_columns, generic_rows, crd_rows, .. } = cx.d;
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
        // Sort mode (`s`): headers show column numbers 0-9 and a digit sorts by that column; the
        // arrows move a cursor along the headers so any column can be picked, and Enter sorts by it.
        // The same column again flips ascending, descending, off. It stays on until `s`, Esc or `q`.
        (Event::Key(key), Mode::List)
            if st.sort_choosing && matches!(key.code, KeyCode::Char('0'..='9' | 's' | 'q' | 'h' | 'l') | KeyCode::Esc | KeyCode::Left | KeyCode::Right | KeyCode::Enter) =>
        {
            let columns = column_count(st.current_kind, *generic_columns, st.wide);
            match key.code {
                KeyCode::Char(c @ '0'..='9') => {
                    let column = c as usize - '0' as usize;
                    if column < columns {
                        st.sort_cursor = column;
                        st.sort = Some(SortSpec::pressed(st.sort, column));
                        st.table_state.select(Some(0));
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => st.sort_cursor = st.sort_cursor.saturating_sub(1),
                KeyCode::Right | KeyCode::Char('l') => st.sort_cursor = (st.sort_cursor + 1).min(columns.saturating_sub(1)),
                KeyCode::Enter => {
                    if st.sort_cursor < columns {
                        st.sort = Some(SortSpec::pressed(st.sort, st.sort_cursor));
                        st.table_state.select(Some(0));
                    }
                }
                _ => st.sort_choosing = false,
            }
        }
        (Event::Mouse(mouse), Mode::List) => (open, to_owner) = mouse::handle(mouse, st, cx),
        // The Port-forwards list: open one in the browser, stop one.
        (Event::Key(key), Mode::List) if st.current_kind == ResourceKind::PortForwards && matches!(key.code, KeyCode::Enter | KeyCode::Char('o' | 'D' | 'x') | KeyCode::Delete) => {
            let real = st.table_state.selected().and_then(|i| generic_visible.get(i).copied()).filter(|&i| i < st.forwards.len());
            if let Some(real) = real {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char('o')) {
                    if let Err(e) = portforward::open_in_browser(&st.forwards[real].url()) {
                        st.mode = Mode::Notice { text: format!("{e:#}"), tone: NoticeTone::Failed, back: Box::new(Mode::List) };
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
        (Event::Key(key), Mode::List) if st.current_kind == ResourceKind::Overview => overview::keys(key, st, cx),
        (Event::Key(key), Mode::List)
            if matches!(st.current_kind, ResourceKind::ExtensionDashboard(_))
                && matches!(key.code, KeyCode::Char('j' | 'k' | 'g' | 'G') | KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown) =>
        {
            dashboard::keys(key, st);
        }
        (Event::Key(key), Mode::List) => match if key.code == KeyCode::Enter && st.current_kind.opens_spec_on_enter() {
            KeyCode::Char('d')
        } else {
            key.code
        } {
            // `q` and Esc undo one step: back out of a drill-down, or return to whatever
            // mode a jump into this list (Relations' `o`, Details' Enter) left behind,
            // in the order these actually happened. Once there is nothing left: to the
            // Overview, always — browsing into a CRD kind's instances (from CustomResources)
            // or one discovered type's Table view (from API Resources) both push a step
            // (see the `Enter` arms below), so an empty stack here only ever means this
            // kind was jumped to directly (an Overview tile, `:kind`), never that its
            // picker is where "back" belongs. `:q` quits.
            KeyCode::Esc if !st.marked.is_empty() => st.marked.clear(),
            KeyCode::Char('q') | KeyCode::Esc if !st.back_stack.is_empty() => match st.back_stack.pop() {
                Some(Step::List(kind, previous_scope, selected)) => {
                    st.current_kind = kind;
                    st.scope = previous_scope;
                    st.sort = None;
                    st.hscroll = 0;
                    st.table_state.select(Some(selected));
                    st.search.clear();
                }
                Some(Step::Mode(mode, snap)) => {
                    st.restore_list(snap);
                    st.mode = *mode;
                }
                None => unreachable!("just checked back_stack is not empty"),
            },
            KeyCode::Char('q') | KeyCode::Esc => {
                st.current_kind = ResourceKind::Overview;
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
                    st.back_stack.push(Step::List(st.current_kind, st.scope.take(), selected));
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
            KeyCode::Char('s') if column_count(st.current_kind, *generic_columns, st.wide) > 0 => {
                st.sort_choosing = true;
                st.sort_cursor = st.sort.map_or(0, |s| s.column);
            }
            // `A` sorts by age, like k9s; again flips the direction, then clears it.
            KeyCode::Char('A') => {
                if let Some(column) = age_column(st.current_kind, *generic_columns, st.wide) {
                    st.sort = Some(SortSpec::pressed(st.sort, column));
                    st.table_state.select(Some(0));
                }
            }
            // `n` gives a namespace one of the keys 1-9. On the Namespaces list it acts on
            // the highlighted row; elsewhere it shows the namespaces to choose from.
            KeyCode::Char('n') => {
                if st.current_kind == ResourceKind::Namespaces {
                    if let Some(name) = st.table_state.selected().and_then(|i| generic_rows.get(i)).map(|r| r.name.clone()) {
                        st.mode = key_picker(name, &st.favorites);
                    }
                } else {
                    let names: Vec<String> =
                        catalog.resolve(ResourceKind::Namespaces, &client).map(|k| k.rows()).unwrap_or_default().into_iter().map(|r| r.name.clone()).collect();
                    open_namespace_picker(&mut st.mode, names);
                }
            }
            KeyCode::Char('j') | KeyCode::Down => select_next(&mut st.table_state, row_count),
            KeyCode::Char('k') | KeyCode::Up => select_prev(&mut st.table_state, row_count),
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
                    st.mode = Mode::Notice { text: outcome.text, tone: outcome.tone, back: Box::new(Mode::List) };
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
                        None => st.mode = Mode::Notice { text: format!("Owned by a {kind} ({name}), which has no list here"), tone: NoticeTone::Info, back: Box::new(Mode::List) },
                    },
                    None => {
                        let name = selected_manifest(st, cx.d, catalog, client).and_then(|m| m.get("metadata")?.get("name")?.as_str().map(String::from));
                        let text = name.map_or_else(|| "It has no owner".to_string(), |n| format!("{n} has no owner"));
                        st.mode = Mode::Notice { text, tone: NoticeTone::Info, back: Box::new(Mode::List) };
                    }
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
                    st.reveal = true;
                    let view = catalog.view_for(&cx.config.extensions.enabled, &manifest);
                    let sections = k8s::details::details(&manifest, &cx.d.overview.events, true, view);
                    let back = std::mem::replace(&mut st.mode, Mode::List);
                    st.mode = Mode::Details { manifest, sections, scroll: 0, hscroll: 0, back: Box::new(back) };
                }
            }
            // What the selected object relates to.
            KeyCode::Char('R') => {
                for kind in RELATED_KINDS {
                    catalog.ensure(kind);
                }
                // The first time, the kinds are still loading: wait for them in the background, then press R again.
                if !catalog.all_ready(&RELATED_KINDS) {
                    let waits: Vec<_> = RELATED_KINDS.iter().filter_map(|k| catalog.get(*k)).map(|k| k.wait_ready()).collect();
                    crate::app::jobs::wait_then_replay(st, "Loading related objects", waits, key);
                } else if let Some(manifest) = selected_manifest(st, cx.d, catalog, client) {
                    let all = surrounding_manifests(cx.pod_store, cx.dep_store, catalog, &manifest);
                    let graph = k8s::relations::graph(&manifest, &k8s::relations::relations(&manifest, &all));
                    let back = std::mem::replace(&mut st.mode, Mode::List);
                    st.mode = Mode::Relations { target: manifest, all, graph, selected: 0, previous: Vec::new(), zoom: ui::DEFAULT_ZOOM, back: Box::new(back) };
                }
            }
            // Copy the row's name (`namespace/name`) to the clipboard.
            KeyCode::Char('Y') => {
                if let Some(target) = selected_manifest(st, cx.d, catalog, client).as_ref().and_then(Target::from_manifest) {
                    let name = target.namespace.as_deref().map(|ns| format!("{ns}/{}", target.name)).unwrap_or_else(|| target.name.clone());
                    let outcome = match clipboard::copy(&name) {
                        Ok(how) => crate::ops::Outcome { text: format!("Copied {name}{}", clipboard::how_note(how)), tone: NoticeTone::Done },
                        Err(e) => crate::ops::Outcome { text: format!("{e:#}"), tone: NoticeTone::Failed },
                    };
                    st.mode = Mode::Notice { text: outcome.text, tone: outcome.tone, back: Box::new(Mode::List) };
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
            // Every pod behind the selected workload (or, on a Pod itself, its
            // siblings), tailed as one merged stream.
            KeyCode::Char('L') if matches!(st.current_kind, ResourceKind::Pods | ResourceKind::Deployments | ResourceKind::ReplicaSets | ResourceKind::StatefulSets | ResourceKind::DaemonSets | ResourceKind::Jobs) => {
                for kind in RELATED_KINDS {
                    catalog.ensure(kind);
                }
                // The first time, the kinds are still loading (the ReplicaSet needs to be
                // in hand to walk from a Pod up to its Deployment): wait, then press L again.
                if !catalog.all_ready(&RELATED_KINDS) {
                    let waits: Vec<_> = RELATED_KINDS.iter().filter_map(|k| catalog.get(*k)).map(|k| k.wait_ready()).collect();
                    crate::app::jobs::wait_then_replay(st, "Loading the workload's pods", waits, key);
                } else if let Some(manifest) = selected_manifest(st, cx.d, catalog, client) {
                    let all = surrounding_manifests(cx.pod_store, cx.dep_store, catalog, &manifest);
                    let (pods, _owner_kind, owner_name) = k8s::relations::sibling_pods(&manifest, &all);
                    let namespace = manifest.get("metadata").and_then(|m| m.get("namespace")).and_then(|n| n.as_str()).unwrap_or_default().to_string();
                    let mut targets: Vec<(String, String, String)> = Vec::new();
                    for pod_value in &pods {
                        let Ok(pod) = serde_yaml::from_value::<k8s_openapi::api::core::v1::Pod>(pod_value.clone()) else { continue };
                        let name = pod.metadata.name.clone().unwrap_or_default();
                        let containers = k8s::containers_for(&pod);
                        let single = containers.len() <= 1;
                        for c in containers {
                            let tag = if single { name.clone() } else { format!("{name}/{}", c.name) };
                            targets.push((name.clone(), c.name, tag));
                        }
                    }
                    if targets.is_empty() {
                        // A ReplicaSet scaled to 0, a completed Job, ... there's a manifest but
                        // no live pods behind it right now; say so instead of doing nothing silently.
                        st.mode = Mode::Notice { text: format!("{namespace}/{owner_name} has no running pods right now"), tone: NoticeTone::Info, back: Box::new(Mode::List) };
                    } else {
                        let title = format!("{namespace}/{owner_name} ({} pod{})", pods.len(), if pods.len() == 1 { "" } else { "s" });
                        let (rx, handles) = k8s::stream_logs_many(client.clone(), namespace, targets);
                        let back = std::mem::replace(&mut st.mode, Mode::List);
                        st.mode = Mode::Logs {
                            title,
                            lines: Vec::new(),
                            scroll: 0,
                            follow: true,
                            timestamp_format: cx.config.logs.timestamp_format,
                            order: cx.config.logs.order,
                            rx,
                            handles: handles.into_iter().map(crate::app::mode::AbortOnDrop).collect(),
                            filter: String::new(),
                            filter_editing: false,
                            back: Box::new(back),
                        };
                    }
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
            // Open the resource type under the cursor as a list of its own, remembering
            // this list so q/Esc returns to it (not just to whichever list is hardcoded
            // as "usually" the one before it).
            KeyCode::Enter if st.current_kind == ResourceKind::ApiResources => {
                let api = st.table_state.selected().and_then(|i| generic_visible.get(i).copied()).and_then(|real| catalog.apis.get(real).map(|a| (real, a.plural)));
                if let Some((index, plural)) = api {
                    let selected = st.table_state.selected().unwrap_or(0);
                    st.back_stack.push(Step::List(st.current_kind, st.scope.take(), selected));
                    st.current_kind = ResourceKind::Api(index, plural);
                    st.sort = None;
                    st.hscroll = 0;
                    st.table_state.select(Some(0));
                    st.search.clear();
                }
            }
            KeyCode::Enter if matches!(st.current_kind, ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_)) => {
                if let Some(index) = st.table_state.selected()
                    && let Some((real_index, crd)) = crd_rows.get(index)
                {
                    st.back_stack.push(Step::List(st.current_kind, st.scope.take(), index));
                    st.current_kind = ResourceKind::CustomResource(*real_index, crd.kind);
                    st.sort = None;
                    st.hscroll = 0;
                    st.table_state.select(Some(0));
                    st.search.clear();
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
