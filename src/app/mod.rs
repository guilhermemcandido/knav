//! The interactive loop: draws the current `Mode` and handles keyboard/mouse input.

pub(crate) mod boot;
mod derive;
pub mod commands;
mod draw;
mod handlers;
pub(crate) use handlers::custom_problems;
pub(crate) mod jobs;
mod list_sort;
mod hints;
pub mod mode;
mod nav;
pub(crate) mod overview_layout;
mod path;
pub(crate) mod settings;
mod sidebar;
mod state;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{Client, runtime::reflector::Store};
use ratatui::{layout::Rect, widgets::TableState};
use tokio::sync::{mpsc, watch};
use tui_tree_widget::{TreeItem, TreeState};

use crate::SessionEnd;
use crate::config::{Config, LogOrder, TimestampFormat, favorites::{self, Favorites}};
use crate::extensions;
use crate::input::keys;
use crate::k8s::{self, ResourceKind, catalog::Catalog, metrics, scope::*, sort::*};
use crate::ops::actions::{self, Action, Target};
use crate::ops::{NoticeTone, clipboard, portforward, shell};
use crate::ui::{self, icons};
use crate::util::fuzzy;

use commands::*;
use handlers::Cx;
use hints::*;
use list_sort::ListSort;
use mode::*;
use nav::*;
use path::*;
use state::{State, Step};


/// The most log lines held for one stream.
const MAX_LOG_LINES: usize = 100_000;

/// The live stores a session reads from.
pub(crate) struct Stores<'a> {
    pub pods: &'a k8s::PodKept,
    pub deployments: &'a k8s::DeploymentKept,
    pub nodes: &'a Store<Node>,
    pub events: &'a Store<k8s_openapi::api::core::v1::Event>,
    pub node_metrics: &'a watch::Receiver<Option<metrics::ClusterUsage>>,
    pub pod_metrics: &'a metrics::PodMetricsFeed,
}

/// Which cluster the session is on, and the config it started with.
pub(crate) struct Session<'a> {
    pub client: Client,
    pub config: &'a Config,
    pub active_context: &'a str,
    pub header: &'a ui::HeaderInfo,
    /// `--read-only` was given.
    pub read_only: bool,
}

/// The usage the info view shows, from what metrics-server last said.
pub(crate) fn live_usage<'a>(pods: Option<&'a metrics::PodUsageMap>, nodes: Option<&'a metrics::ClusterUsage>) -> k8s::details::Usage<'a> {
    k8s::details::Usage { pods, nodes }
}

/// The header's role: what RBAC allows, or `read-only` while knav blocks changes,
/// since that is what applies. The real role stays in the Permissions menu.
fn role_label(role: &str, read_only: bool) -> String {
    match (role, read_only) {
        (_, false) => role.to_string(),
        (r, true) if r.starts_with("read-only") || r == "limited" => r.to_string(),
        (_, true) => "read-only".into(),
    }
}

