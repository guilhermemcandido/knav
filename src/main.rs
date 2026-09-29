mod app;
mod config;
mod input;
mod ops;
mod startup;
mod ui;

use std::io::stdout;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use k8s_openapi::api::{apps::v1::Deployment, core::v1::{Node, Pod}};

use knav_common::{theme, util};
use knav_extensions as extensions;
use knav_k8s as k8s;

use app::settings;
use config::Config;
use input::keymap;
use k8s::{catalog::Catalog, metrics};
use startup::{cli::{self, Cli, resolve_context}, update};

/// How one connected session ended: quit for good, or reconnect to a
/// different kubeconfig context.
pub(crate) enum SessionEnd {
    Quit,
    SwitchContext(String),
}

fn main() -> Result<()> {
    let (context_query, pick) = match Cli::parse(std::env::args().skip(1))? {
        Cli::Version => {
            println!("knav {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Cli::Help => {
            print!("{}", cli::USAGE);
            return Ok(());
        }
        Cli::Update { yes } => return update::run(yes),
        Cli::Launch { context_query, pick } => (context_query, pick),
    };

    // Problems with the config are kept and shown in the app, since the screen clears when it starts.
    let (config, mut notes) = Config::load_reporting();
    notes.extend(settings::apply(&config).into_iter().chain(keymap::Keymap::from_app_config(&config).1));
    let mut context = resolve_context(context_query.as_deref(), pick, &config)?;

    // One runtime per connected session: dropping it kills every watch and
    // log-stream task spawned against the old cluster, which switching
    // context would otherwise leave running in the background forever.
    loop {
        // Reloaded so a switch keeps what was saved in Settings meanwhile.
        let config = Config::load();
        let runtime = tokio::runtime::Runtime::new()?;
        let outcome = runtime.block_on(session(&config, context.as_deref(), std::mem::take(&mut notes)));
        runtime.shutdown_background();
        match outcome? {
            SessionEnd::Quit => return Ok(()),
            SessionEnd::SwitchContext(name) => {
                eprintln!("Connecting to {name}…");
                context = Some(name);
            }
        }
    }
}

pub(crate) async fn session(config: &Config, context: Option<&str>, notes: Vec<String>) -> Result<SessionEnd> {
    let client = k8s::connect_to_context(context).await?;
    // Everything that loads starts now, beside the reachability check and the loading screen.
    let (pod_reader, pod_feed, _pod_watch_handle) = k8s::watch_live::<Pod>(client.clone());
    let (dep_reader, dep_feed, _dep_watch_handle) = k8s::watch_live::<Deployment>(client.clone());
    let (node_store, node_feed, _node_watch_handle) = k8s::watch_live::<Node>(client.clone());
    let pod_store = k8s::PodKept::new(pod_reader, pod_feed, k8s::row_for);
    let dep_store = k8s::DeploymentKept::new(dep_reader, dep_feed, k8s::row_for_deployment);
    let (event_store, _event_watch_handle) = k8s::watch_store::<k8s_openapi::api::core::v1::Event>(client.clone());
    let (node_metrics_rx, _metrics_handle) = metrics::watch_node_metrics(client.clone());
    // Discovery runs beside the first lists; the loading screen waits for all of them.
    let discovery = tokio::spawn({
        let client = client.clone();
        async move { k8s::discover(&client).await }
    });
    let registry = extensions::Registry::load(&Config::dir());
    let mut catalog = Catalog::spawn(&client, node_store.clone(), node_feed, registry.index());

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
    let mut terminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);
    let stores = (&pod_store.store, &dep_store.store, &node_store);
    if let app::boot::Boot::Quit = app::boot::wait(&mut terminal, &active_context, &header.k8s_version, stores, discovery, &mut catalog).await? {
        let _ = execute!(stdout(), DisableMouseCapture);
        ratatui::restore();
        return Ok(SessionEnd::Quit);
    }

    let result = app::run(
        &mut terminal,
        &pod_store,
        &dep_store,
        &node_store,
        &event_store,
        &node_metrics_rx,
        &mut catalog,
        &registry,
        client,
        config,
        &active_context,
        &header,
        notes,
    );

    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}
