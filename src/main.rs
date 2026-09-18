mod config;
mod fuzzy;
mod icons;
mod k8s;
mod metrics;
mod picker;
mod ui;

use std::collections::HashMap;
use std::io::stdout;
use std::time::Duration;

use anyhow::{Context as _, Result};
use config::{Config, StartupMode, TimestampFormat};
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
    /// A vim/k9s-style `:` command line — `:q`/`:quit` exits, `:pods`/
    /// `:namespaces`/etc. (see `ResourceKind::from_command`) switches the
    /// current view. Esc cancels back to `List` without acting.
    Command { input: String },
    Menu { selected: (usize, usize) },
    Spec { title: String, items: Vec<TreeItem<'static, String>>, state: TreeState<String> },
    /// Freelens-style node drill-down: that node's own metrics + the
    /// pods scheduled on it. `current_kind` stays `Nodes` throughout —
    /// this just overlays on top, same as `Containers` overlays on Pods.
    NodeDetail { name: String, state: TableState },
    /// The full Events browser, opened by pressing Enter on the
    /// Overview's Events panel — every event, filterable by severity.
    Events { filter: k8s::EventFilter, state: TableState },
    Containers {
        title: String,
        namespace: String,
        pod: String,
        containers: Vec<k8s::ContainerInfo>,
        state: TableState,
        // Where Esc returns to — the Pods list normally, or the
        // NodeDetail view if this pod was opened from there.
        back: Box<Mode>,
    },
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
    /// Every discovered CRD kind — listed once at startup, watched lazily
    /// (see `resolve`) only once the user actually opens one.
    crds: Vec<k8s::CrdInfo>,
    crd_watches: HashMap<usize, Box<dyn k8s::CatalogKind>>,
}

impl Catalog {
    fn spawn(client: &Client, node_store: Store<Node>, crds: Vec<k8s::CrdInfo>) -> Self {
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
            crds,
            crd_watches: HashMap::new(),
        }
    }

    fn count(&self, kind: ResourceKind) -> usize {
        self.get(kind).map(|k| k.count()).unwrap_or(0)
    }

    /// Looks up the live watch for a built-in kind — `None` for Overview/
    /// Pods/Deployments/the CRD kinds, which aren't in `entries` (Pods/
    /// Deployments have their own specialized reflectors and row types;
    /// CRDs go through `resolve` instead since opening one may need to
    /// lazily start its watch).
    fn get(&self, kind: ResourceKind) -> Option<&dyn k8s::CatalogKind> {
        self.entries.iter().find(|(k, _, _)| *k == kind).map(|(_, _, b)| b.as_ref())
    }

    /// Like `get`, but also covers CRD kinds — starting their watch on
    /// first use ("watch on open", not eagerly for all installed CRDs).
    /// The one place `main::run` should go through to read rows/spec for
    /// whatever `current_kind` actually is.
    fn resolve(&mut self, kind: ResourceKind, client: &Client) -> Option<&dyn k8s::CatalogKind> {
        match kind {
            ResourceKind::CustomResource(index, _) => {
                if !self.crd_watches.contains_key(&index) {
                    let crd = self.crds.get(index)?.clone();
                    let (boxed, _handle) = k8s::watch_crd(client.clone(), &crd);
                    self.crd_watches.insert(index, boxed);
                }
                self.crd_watches.get(&index).map(|b| b.as_ref())
            }
            _ => self.get(kind),
        }
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
            (
                "Custom Resources",
                // "Custom Resources" itself is the whole unfiltered
                // picker; one further tile per discovered API group so
                // the (often long) flat list is organized the way
                // Freelens groups its own custom-resource menu. Every
                // count here is "how many CRD *kinds*", not a live
                // object count — known for free from discovery, no
                // watch needed, consistent with "list only, watch on
                // open".
                std::iter::once(("Custom Resources", self.crds.len()))
                    .chain(self.crd_groups().into_iter().map(|group| (group, self.crds.iter().filter(|c| c.group == group).count())))
                    .collect(),
            ),
        ]
    }

    /// Every distinct API group among the discovered CRDs, in the same
    /// order `discover_crds` already sorted them (group, then kind) —
    /// a simple adjacent-dedup instead of a `HashSet` keeps that order
    /// intact instead of scrambling it.
    fn crd_groups(&self) -> Vec<&'static str> {
        let mut groups: Vec<&'static str> = Vec::new();
        for crd in &self.crds {
            if groups.last() != Some(&crd.group) {
                groups.push(crd.group);
            }
        }
        groups
    }

    /// Resolves an Overview tile's/menu's label back to the `ResourceKind`
    /// it switches to. Tries the fixed kinds first (`ResourceKind::
    /// from_label`); a label that isn't one of those but does match a
    /// discovered CRD group must be that group's tile (the "Custom
    /// Resources" section is the only place such labels appear).
    fn kind_for_tile_label(&self, label: &str) -> Option<ResourceKind> {
        ResourceKind::from_label(label).or_else(|| self.crds.iter().find(|c| c.group == label).map(|c| ResourceKind::CustomResourceGroup(c.group)))
    }
}

