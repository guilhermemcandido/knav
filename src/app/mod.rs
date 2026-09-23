//! The interactive loop: draws the current `Mode` and handles keyboard/mouse input.

pub(crate) mod boot;
mod derive;
pub mod commands;
mod draw;
mod handlers;
pub(crate) mod jobs;
mod hints;
pub mod mode;
mod nav;
mod path;
mod sidebar;
mod state;

use handlers::Cx;
use state::{State, Step};

use super::*;

/// The most log lines held for one stream.
const MAX_LOG_LINES: usize = 100_000;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    terminal: &mut ratatui::DefaultTerminal,
    pod_store: &k8s::PodKept,
    dep_store: &k8s::DeploymentKept,
    node_store: &Store<Node>,
    event_store: &Store<k8s_openapi::api::core::v1::Event>,
    node_metrics_rx: &watch::Receiver<Option<metrics::ClusterUsage>>,
    catalog: &mut Catalog,
    client: Client,
    config: &Config,
    active_context: &str,
    header: &ui::HeaderInfo,
    notes: Vec<String>,
) -> Result<Outcome> {
    let mut st = State::new(icons::IconCache::detect(), Favorites::load(active_context), config.clone());
    if !notes.is_empty() {
        st.mode = Mode::Notice { text: format!("Problems with your settings:\n{}", notes.join("\n")), error: true, back: Box::new(Mode::List) };
    }

    let mut cache: Option<derive::Cache> = None;
    loop {
        if let Some(outcome) = jobs::finish(&mut st) {
            return Ok(outcome);
        }
        st.record_view();
        // A forward that kubectl dropped (the pod went away) leaves the list.
        st.forwards.retain_mut(|f| f.alive());
        let forward_rows: Vec<k8s::GenericRow> = st.forwards.iter().map(|f| f.row()).collect();
        let src = derive::Sources { pod_store, dep_store, node_store, event_store, node_metrics_rx, client: &client, forwards: &forward_rows };
        let query = derive::Query {
            current_kind: st.current_kind,
            namespace: st.namespace.as_deref(),
            scope: st.scope.as_ref(),
            search: &st.search,
            sort: st.sort,
            faults: st.faults_only,
            wide: st.wide,
            layout: &st.config.overview,
            extensions_enabled: &st.config.extensions.enabled,
        };
        let fresh = derive::Cache::take_or_derive(cache.take(), &src, catalog, &st.mode, &query);
        let derived = fresh.derived();
        let derive::Derived { pod_rows, dep_rows, nodes, usage, node_detail_rows, node_rows, overview, generic_headers, generic_rows, crd_rows, crd_counts, .. } = derived;

        let row_count = match st.current_kind {
            ResourceKind::Overview => overview.events.len(),
            ResourceKind::Pods => pod_rows.len(),
            ResourceKind::Deployments => dep_rows.len(),
            ResourceKind::Nodes => node_rows.len(),
            ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => crd_rows.len(),
            _ => generic_rows.len(),
        };
        // Only the object counts of the types on screen (and one screen further) are fetched,
        // plus whatever an enabled extension put on the Overview (a small, fixed set, unlike
        // a whole picker's worth of types, so it's always worth asking for).
        let mut wanted_counts = catalog.want_extension_counts(&st.config.extensions.enabled);
        if matches!(st.current_kind, ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) | ResourceKind::ApiResources) {
            let reach = usize::from(terminal.size().map(|s| s.height).unwrap_or(40)) * 2;
            let from = st.table_state.offset();
            if st.current_kind == ResourceKind::ApiResources {
                wanted_counts.extend(generic_rows.iter().skip(from).take(reach).map(|r| k8s::count_key(r.extras.first().map_or("", |g| if g.text == "core" { "" } else { g.text.as_str() }), &r.name)));
            } else {
                wanted_counts.extend(crd_rows.iter().skip(from).take(reach).map(|(_, c)| k8s::count_key(c.group, &c.plural)));
            }
        }
        catalog.want_counts(wanted_counts);
        // Selection can't outrun the list as rows come and go. The Overview has no
        // selectable row, so this only matters for lists.
        if st.current_kind != ResourceKind::Overview && row_count > 0 {
            let clamped = st.table_state.selected().unwrap_or(0).min(row_count - 1);
            st.table_state.select(Some(clamped));
        }

        // Logs keep arriving in the background regardless of what key was
        // last pressed, drain whatever's ready before every redraw.
        if let Mode::Logs { lines, rx, .. } = &mut st.mode {
            while let Ok(line) = rx.try_recv() {
                lines.push(line);
            }
            // Keep a long follow from growing without end.
            if lines.len() > MAX_LOG_LINES {
                lines.drain(..lines.len() - MAX_LOG_LINES);
            }
        }

        let rows_view = || match st.current_kind {
            ResourceKind::Overview => ui::Rows::Overview(&overview, st.overview_selection, st.overview_col_scroll, st.overview_item_scroll),
            ResourceKind::Pods => ui::Rows::Pods(&pod_rows),
            ResourceKind::Deployments => ui::Rows::Deployments(&dep_rows),
            ResourceKind::Nodes => ui::Rows::Nodes(&node_rows),
            ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => ui::Rows::CrdList(&crd_rows, crd_counts, st.current_kind.label()),
            _ => ui::Rows::Generic(&generic_rows, st.current_kind.label(), &generic_headers),
        };

        let header_now = ui::HeaderInfo {
            namespace: st.namespace.clone().unwrap_or_else(|| "all".into()),
            scope: st.scope.as_ref().map(Scope::label).unwrap_or_default(),
            namespace_slots: st.favorites.slots.clone(),
            faults_only: st.faults_only,
            wide: st.wide,
            ..header.clone()
        };
        let sort_view = ui::SortState { column: st.sort.map(|s| s.column), descending: st.sort.is_some_and(|s| s.descending), choosing: st.sort_choosing, cursor: st.sort_choosing.then_some(st.sort_cursor) };
        let path_segments = full_path(&st.mode, location(st.current_kind, &st.back_stack, st.scope.as_ref()));
        let screen = crate::input::keymap::screen_of(&st.mode, st.current_kind).unwrap_or(crate::input::keymap::Screen::Other);
        let hints_owned: Vec<(String, &'static str)> = hints_for(&st.mode, st.current_kind).into_iter().map(|(k, d)| (st.keymap.display_hint(screen, k), d)).collect();
        let hints: Vec<(&str, &str)> = hints_owned.iter().map(|(k, d)| (k.as_str(), *d)).collect();
        // The sidebar shows Home and every category, with the cursor where the keys left it.
        if st.sidebar {
            let all = sidebar::entries(st.current_kind, &st.sidebar_folded, catalog, overview);
            let selected = if st.sidebar_focus { st.sidebar_cursor.min(all.len().saturating_sub(1)) } else { sidebar::current_index(&all) };
            ui::set_sidebar(Some(ui::Sidebar { rows: all.into_iter().map(|e| e.row).collect(), selected, focused: st.sidebar_focus }));
        } else {
            ui::set_sidebar(None);
        }
        // The info panel beside the list follows the selected row.
        let panel_wide = terminal.size().map(|s| s.width >= ui::SIDE_PANEL_MIN_WIDTH).unwrap_or(false);
        if st.info_panel && panel_wide && matches!(st.mode, Mode::List) && st.current_kind != ResourceKind::Overview {
            match handlers::selected_manifest(&st, derived, catalog, &client) {
                Some(manifest) => {
                    let key = format!("{:?}{}", st.current_kind, mode::object_title(&manifest));
                    if key != st.info_key {
                        st.info_key = key;
                        st.reveal = true;
                        st.info_scroll = 0;
                        st.info_hscroll = 0;
                    }
                    let sections = k8s::details::details(&manifest, &overview.events, st.reveal);
                    if let Ok(size) = terminal.size() {
                        let (down, right) = ui::side_panel_max_scroll(&sections, size);
                        st.info_scroll = st.info_scroll.min(down);
                        st.info_hscroll = st.info_hscroll.min(right);
                    }
                    ui::set_side_panel(Some(ui::SidePanel { title: mode::object_title(&manifest), sections, scroll: st.info_scroll, hscroll: st.info_hscroll, focused: st.info_focus }));
                }
                None => ui::set_side_panel(None),
            }
        } else {
            ui::set_side_panel(None);
        }
        // With the sidebar or the info panel open, the border of whichever pane has the keys is lit.
        let sidebar_shown = st.sidebar && terminal.size().map(|s| s.width >= ui::SIDEBAR_MIN_WIDTH).unwrap_or(false);
        let beside_others = (st.info_panel && panel_wide) || sidebar_shown;
        ui::set_content_unfocused(sidebar_shown && st.sidebar_focus && matches!(st.mode, Mode::List));
        ui::set_list_focused(beside_others && matches!(st.mode, Mode::List) && st.current_kind != ResourceKind::Overview && !st.info_focus && !st.sidebar_focus);
        // Marks belong to the list they were made in.
        if st.marked_kind != st.current_kind {
            st.marked.clear();
            st.marked_kind = st.current_kind;
        }
        let view = draw::View {
            rows: &rows_view,
            overview: &overview,
            nodes: &nodes,
            node_rows: &node_rows,
            usage: usage.as_ref(),
            node_detail_rows: &node_detail_rows,
            crds: &catalog.crds,
            extensions: &catalog.extensions.loaded,
            apis: &catalog.apis,
            favorites: &st.favorites,
            hints: &hints,
            show_hints_panel: st.show_hints_panel,
            path: &path_segments,
            header_now: &header_now,
            search: &st.search,
            sort_view,
            marked: &st.marked,
            config_preset: &st.config.theme.preset,
            config: &st.config,
        };
        let frame_area = draw::draw_mode(terminal, &mut st.mode, &view, &mut st.table_state, st.hovered, &mut st.icons, &mut st.hscroll)?;

        // A shell's output arrives on its own, so redraw quickly while one is open.
        let wait = if matches!(st.mode, Mode::Working { .. }) { 50 } else if matches!(st.mode, Mode::Shell { .. }) { crate::config::tunables::tunables().shell_redraw_ms } else { crate::config::tunables::tunables().idle_redraw_ms };
        if st.replay.is_none() && !event::poll(Duration::from_millis(wait))? {
            cache = Some(fresh);
            continue;
        }

        // Handle every queued event before redrawing. A trackpad flick queues dozens
        // of wheel events, and a key typed after it would otherwise wait behind them.
        loop {
            let event = match st.replay.take() {
                Some(key) => Event::Key(key),
                None => event::read()?,
            };
            let config_now = st.config.clone();
            if let Some(outcome) = handlers::dispatch(event, &mut st, &mut Cx { terminal, catalog, pod_store, dep_store, client: &client, config: &config_now, active_context, frame_area, row_count, d: derived })? {
                return Ok(outcome);
            }
            if !event::poll(Duration::from_millis(0))? {
                break;
            }
        }
        cache = Some(fresh);
    }
}
