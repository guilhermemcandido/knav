mod config;
mod k8s;
mod metrics;
mod ui;

use std::io::stdout;
use std::time::Duration;

use anyhow::Result;
use config::{Config, TimestampFormat};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind};
use crossterm::execute;
use k8s::ResourceKind;
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet},
    autoscaling::v2::HorizontalPodAutoscaler,
    batch::v1::{CronJob, Job},
    core::v1::{ConfigMap, Endpoints, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod, Secret, Service, ServiceAccount},
    networking::v1::{Ingress, NetworkPolicy},
    rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding},
    storage::v1::StorageClass,
};
use kube::{Client, runtime::reflector::Store};
use ratatui::{layout::Rect, widgets::TableState};
use tokio::sync::{mpsc, watch};
use tui_tree_widget::{TreeItem, TreeState};

enum Mode {
    List,
    Menu { selected: (usize, usize) },
    Spec { title: String, items: Vec<TreeItem<'static, String>>, state: TreeState<String> },
    Containers { title: String, namespace: String, pod: String, containers: Vec<k8s::ContainerInfo>, state: TableState },
    Logs {
        title: String,
        lines: Vec<String>,
        scroll: u16,
        follow: bool,
        timestamp_format: TimestampFormat,
        rx: mpsc::UnboundedReceiver<String>,
        handle: tokio::task::JoinHandle<()>,
        // What to go back to on Esc — the Containers view we came from,
        // so backing out of logs doesn't dump you all the way to the
        // pod list.
        back: Box<Mode>,
    },
}

/// Every resource kind that gets a live watch + generic list/spec view but
/// no specialized row type (unlike Pods/Deployments). Nodes reuses the
/// existing `node_store` reflector instead of opening a second watch on
/// the same kind; everything else spawns its own.
struct Catalog {
    entries: Vec<(ResourceKind, &'static str, Box<dyn k8s::CatalogKind>)>,
}

impl Catalog {
    fn spawn(client: &Client, node_store: Store<Node>) -> Self {
        macro_rules! kind {
            ($variant:ident, $label:literal, $ty:ty) => {{
                let (boxed, _handle) = k8s::watch_kind::<$ty>(client.clone());
                (ResourceKind::$variant, $label, boxed)
            }};
        }
        Catalog {
            entries: vec![
                (ResourceKind::Nodes, "Nodes", Box::new(k8s::WatchedKind::from_store(node_store))),
                kind!(Namespaces, "Namespaces", Namespace),
                kind!(ReplicaSets, "ReplicaSets", ReplicaSet),
                kind!(StatefulSets, "StatefulSets", StatefulSet),
                kind!(DaemonSets, "DaemonSets", DaemonSet),
                kind!(Jobs, "Jobs", Job),
                kind!(CronJobs, "CronJobs", CronJob),
                kind!(ConfigMaps, "ConfigMaps", ConfigMap),
                kind!(Secrets, "Secrets", Secret),
                kind!(Hpas, "HPAs", HorizontalPodAutoscaler),
                kind!(Services, "Services", Service),
                kind!(Endpoints, "Endpoints", Endpoints),
                kind!(Ingresses, "Ingresses", Ingress),
                kind!(NetworkPolicies, "NetworkPolicies", NetworkPolicy),
                kind!(Pvcs, "PVCs", PersistentVolumeClaim),
                kind!(Pvs, "PVs", PersistentVolume),
                kind!(StorageClasses, "StorageClasses", StorageClass),
                kind!(ServiceAccounts, "ServiceAccounts", ServiceAccount),
                kind!(Roles, "Roles", Role),
                kind!(RoleBindings, "RoleBindings", RoleBinding),
                kind!(ClusterRoles, "ClusterRoles", ClusterRole),
                kind!(ClusterRoleBindings, "ClusterRoleBindings", ClusterRoleBinding),
            ],
        }
    }

    fn count(&self, kind: ResourceKind) -> usize {
        self.get(kind).map(|k| k.count()).unwrap_or(0)
    }

