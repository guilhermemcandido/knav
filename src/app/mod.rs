//! The interactive loop: draws the current `Mode` and handles keyboard/mouse input.

mod derive;
mod draw;
mod handlers;
mod state;

use handlers::Cx;
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
    let mut st = State::new(icons::IconCache::detect(), Favorites::load(active_context));

    loop {
        let src = derive::Sources { pod_store, dep_store, node_store, event_store, node_metrics_rx, client: &client };
        let query = derive::Query { current_kind: st.current_kind, namespace: st.namespace.as_deref(), scope: st.scope.as_ref(), search: &st.search, sort: st.sort };
        let derived = derive::derive(&src, catalog, &st.mode, &query);
        let derive::Derived { pod_rows, dep_rows, nodes, usage, node_detail_rows, node_rows, overview, generic_headers, generic_rows, crd_rows, .. } = &derived;

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
        let hints = hints_for(&st.mode, st.current_kind);
        if !hints.is_empty() {
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
            if let Some(outcome) = handlers::dispatch(event, &mut st, &mut Cx { terminal, catalog, client: &client, config, active_context, frame_area, row_count, d: &derived })? {
                return Ok(outcome);
            }
            if !event::poll(Duration::from_millis(0))? {
                break;
            }
        }
    }
}
