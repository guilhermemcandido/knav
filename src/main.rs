mod app;
mod config;
mod input;
mod k8s;
mod ops;
mod startup;
mod theme;
mod ui;

// Short names for the leaf modules, so siblings can say `keys::encode`.
use app::{commands, mode};
use config::{favorites, settings};
use input::{keymap, keys};
use k8s::{catalog, metrics, scope, sort};
use ops::{actions, clipboard, edit, portforward, shell};
use startup::{cli, fuzzy, picker};
use ui::icons;

use std::collections::{HashMap, HashSet};
use std::io::stdout;
use std::time::Duration;

use anyhow::{Context as _, Result};
use config::{Config, LogOrder, StartupMode, TimestampFormat};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
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

use actions::{Action, Target};
use app::*;
use catalog::*;
use cli::*;
use commands::*;
use favorites::*;
use mode::*;
use scope::*;
use sort::*;

/// How one connected session ended: quit for good, or reconnect to a
/// different kubeconfig context.
pub(crate) enum Outcome {
    Quit,
    SwitchContext(String),
}

fn main() -> Result<()> {
    // Read before the TUI takes over the screen, a parse error needs to
    // print somewhere a human can actually see it.
    let config = Config::load();
    for problem in settings::apply(&config).into_iter().chain(keymap::Keymap::from_app_config(&config).1) {
        eprintln!("warning: {problem}");
    }
    let cli = Cli::parse(std::env::args().skip(1))?;
    let mut context = resolve_context(&cli, &config)?;

    // One runtime per connected session: dropping it kills every watch and
    // log-stream task spawned against the old cluster, which switching
    // context would otherwise leave running in the background forever.
    loop {
        // Reloaded so a switch keeps what was saved in Settings meanwhile.
        let config = Config::load();
        let runtime = tokio::runtime::Runtime::new()?;
        let outcome = runtime.block_on(session(&config, context.as_deref()));
        runtime.shutdown_background();
        match outcome? {
            Outcome::Quit => return Ok(()),
            Outcome::SwitchContext(name) => {
                eprintln!("Connecting to {name}…");
                context = Some(name);
            }
        }
    }
}

pub(crate) async fn session(config: &Config, context: Option<&str>) -> Result<Outcome> {
    let client = k8s::connect_to_context(context).await?;
    let k8s_version = k8s::ensure_reachable(&client, context).await?;
    let active_context = match context {
        Some(name) => name.to_string(),
        None => k8s::list_contexts().ok().and_then(|c| c.into_iter().find(|c| c.is_current).map(|c| c.name)).unwrap_or_default(),
    };
    let info = k8s::list_contexts().ok().and_then(|c| c.into_iter().find(|c| c.name == active_context));
    let header = ui::HeaderInfo {
        context: active_context.clone(),
        cluster: info.as_ref().map(|c| c.cluster.clone()).unwrap_or_default(),
        user: info.map(|c| c.user).unwrap_or_default(),
        role: "read-and-write".to_string(),
        namespace: "all".to_string(),
        namespace_slots: Vec::new(),
        scope: String::new(),
        k8s_version,
        knav_version: format!("v{}", env!("CARGO_PKG_VERSION")),
        faults_only: false,
        wide: false,
    };
    let (pod_store, _pod_watch_handle) = k8s::watch_store::<Pod>(client.clone());
    let (dep_store, _dep_watch_handle) = k8s::watch_store::<Deployment>(client.clone());
    let (node_store, _node_watch_handle) = k8s::watch_store::<Node>(client.clone());
    let (event_store, _event_watch_handle) = k8s::watch_store::<k8s_openapi::api::core::v1::Event>(client.clone());
    let (node_metrics_rx, _metrics_handle) = metrics::watch_node_metrics(client.clone());
    let crds = k8s::discover_crds(&client).await;
    let apis = k8s::discover_apis(&client).await;
    let mut catalog = Catalog::spawn(&client, node_store.clone(), crds, apis);

    // Block until each reflector's initial list-and-watch has populated
    // its store at least once, so the first frame isn't just empty.
    pod_store.wait_until_ready().await?;
    dep_store.wait_until_ready().await?;
    node_store.wait_until_ready().await?;
    event_store.wait_until_ready().await?;

    let mut terminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);

    let result = run(
        &mut terminal,
        &pod_store,
        &dep_store,
        &node_store,
        &event_store,
        &node_metrics_rx,
        &mut catalog,
        client,
        config,
        &active_context,
        &header,
    );

    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}