/// The `m` menu's layout — same six categories as the Overview catalog.
/// One shared function so the popup's render pass and its keyboard/Enter
/// handling can't drift apart.
fn menu_sections() -> [ui::MenuSection<'static>; 7] {
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
        ui::MenuSection { title: "Custom Resources", tiles: &[ResourceKind::CustomResourceList] },
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

/// Command-line arguments — deliberately hand-rolled instead of pulling in
/// a full argument-parsing crate for what's currently a single flag.
struct Cli {
    /// `-c`/`--context <query>` — fuzzy-matched against the kubeconfig's
    /// contexts and connected to directly, bypassing the cluster picker
    /// regardless of `startup.mode`.
    context_query: Option<String>,
}

impl Cli {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut context_query = None;
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-c" | "--context" => {
                    context_query = Some(args.next().with_context(|| format!("{arg} requires a value"))?);
                }
                "-h" | "--help" => {
                    println!(
                        "knav [-c|--context <name>]\n\n  -c, --context <name>  fuzzy-match a kubeconfig context and connect to it directly\n  -h, --help            show this help"
                    );
                    std::process::exit(0);
                }
                other => anyhow::bail!("unrecognized argument: {other} (try --help)"),
            }
        }
        Ok(Cli { context_query })
    }
}

/// Resolves which kubeconfig context to connect to, before anything else
/// starts up — `--context` always wins (resolved once, non-interactively,
/// via fuzzy match); otherwise the config's `startup.mode` decides between
/// connecting directly (k9s-style, the default) or showing the
/// freelens-style cluster picker first. `Ok(None)` from the picker means
/// the user cancelled, which should exit knav entirely rather than
/// silently falling back to some default cluster.
fn resolve_context(cli: &Cli, config: &Config) -> Result<Option<String>> {
    if let Some(query) = &cli.context_query {
        let contexts = k8s::list_contexts()?;
        return fuzzy::best_match(query, contexts.iter().map(|c| c.name.as_str()))
            .map(|name| Some(name.to_string()))
            .with_context(|| format!("no kubeconfig context matches '{query}'"));
    }

    match config.startup.mode {
        StartupMode::Direct => Ok(None),
        StartupMode::Menu => {
            let contexts = k8s::list_contexts()?;
            match picker::run(&contexts)? {
                Some(name) => Ok(Some(name)),
                None => std::process::exit(0),
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Read before the TUI takes over the screen — a parse error needs to
    // print somewhere a human can actually see it.
    let config = Config::load();
    let cli = Cli::parse(std::env::args().skip(1))?;
    let context = resolve_context(&cli, &config)?;

    let client = k8s::connect_to_context(context.as_deref()).await?;
    let (pod_store, _pod_watch_handle) = k8s::watch_pods(client.clone());
    let (dep_store, _dep_watch_handle) = k8s::watch_deployments(client.clone());
    let (node_store, _node_watch_handle) = k8s::watch_nodes(client.clone());
    let (event_store, _event_watch_handle) = k8s::watch_events(client.clone());
    let (node_metrics_rx, _metrics_handle) = metrics::watch_node_metrics(client.clone());
    let crds = k8s::discover_crds(&client).await;
    let mut catalog = Catalog::spawn(&client, node_store.clone(), crds);

    // Block until each reflector's initial list-and-watch has populated
    // its store at least once, so the first frame isn't just empty.
    pod_store.wait_until_ready().await?;
    dep_store.wait_until_ready().await?;
    node_store.wait_until_ready().await?;
    event_store.wait_until_ready().await?;

    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;

    let result =
        run(&mut terminal, &pod_store, &dep_store, &node_store, &event_store, &node_metrics_rx, &mut catalog, client, &config);

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
    catalog: &mut Catalog,
    client: Client,
    config: &Config,
) -> Result<()> {
    let mut table_state = TableState::default().with_selected(0);
    let mut mode = Mode::List;
    let mut hovered: Option<ui::Hover> = None;
    let mut current_kind = ResourceKind::Overview;
    // Queries the terminal's actual graphics capability (Kitty/Sixel/
    // iTerm2, falling back to halfblocks) — must happen after raw mode is
    // enabled and before the event-read loop below starts, so its own
    // terminal query doesn't race with crossterm's stdin reads.
    let mut icons = icons::IconCache::detect();
    let mut overview_selection = ui::OverviewSelection::Item(0, 0);
    // Horizontal scroll offset into the Overview's columns (Cluster,
    // Workloads, Config, ... — one per catalog category).
    let mut overview_col_scroll: usize = 0;
    // Vertical scroll offset into whichever column currently holds the
    // selection — item cards are tall enough now that a category like
    // Workloads can't always fit on screen at once.
    let mut overview_item_scroll: usize = 0;

    loop {
        let pods = k8s::snapshot(pod_store);
        let pod_rows: Vec<k8s::PodRow> = pods.iter().map(|p| k8s::row_for(p)).collect();
        let deployments = k8s::snapshot_deployments(dep_store);
        let dep_rows: Vec<k8s::DeploymentRow> = deployments.iter().map(|d| k8s::row_for_deployment(d)).collect();
        let nodes = node_store.state();
        let events = event_store.state();
        let usage = node_metrics_rx.borrow().clone();
        // Only populated while actually viewing a node's detail — which
        // pod, out of everything on the cluster, is scheduled on this
        // one node.
        let node_detail_pods: Vec<std::sync::Arc<Pod>> = if let Mode::NodeDetail { name, .. } = &mode {
            pods.iter().filter(|p| p.spec.as_ref().and_then(|s| s.node_name.as_deref()) == Some(name.as_str())).cloned().collect()
        } else {
            Vec::new()
        };
        let node_detail_rows: Vec<k8s::PodRow> = node_detail_pods.iter().map(|p| k8s::row_for(p)).collect();
        // Nodes get their own specialized rows (CPU/Memory visible right
        // in the list) instead of the generic Namespace/Name/Age table.
        // Sorted the same way `snapshot_generic` sorts the generic Nodes
        // catalog entry, so this stays index-aligned with `generic_rows`
        // for the 'd' (spec) key, which still goes through that entry.
        let sorted_nodes = k8s::snapshot_generic(node_store);
        let node_rows: Vec<k8s::NodeRow> = sorted_nodes
            .iter()
            .map(|n| {
                let name = n.metadata.name.clone().unwrap_or_default();
                let node_usage = usage.as_ref().and_then(|u| u.for_node(&name));
                let pod_count =
                    pods.iter().filter(|p| p.spec.as_ref().and_then(|s| s.node_name.as_deref()) == Some(name.as_str())).count();
                k8s::node_row(n, node_usage, pod_count)
            })
            .collect();
        let catalog_sections = catalog.sections(pod_rows.len(), dep_rows.len());
        let overview = k8s::overview(&nodes, &events, usage.as_ref(), catalog_sections);
        // Only ever populated for whatever kind is currently on screen —
        // computed unconditionally so every match arm below can just read
        // it, same as `pod_rows`/`dep_rows` are always computed too.
        // `resolve` also lazily starts a CRD's watch the first time it's
        // the current kind — "watch on open", not for every installed CRD.
        let generic_rows: Vec<k8s::GenericRow> = catalog.resolve(current_kind, &client).map(|k| k.rows()).unwrap_or_default();
        // The CRD picker, unfiltered or scoped to one API group — each
        // entry keeps its real index into `catalog.crds` (needed to open
        // the right one on Enter even though this may be a filtered
        // subset of the full list).
        let crd_rows: Vec<(usize, k8s::CrdInfo)> = match current_kind {
            ResourceKind::CustomResourceList => catalog.crds.iter().cloned().enumerate().collect(),
            ResourceKind::CustomResourceGroup(group) => {
                catalog.crds.iter().cloned().enumerate().filter(|(_, c)| c.group == group).collect()
            }
            _ => Vec::new(),
        };

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
            _ => ui::Rows::Generic(&generic_rows, current_kind.label()),
        };

        let mut frame_area = Rect::default();
        match &mut mode {
            Mode::List => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    ui::draw(frame, rows_view(), &mut table_state, hovered, None, &mut icons);
                })?;
            }
            Mode::Command { input } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Command { input };
                    ui::draw(frame, rows_view(), &mut table_state, hovered, Some(overlay), &mut icons);
                })?;
            }
            Mode::Menu { selected } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let sections = menu_sections();
                    let overlay = ui::Overlay::Menu { sections: &sections, selected: *selected };
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay), &mut icons);
                })?;
            }
            Mode::Spec { title, items, state } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Spec { title, items, state };
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay), &mut icons);
                })?;
            }
            Mode::Containers { title, containers, state, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Containers { title, containers, state };
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay), &mut icons);
                })?;
            }
            Mode::NodeDetail { name, state } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let found_node = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str()));
                    let capacity = found_node.map(|n| k8s::node_capacity(n));
                    let detail_info = found_node.map(|n| k8s::node_detail_info(n));
                    let node_usage = usage.as_ref().and_then(|u| u.for_node(name));
                    let overlay = ui::Overlay::NodeDetail {
                        name: name.as_str(),
                        cpu_usage: node_usage.map(|u| u.cpu_millicores),
                        cpu_capacity: capacity.as_ref().map(|c| c.cpu_millicores).unwrap_or(0),
                        memory_usage: node_usage.map(|u| u.memory_bytes),
                        memory_capacity: capacity.as_ref().map(|c| c.memory_bytes).unwrap_or(0),
                        pod_capacity: capacity.as_ref().map(|c| c.pods).unwrap_or(0),
                        info: detail_info.as_ref(),
                        pods: &node_detail_rows,
                        state,
                    };
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay), &mut icons);
                })?;
            }
            Mode::Events { filter, state } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Events { events: &overview.events, filter: *filter, state };
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay), &mut icons);
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
                    ui::draw(frame, rows_view(), &mut table_state, None, Some(overlay), &mut icons);
                })?;
            }
        }

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        match (event::read()?, &mut mode) {
            (Event::Mouse(mouse), Mode::List) if mouse.kind == MouseEventKind::Moved || matches!(mouse.kind, MouseEventKind::Down(_)) => {
                if current_kind == ResourceKind::Overview {
                    let active_col = match overview_selection {
                        ui::OverviewSelection::Header(c) | ui::OverviewSelection::Item(c, _) => c,
                        ui::OverviewSelection::Resources | ui::OverviewSelection::Events => usize::MAX,
                    };
                    if matches!(mouse.kind, MouseEventKind::Down(_))
                        && let Some(hit) = ui::column_hit(
                            frame_area,
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
                    hovered = ui::row_at(frame_area, &table_state, row_count, mouse.column, mouse.row).map(|row| {
                        ui::Hover { row, column: mouse.column, row_on_screen: mouse.row }
                    });
                }
            }
            (Event::Key(key), Mode::List) if current_kind == ResourceKind::Overview => {
                let columns_area = ui::columns_area(frame_area, &overview);
                let cols_visible = ui::visible_columns(columns_area.width, overview.catalog.len());
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char(':') => {
                        mode = Mode::Command { input: String::new() };
                    }
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
                        mode = Mode::Menu { selected: menu_position_for(current_kind) };
                    }
                    KeyCode::Enter => match overview_selection {
                        ui::OverviewSelection::Resources => {}
                        ui::OverviewSelection::Events => {
                            mode = Mode::Events {
                                filter: k8s::EventFilter::default(),
                                state: TableState::default().with_selected(if overview.events.is_empty() { None } else { Some(0) }),
                            };
                        }
                        ui::OverviewSelection::Header(_) => {}
                        ui::OverviewSelection::Item(col, item) => {
                            if let Some((_, items)) = overview.catalog.get(col)
                                && let Some((label, _)) = items.get(item)
                                && let Some(kind) = catalog.kind_for_tile_label(label)
                            {
                                current_kind = kind;
                                table_state.select(Some(0));
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
                    let items_visible = ui::visible_items_per_column(columns_area.height);
                    overview_item_scroll = ui::scroll_columns_to_show(overview_item_scroll, items_visible, target_item);
                }
            }
            (Event::Key(key), Mode::Events { filter, state }) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => mode = Mode::List,
                KeyCode::Char('a') => *filter = k8s::EventFilter::All,
                KeyCode::Char('w') => *filter = k8s::EventFilter::Warnings,
                KeyCode::Char('n') => *filter = k8s::EventFilter::Normal,
                KeyCode::Char('j') | KeyCode::Down => {
                    let filtered_len = overview.events.iter().filter(|e| filter.matches(e)).count();
                    select_next(state, filtered_len);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    let filtered_len = overview.events.iter().filter(|e| filter.matches(e)).count();
                    select_prev(state, filtered_len);
                }
                _ => {}
            },
            (Event::Key(key), Mode::List) => match key.code {
                KeyCode::Char('q') => return Ok(()),
                // Esc backs out one level instead of quitting — to
                // Overview from any top-level kind, or to the specific
                // CRD-group picker (or the flat list, if discovery
                // somehow can't find it) one specific CRD kind's
                // instances came from, mirroring how you got there.
                KeyCode::Esc => {
                    current_kind = match current_kind {
                        ResourceKind::CustomResource(index, _) => catalog
                            .crds
                            .get(index)
                            .map(|c| ResourceKind::CustomResourceGroup(c.group))
                            .unwrap_or(ResourceKind::CustomResourceList),
                        _ => ResourceKind::Overview,
                    };
                    table_state.select(Some(0));
                }
                KeyCode::Char(':') => {
                    mode = Mode::Command { input: String::new() };
                }
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
                            && let Some(value) = catalog.resolve(current_kind, &client).and_then(|k| k.spec_at(index))
                        {
                            let title = format!("{}/{}", row.namespace, row.name);
                            open_spec_value(&mut mode, title, value);
                        }
                    }
                },
                KeyCode::Enter if matches!(current_kind, ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_)) => {
                    if let Some(index) = table_state.selected()
                        && let Some((real_index, crd)) = crd_rows.get(index)
                    {
                        current_kind = ResourceKind::CustomResource(*real_index, crd.kind);
                        table_state.select(Some(0));
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
                            back: Box::new(Mode::List),
                        };
                    }
                }
                // Freelens-style node drill-down: what's actually running
                // on this node, plus its own CPU/Memory/Pods gauges.
                KeyCode::Enter if current_kind == ResourceKind::Nodes => {
                    if let Some(index) = table_state.selected()
                        && let Some(row) = generic_rows.get(index)
                    {
                        mode = Mode::NodeDetail { name: row.name.clone(), state: TableState::default().with_selected(0) };
                    }
                }
                _ => {}
            },
            (Event::Key(key), Mode::Command { input }) => match key.code {
                KeyCode::Esc => mode = Mode::List,
                KeyCode::Enter => {
                    let cmd = input.trim().to_lowercase();
                    mode = Mode::List;
                    if matches!(cmd.as_str(), "q" | "quit" | "exit") {
                        return Ok(());
                    }
                    if let Some(kind) = k8s::ResourceKind::from_command(&cmd) {
                        current_kind = kind;
                        table_state.select(Some(0));
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
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
            (Event::Key(key), Mode::Containers { title, namespace, pod, containers, state, back }) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    mode = std::mem::replace(&mut **back, Mode::List);
                }
                KeyCode::Char('j') | KeyCode::Down => select_next(state, containers.len()),
                KeyCode::Char('k') | KeyCode::Up => select_prev(state, containers.len()),
                KeyCode::Enter => {
                    if let Some(container) = state.selected().and_then(|i| containers.get(i)) {
                        let log_title = format!("{namespace}/{pod}/{}", container.name);
                        let (rx, handle) =
                            k8s::stream_logs(client.clone(), namespace.clone(), pod.clone(), container.name.clone());
                        let containers_snapshot = Mode::Containers {
                            title: title.clone(),
                            namespace: namespace.clone(),
                            pod: pod.clone(),
                            containers: containers.clone(),
                            state: *state,
                            back: std::mem::replace(back, Box::new(Mode::List)),
                        };
                        mode = Mode::Logs {
                            title: log_title,
                            lines: Vec::new(),
                            scroll: 0,
                            follow: true,
                            timestamp_format: config.logs.timestamp_format,
                            rx,
                            handle,
                            back: Box::new(containers_snapshot),
                        };
                    }
                }
                _ => {}
            },
            (Event::Key(key), Mode::NodeDetail { name, state }) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => mode = Mode::List,
                KeyCode::Char('d') => {
                    if let Some(node) = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str())) {
                        let title = name.clone();
                        open_spec(&mut mode, title, node.as_ref());
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
                        let back = Box::new(Mode::NodeDetail { name: name.clone(), state: *state });
                        mode = Mode::Containers {
                            title,
                            namespace,
                            pod: pod_name,
                            containers,
                            state: TableState::default().with_selected(0),
                            back,
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
            ResourceKind::CustomResourceList,
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