pub(crate) fn run(terminal: &mut ratatui::DefaultTerminal, stores: Stores, catalog: &mut Catalog, registry: &extensions::Registry, session: Session, notes: Vec<String>) -> Result<SessionEnd> {
    let Stores { pods: pod_store, deployments: dep_store, nodes: node_store, events: event_store, node_metrics: node_metrics_rx, pod_metrics } = stores;
    let Session { client, config, active_context, header, read_only } = session;
    let mut st = State::new(icons::IconCache::detect(), Favorites::load(active_context), config.clone());
    st.read_only_flag = read_only;
    st.context = active_context.to_string();
    st.role = header.role.clone();
    if !notes.is_empty() {
        st.mode = Mode::Notice { text: format!("Problems with your settings:\n{}", notes.join("\n")), tone: NoticeTone::Failed, back: Box::new(Mode::List) };
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
        // Pod usage is polled only while pods or the info view are on screen.
        if st.current_kind == ResourceKind::Pods || mode::node_detail_name(&st.mode).is_some() || st.info_panel || matches!(st.mode, Mode::Details { .. } | Mode::Relations { .. }) {
            pod_metrics.want();
        }
        let src = derive::Sources { pod_store, dep_store, node_store, event_store, node_metrics_rx, pod_usage_rx: &pod_metrics.rx, client: &client, forwards: &forward_rows, registry };
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
        let derive::Derived { pod_rows, dep_rows, nodes, usage, pod_usage, problems, node_detail_rows, node_rows, overview, generic_headers, generic_rows, crd_rows, crd_counts, dashboard, .. } = derived;

        let row_count = match st.current_kind {
            ResourceKind::Overview => overview.events.len(),
            ResourceKind::Pods => pod_rows.len(),
            ResourceKind::Deployments => dep_rows.len(),
            ResourceKind::Nodes => node_rows.len(),
            ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => crd_rows.len(),
            ResourceKind::ExtensionDashboard(_) => 0,
            _ => generic_rows.len(),
        };
        // Counts are fetched only for the types on screen (and one screen further),
        // plus the enabled extensions' kinds, a small set the Overview always shows.
        catalog.ensure_helm(&client, &st.config.extensions.enabled);
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
        // Selection can't outrun the list as rows come and go.
        if st.current_kind != ResourceKind::Overview && row_count > 0 {
            let clamped = st.table_state.selected().unwrap_or(0).min(row_count - 1);
            st.table_state.select(Some(clamped));
        }

        // Logs keep arriving in the background; take whatever is ready before each redraw.
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
            ResourceKind::Overview => ui::Rows::Overview(overview, st.overview_selection, st.overview_col_scroll, st.overview_item_scroll),
            ResourceKind::Pods => ui::Rows::Pods(pod_rows, pod_usage.as_deref()),
            ResourceKind::Deployments => ui::Rows::Deployments(dep_rows),
            ResourceKind::Nodes => ui::Rows::Nodes(node_rows),
            ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => ui::Rows::CrdList(crd_rows, crd_counts, st.current_kind.label()),
            ResourceKind::ExtensionDashboard(_) => match dashboard {
                Some((title, content)) => ui::Rows::Dashboard(title, content, st.dashboard_scroll),
                None => ui::Rows::Generic(generic_rows, st.current_kind.label(), generic_headers),
            },
            _ => ui::Rows::Generic(generic_rows, st.current_kind.label(), generic_headers),
        };

        let header_now = ui::HeaderInfo {
            namespace: st.namespace.clone().unwrap_or_else(|| "all".into()),
            scope: st.scope.as_ref().map(Scope::label).unwrap_or_default(),
            namespace_slots: st.favorites.slots.clone(),
            faults_only: st.faults_only,
            wide: st.wide,
            role: role_label(&header.role, st.read_only()),
            read_only: st.read_only(),
            highlight: st.config.highlight.applies_to(&st.context),
            ..header.clone()
        };
        let sort_view = ui::SortState { column: st.sort.map(|s| s.column), descending: st.sort.is_some_and(|s| s.descending), choosing: st.sort_choosing, cursor: st.sort_choosing.then_some(st.sort_cursor) };
        let path_segments = full_path(&st.mode, location(st.current_kind, &st.back_stack, st.scope.as_ref()));
        let screen = mode::screen_of(&st.mode, st.current_kind).unwrap_or(crate::input::keymap::Screen::Other);
        let mut hints_owned: Vec<(String, &'static str)> = hints_for(&st.mode, st.current_kind).into_iter().filter(|h| !(st.read_only() && changes_cluster(h))).map(|(k, d)| (st.keymap.display_hint(screen, k), d)).collect();
        // Your own commands, on the lists they run on.
        if matches!(st.mode, Mode::List) && st.current_kind != ResourceKind::Overview {
            hints_owned.extend(st.command_hints.iter().cloned());
        }
        let hints: Vec<(&str, &str)> = hints_owned.iter().map(|(k, d)| (k.as_str(), *d)).collect();
        // The sidebar shows Home and every category, with the cursor where the keys left it.
        if st.sidebar {
            let all = sidebar::entries(st.current_kind, &st.sidebar_folded, catalog, overview);
            let selected = if st.sidebar_focus { st.sidebar_cursor.min(all.len().saturating_sub(1)) } else { sidebar::current_index(&all) };
            st.chrome.sidebar = Some(ui::Sidebar { rows: all.into_iter().map(|e| e.row).collect(), selected, focused: st.sidebar_focus });
        } else {
            st.chrome.sidebar = None;
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
                    let view = catalog.view_for(&st.config.extensions.enabled, &manifest);
                    let sections = k8s::details::details(&manifest, &overview.events, crate::app::live_usage(pod_usage.as_deref(), usage.as_ref()), st.reveal, view);
                    if let Ok(size) = terminal.size() {
                        let (down, right) = ui::side_panel_max_scroll(&sections, size);
                        st.info_scroll = st.info_scroll.min(down);
                        st.info_hscroll = st.info_hscroll.min(right);
                    }
                    st.chrome.panel = Some(ui::SidePanel { title: mode::object_title(&manifest), sections, scroll: st.info_scroll, hscroll: st.info_hscroll, focused: st.info_focus });
                }
                None => st.chrome.panel = None,
            }
        } else {
            st.chrome.panel = None;
        }
        // With the sidebar or panel open, the border of the pane with the keys is lit.
        let sidebar_shown = st.sidebar && terminal.size().map(|s| s.width >= ui::SIDEBAR_MIN_WIDTH).unwrap_or(false);
        let beside_others = (st.info_panel && panel_wide) || sidebar_shown;
        st.chrome.content_unfocused = sidebar_shown && st.sidebar_focus && matches!(st.mode, Mode::List);
        st.chrome.list_focused = beside_others && matches!(st.mode, Mode::List) && st.current_kind != ResourceKind::Overview && !st.info_focus && !st.sidebar_focus;
        // Marks belong to the list they were made in.
        if st.marked_kind != st.current_kind {
            st.marked.clear();
            st.marked_kind = st.current_kind;
        }
        let view = draw::View {
            rows: &rows_view,
            overview,
            nodes,
            node_rows,
            usage: usage.as_ref(),
            pod_usage: pod_usage.as_deref(),
            problems,
            node_detail_rows,
            crds: &catalog.crds,
            extensions: &registry.loaded,
            helm_present: catalog.count(ResourceKind::HelmReleases) > 0,
            layout_names: &catalog.layout_names(&st.config.extensions.enabled),
            dashboard_categories: &catalog.dashboard_categories(),
            apis: &catalog.apis,
            favorites: &st.favorites,
            hints: &hints,
            show_hints_panel: st.show_hints_panel,
            chrome: &st.chrome,
            path: &path_segments,
            header_now: &header_now,
            search: &st.search,
            sort_view,
            marked: &st.marked,
            config_preset: &st.config.theme.preset,
            config: &st.config,
            context: &st.context,
            role: &st.role,
            read_only: st.read_only(),
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
            if let Some(outcome) = handlers::dispatch(event, &mut st, &mut Cx { terminal, catalog, registry, pod_store, dep_store, client: &client, config: &config_now, active_context, frame_area, row_count, d: derived })? {
                return Ok(outcome);
            }
            if !event::poll(Duration::from_millis(0))? {
                break;
            }
        }
        cache = Some(fresh);
    }
}

#[cfg(test)]
mod role_tests {
    use super::role_label;

    #[test]
    fn read_only_mode_shows_what_applies() {
        assert_eq!(role_label("admin", false), "admin");
        assert_eq!(role_label("admin", true), "read-only");
        assert_eq!(role_label("read-write (team)", true), "read-only");
        assert_eq!(role_label("read-only (team)", true), "read-only (team)", "RBAC already says more");
        assert_eq!(role_label("", true), "read-only");
    }
}