    /// Looks up the live watch for a kind — `None` for Overview/Pods/
    /// Deployments, which aren't in `entries` (they have their own
    /// specialized reflectors and row types, handled directly in `run`).
    fn get(&self, kind: ResourceKind) -> Option<&dyn k8s::CatalogKind> {
        self.entries.iter().find(|(k, _, _)| *k == kind).map(|(_, _, b)| b.as_ref())
    }

    /// Merges in the live-reflector counts for Pods/Deployments so
    /// callers get one complete catalog instead of two partial ones.
    fn sections(&self, pod_count: usize, deployment_count: usize) -> Vec<(&'static str, Vec<(&'static str, usize)>)> {
        vec![
            ("Cluster", vec![("Nodes", self.count(ResourceKind::Nodes)), ("Namespaces", self.count(ResourceKind::Namespaces))]),
            (
                "Workloads",
                vec![
                    ("Pods", pod_count),
                    ("Deployments", deployment_count),
                    ("ReplicaSets", self.count(ResourceKind::ReplicaSets)),
                    ("StatefulSets", self.count(ResourceKind::StatefulSets)),
                    ("DaemonSets", self.count(ResourceKind::DaemonSets)),
                    ("Jobs", self.count(ResourceKind::Jobs)),
                    ("CronJobs", self.count(ResourceKind::CronJobs)),
                ],
            ),
            (
                "Config",
                vec![
                    ("ConfigMaps", self.count(ResourceKind::ConfigMaps)),
                    ("Secrets", self.count(ResourceKind::Secrets)),
                    ("HPAs", self.count(ResourceKind::Hpas)),
                ],
            ),
            (
                "Network",
                vec![
                    ("Services", self.count(ResourceKind::Services)),
                    ("Endpoints", self.count(ResourceKind::Endpoints)),
                    ("Ingresses", self.count(ResourceKind::Ingresses)),
                    ("NetworkPolicies", self.count(ResourceKind::NetworkPolicies)),
                ],
            ),
            (
                "Storage",
                vec![
                    ("PVCs", self.count(ResourceKind::Pvcs)),
                    ("PVs", self.count(ResourceKind::Pvs)),
                    ("StorageClasses", self.count(ResourceKind::StorageClasses)),
                ],
            ),
            (
                "Access Control",
                vec![
                    ("ServiceAccounts", self.count(ResourceKind::ServiceAccounts)),
                    ("Roles", self.count(ResourceKind::Roles)),
                    ("RoleBindings", self.count(ResourceKind::RoleBindings)),
                    ("ClusterRoles", self.count(ResourceKind::ClusterRoles)),
                    ("ClusterRoleBindings", self.count(ResourceKind::ClusterRoleBindings)),
                ],
            ),
        ]
    }
}

/// Maps an Overview tile's/menu's display label back to the `ResourceKind`
/// it switches to — the join key between the (label, count) tuples the
/// catalog renders and the enum `current_kind` actually switches on.
fn kind_for_label(label: &str) -> Option<ResourceKind> {
    match label {
        "Pods" => Some(ResourceKind::Pods),
        "Deployments" => Some(ResourceKind::Deployments),
        "Nodes" => Some(ResourceKind::Nodes),
        "Namespaces" => Some(ResourceKind::Namespaces),
        "ReplicaSets" => Some(ResourceKind::ReplicaSets),
        "StatefulSets" => Some(ResourceKind::StatefulSets),
        "DaemonSets" => Some(ResourceKind::DaemonSets),
        "Jobs" => Some(ResourceKind::Jobs),
        "CronJobs" => Some(ResourceKind::CronJobs),
        "ConfigMaps" => Some(ResourceKind::ConfigMaps),
        "Secrets" => Some(ResourceKind::Secrets),
        "HPAs" => Some(ResourceKind::Hpas),
        "Services" => Some(ResourceKind::Services),
        "Endpoints" => Some(ResourceKind::Endpoints),
        "Ingresses" => Some(ResourceKind::Ingresses),
        "NetworkPolicies" => Some(ResourceKind::NetworkPolicies),
        "PVCs" => Some(ResourceKind::Pvcs),
        "PVs" => Some(ResourceKind::Pvs),
        "StorageClasses" => Some(ResourceKind::StorageClasses),
        "ServiceAccounts" => Some(ResourceKind::ServiceAccounts),
        "Roles" => Some(ResourceKind::Roles),
        "RoleBindings" => Some(ResourceKind::RoleBindings),
        "ClusterRoles" => Some(ResourceKind::ClusterRoles),
        "ClusterRoleBindings" => Some(ResourceKind::ClusterRoleBindings),
        _ => None,
    }
}

/// The `m` menu's layout — same six categories as the Overview catalog.
/// One shared function so the popup's render pass and its keyboard/Enter
/// handling can't drift apart (same principle as `build_catalog_rows`
/// backing the Overview grid's render + navigation).
fn menu_sections() -> [ui::MenuSection<'static>; 6] {
    [
        ui::MenuSection { title: "Cluster", tiles: &[ResourceKind::Overview, ResourceKind::Nodes, ResourceKind::Namespaces] },
        ui::MenuSection {
            title: "Workloads",
            tiles: &[
                ResourceKind::Pods,
                ResourceKind::Deployments,
                ResourceKind::ReplicaSets,
                ResourceKind::StatefulSets,
                ResourceKind::DaemonSets,
                ResourceKind::Jobs,
                ResourceKind::CronJobs,
            ],
        },
        ui::MenuSection { title: "Config", tiles: &[ResourceKind::ConfigMaps, ResourceKind::Secrets, ResourceKind::Hpas] },
        ui::MenuSection {
            title: "Network",
            tiles: &[ResourceKind::Services, ResourceKind::Endpoints, ResourceKind::Ingresses, ResourceKind::NetworkPolicies],
        },
        ui::MenuSection { title: "Storage", tiles: &[ResourceKind::Pvcs, ResourceKind::Pvs, ResourceKind::StorageClasses] },
        ui::MenuSection {
            title: "Access Control",
            tiles: &[
                ResourceKind::ServiceAccounts,
                ResourceKind::Roles,
                ResourceKind::RoleBindings,
                ResourceKind::ClusterRoles,
                ResourceKind::ClusterRoleBindings,
            ],
        },
    ]
}

/// Where a `ResourceKind` sits in the menu grid, so opening the menu
/// starts with the currently-viewed kind selected instead of always
/// resetting to the top-left tile.
fn menu_position_for(kind: ResourceKind) -> (usize, usize) {
    let sections = menu_sections();
    for (section_idx, section) in sections.iter().enumerate() {
        if let Some(tile_idx) = section.tiles.iter().position(|k| *k == kind) {
            return (section_idx, tile_idx);
        }
    }
    (0, 0)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Read before the TUI takes over the screen — a parse error needs to
    // print somewhere a human can actually see it.
    let config = Config::load();

    let client = k8s::connect().await?;
    let (pod_store, _pod_watch_handle) = k8s::watch_pods(client.clone());
    let (dep_store, _dep_watch_handle) = k8s::watch_deployments(client.clone());
    let (node_store, _node_watch_handle) = k8s::watch_nodes(client.clone());
    let (event_store, _event_watch_handle) = k8s::watch_events(client.clone());
    let (node_metrics_rx, _metrics_handle) = metrics::watch_node_metrics(client.clone());
    let catalog = Catalog::spawn(&client, node_store.clone());

    // Block until each reflector's initial list-and-watch has populated
    // its store at least once, so the first frame isn't just empty.
    pod_store.wait_until_ready().await?;
    dep_store.wait_until_ready().await?;
    node_store.wait_until_ready().await?;
    event_store.wait_until_ready().await?;

    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;

    let result = run(&mut terminal, &pod_store, &dep_store, &node_store, &event_store, &node_metrics_rx, &catalog, client, &config);

    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

#[allow(clippy::too_many_arguments)]
fn run(
    terminal: &mut ratatui::DefaultTerminal,
    pod_store: &Store<Pod>,
    dep_store: &Store<Deployment>,
    node_store: &Store<Node>,
    event_store: &Store<k8s_openapi::api::core::v1::Event>,
    node_metrics_rx: &watch::Receiver<Option<metrics::ClusterUsage>>,
    catalog: &Catalog,
    client: Client,
    config: &Config,
) -> Result<()> {
    let mut table_state = TableState::default().with_selected(0);
    let mut mode = Mode::List;
    let mut hovered: Option<ui::Hover> = None;
    let mut current_kind = ResourceKind::Overview;
    let mut overview_scroll: usize = 0;
    let mut overview_selected: (usize, usize) = (0, 0);

    loop {
        let pods = k8s::snapshot(pod_store);
        let pod_rows: Vec<k8s::PodRow> = pods.iter().map(|p| k8s::row_for(p)).collect();
        let deployments = k8s::snapshot_deployments(dep_store);
        let dep_rows: Vec<k8s::DeploymentRow> = deployments.iter().map(|d| k8s::row_for_deployment(d)).collect();
        let nodes = node_store.state();
        let events = event_store.state();
        let usage = node_metrics_rx.borrow().clone();
        let catalog_sections = catalog.sections(pod_rows.len(), dep_rows.len());
        let overview = k8s::overview(&nodes, &events, usage.as_ref(), catalog_sections);
        // Only ever populated for whatever kind is currently on screen —
        // computed unconditionally so every match arm below can just read
        // it, same as `pod_rows`/`dep_rows` are always computed too.
        let generic_rows: Vec<k8s::GenericRow> = catalog.get(current_kind).map(|k| k.rows()).unwrap_or_default();

        let row_count = match current_kind {
            ResourceKind::Overview => overview.warnings.len(),
            ResourceKind::Pods => pod_rows.len(),
            ResourceKind::Deployments => dep_rows.len(),
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
            ResourceKind::Overview => ui::Rows::Overview(&overview, overview_scroll, overview_selected),
            ResourceKind::Pods => ui::Rows::Pods(&pod_rows),
            ResourceKind::Deployments => ui::Rows::Deployments(&dep_rows),
            _ => ui::Rows::Generic(&generic_rows, current_kind.label()),
        };

        let mut frame_area = Rect::default();
        match &mut mode {
            Mode::List => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    ui::draw(frame, rows_view(), &mut table_state, hovered, None);
                })?;
            }
            Mode::Menu { selected } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let sections = menu_sections();
                    let overlay = ui::Overlay::Menu { sections: &sections, selected: *selected };
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay));
                })?;
            }
            Mode::Spec { title, items, state } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Spec { title, items, state };
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay));
                })?;
            }
            Mode::Containers { title, containers, state, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Containers { title, containers, state };
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay));
                })?;
            }
            Mode::Logs { title, lines, scroll, follow, timestamp_format, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Logs {
                        title,
                        lines,
                        scroll: *scroll,
                        follow: *follow,
                        timestamp_format: *timestamp_format,
                    };
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay));
                })?;
            }
        }

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        match (event::read()?, &mut mode) {
            (Event::Mouse(mouse), Mode::List) if mouse.kind == MouseEventKind::Moved || matches!(mouse.kind, MouseEventKind::Down(_)) => {
                if current_kind == ResourceKind::Overview {
                    if let Some(tile) = ui::tile_at(frame_area, &overview, overview_scroll, mouse.column, mouse.row) {
                        overview_selected = tile;
                    }
                } else {
                    hovered = ui::row_at(frame_area, &table_state, row_count, mouse.column, mouse.row).map(|row| {
                        ui::Hover { row, column: mouse.column, row_on_screen: mouse.row }
                    });
                }
            }
            (Event::Key(key), Mode::List) if current_kind == ResourceKind::Overview => {
                let catalog_area = ui::catalog_area(frame_area);
                let cols = ui::tile_cols(catalog_area.width);
                let mut moved = true;
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('j') | KeyCode::Down => {
                        overview_selected = ui::move_tile_selection(&overview, cols, overview_selected, ui::Direction::Down);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        overview_selected = ui::move_tile_selection(&overview, cols, overview_selected, ui::Direction::Up);
                    }
                    KeyCode::Char('h') | KeyCode::Left => {
                        overview_selected = ui::move_tile_selection(&overview, cols, overview_selected, ui::Direction::Left);
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
                        overview_selected = ui::move_tile_selection(&overview, cols, overview_selected, ui::Direction::Right);
                    }
                    KeyCode::Char('m') => {
                        mode = Mode::Menu { selected: menu_position_for(current_kind) };
                        moved = false;
                    }
                    KeyCode::Enter => {
                        moved = false;
                        if let Some((_, tiles)) = overview.catalog.get(overview_selected.0)
                            && let Some((label, _)) = tiles.get(overview_selected.1)
                            && let Some(kind) = kind_for_label(label)
                        {
                            current_kind = kind;
                            table_state.select(Some(0));
                        }
                    }
                    _ => moved = false,
                }
                if moved {
                    overview_scroll = ui::scroll_to_show(&overview, cols, catalog_area.height, overview_scroll, overview_selected);
                }
            }
            (Event::Key(key), Mode::List) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('j') | KeyCode::Down => select_next(&mut table_state, row_count),
                KeyCode::Char('k') | KeyCode::Up => select_prev(&mut table_state, row_count),
                KeyCode::Char('m') => {
                    mode = Mode::Menu { selected: menu_position_for(current_kind) };
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
                    _ => {
                        if let Some(index) = table_state.selected()
                            && let Some(row) = generic_rows.get(index)
                            && let Some(value) = catalog.get(current_kind).and_then(|k| k.spec_at(index))
                        {
                            let title = format!("{}/{}", row.namespace, row.name);
                            open_spec_value(&mut mode, title, value);
                        }
                    }
                },
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
                        };
                    }
                }
                _ => {}
            },
            (Event::Key(key), Mode::Menu { selected }) => {
                let sections = menu_sections();
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
                            table_state.select(Some(0));
                            mode = Mode::List;
                        }
                    }
                    _ => {}
                }
            }
            (Event::Key(key), Mode::Spec { state, .. }) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => mode = Mode::List,
                KeyCode::Char('j') | KeyCode::Down => {
                    state.key_down();
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.key_up();
                }
                KeyCode::Char('h') | KeyCode::Left => {
                    state.key_left();
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    state.key_right();
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    state.toggle_selected();
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
            (Event::Key(key), Mode::Containers { title, namespace, pod, containers, state }) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => mode = Mode::List,
                KeyCode::Char('j') | KeyCode::Down => select_next(state, containers.len()),
                KeyCode::Char('k') | KeyCode::Up => select_prev(state, containers.len()),
                KeyCode::Enter => {
                    if let Some(container) = state.selected().and_then(|i| containers.get(i)) {
                        let log_title = format!("{namespace}/{pod}/{}", container.name);
                        let (rx, handle) =
                            k8s::stream_logs(client.clone(), namespace.clone(), pod.clone(), container.name.clone());
                        let back = Mode::Containers {
                            title: title.clone(),
                            namespace: namespace.clone(),
                            pod: pod.clone(),
                            containers: containers.clone(),
                            state: *state,
                        };
                        mode = Mode::Logs {
                            title: log_title,
                            lines: Vec::new(),
                            scroll: 0,
                            follow: true,
                            timestamp_format: config.logs.timestamp_format,
                            rx,
                            handle,
                            back: Box::new(back),
                        };
                    }
                }
                _ => {}
            },
            (Event::Key(key), Mode::Logs { scroll, follow, timestamp_format, handle, back, .. }) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    handle.abort();
                    mode = std::mem::replace(&mut **back, Mode::List);
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    *follow = false;
                    *scroll = scroll.saturating_add(1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    *follow = false;
                    *scroll = scroll.saturating_sub(1);
                }
                KeyCode::Char('G') => *follow = true,
                KeyCode::Char(c) if c == config.keybindings.logs.toggle_timestamp => {
                    *timestamp_format = timestamp_format.toggled();
                }
                _ => {}
            },
            (Event::Mouse(mouse), Mode::Logs { scroll, follow, .. }) => match mouse.kind {
                MouseEventKind::ScrollDown => {
                    *follow = false;
                    *scroll = scroll.saturating_add(1);
                }
                MouseEventKind::ScrollUp => {
                    *follow = false;
                    *scroll = scroll.saturating_sub(1);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn title_for(namespace: Option<&str>, name: Option<&str>) -> String {
    format!("{}/{}", namespace.unwrap_or("?"), name.unwrap_or("?"))
}

fn open_spec<T: serde::Serialize>(mode: &mut Mode, title: String, item: &T) {
    open_spec_value(mode, title, k8s::manifest_value(item));
}

fn open_spec_value(mode: &mut Mode, title: String, value: serde_yaml::Value) {
    let items = ui::build_manifest_tree(&value);
    let mut state = TreeState::default();
    for item in &items {
        state.open(vec![item.identifier().clone()]);
    }
    *mode = Mode::Spec { title, items, state };
}

fn select_next(state: &mut TableState, len: usize) {
    if len == 0 {
        return;
    }
    let next = state.selected().map(|i| (i + 1).min(len - 1)).unwrap_or(0);
    state.select(Some(next));
}

fn select_prev(state: &mut TableState, len: usize) {
    if len == 0 {
        return;
    }
    let prev = state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
    state.select(Some(prev));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_stops_at_bottom_instead_of_wrapping() {
        let mut state = TableState::default().with_selected(0);
        for _ in 0..5 {
            select_next(&mut state, 3);
        }
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    fn prev_stops_at_top_instead_of_wrapping() {
        let mut state = TableState::default().with_selected(1);
        select_prev(&mut state, 3);
        select_prev(&mut state, 3);
        select_prev(&mut state, 3);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn empty_list_does_not_panic_or_select() {
        let mut state = TableState::default();
        select_next(&mut state, 0);
        select_prev(&mut state, 0);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn single_item_list_stays_put() {
        let mut state = TableState::default().with_selected(0);
        select_next(&mut state, 1);
        assert_eq!(state.selected(), Some(0));
        select_prev(&mut state, 1);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn menu_sections_cover_every_resource_kind_exactly_once() {
        let expected = [
            ResourceKind::Overview,
            ResourceKind::Nodes,
            ResourceKind::Namespaces,
            ResourceKind::Pods,
            ResourceKind::Deployments,
            ResourceKind::ReplicaSets,
            ResourceKind::StatefulSets,
            ResourceKind::DaemonSets,
            ResourceKind::Jobs,
            ResourceKind::CronJobs,
            ResourceKind::ConfigMaps,
            ResourceKind::Secrets,
            ResourceKind::Hpas,
            ResourceKind::Services,
            ResourceKind::Endpoints,
            ResourceKind::Ingresses,
            ResourceKind::NetworkPolicies,
            ResourceKind::Pvcs,
            ResourceKind::Pvs,
            ResourceKind::StorageClasses,
            ResourceKind::ServiceAccounts,
            ResourceKind::Roles,
            ResourceKind::RoleBindings,
            ResourceKind::ClusterRoles,
            ResourceKind::ClusterRoleBindings,
        ];
        let sections = menu_sections();
        let total: usize = sections.iter().map(|s| s.tiles.len()).sum();
        assert_eq!(total, expected.len(), "a kind is missing from (or duplicated in) the menu");
        for kind in expected {
            assert!(sections.iter().any(|s| s.tiles.contains(&kind)), "{} missing from menu_sections", kind.label());
        }
    }

    #[test]
    fn menu_position_for_finds_the_matching_tile() {
        let sections = menu_sections();
        let pos = menu_position_for(ResourceKind::ConfigMaps);
        assert_eq!(sections[pos.0].tiles[pos.1], ResourceKind::ConfigMaps);
    }
}
