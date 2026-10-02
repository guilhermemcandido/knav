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

/// How a connected session ended: quit, or reconnect to another context.
pub(crate) enum SessionEnd {
    Quit,
    SwitchContext(String),
}

fn main() -> Result<()> {
    let (context_query, pick, read_only) = match Cli::parse(std::env::args().skip(1))? {
        Cli::Version => {
            println!("knav {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Cli::Help => {
            print!("{}", cli::USAGE);
            return Ok(());
        }
        Cli::Update { yes } => return update::run(yes),
        Cli::Launch { context_query, pick, read_only } => (context_query, pick, read_only),
    };

    // Problems with the config are kept and shown in the app, since the screen clears when it starts.
    let (config, mut notes) = Config::load_reporting();
    let keymap = keymap::Keymap::from_app_config(&config);
    notes.extend(settings::apply(&config).into_iter().chain(keymap.1).chain(app::custom_problems(&config.commands, &keymap.0)));
    let mut context = resolve_context(context_query.as_deref(), pick, &config)?;

    // One runtime per session: dropping it stops every watch and log stream of the
    // old cluster, which switching context would otherwise leave running.
    loop {
        // Reloaded so a switch keeps what was saved in Settings meanwhile.
        let config = Config::load();
        let runtime = tokio::runtime::Runtime::new()?;
        let outcome = runtime.block_on(session(&config, context.as_deref(), std::mem::take(&mut notes), read_only));
        runtime.shutdown_background();
        // A context that can't be opened (an expired login, say) goes back to the picker
        // with why, rather than out with a wall of plugin output.
        let outcome = match outcome {
            Err(error) if error.downcast_ref::<k8s::Unreachable>().is_some() => {
                let failed = error.downcast::<k8s::Unreachable>().expect("checked above");
                let contexts = k8s::list_contexts().unwrap_or_default();
                match startup::picker::run(&contexts, Some(&failed))? {
                    Some(name) => {
                        context = Some(name);
                        continue;
                    }
                    None => {
                        eprintln!("{}: {}\n{}", failed.context, failed.reason, failed.fix);
                        return Ok(());
                    }
                }
            }
            other => other,
        };
        match outcome? {
            SessionEnd::Quit => return Ok(()),
            SessionEnd::SwitchContext(name) => {
                eprintln!("Connecting to {name}…");
                context = Some(name);
            }
        }
    }
}

pub(crate) async fn session(config: &Config, context: Option<&str>, notes: Vec<String>, read_only: bool) -> Result<SessionEnd> {
    let (client, k8s_version) = k8s::connect_checked(context).await?;
    // Before any watch starts, so they all list the faster way the server allows.
    k8s::detect_streaming(&client).await;
    // Everything starts loading now, beside the reachability check and loading screen.
    let (pod_reader, pod_feed, _pod_watch_handle) = k8s::watch_live::<Pod>(client.clone());
    let (dep_reader, dep_feed, _dep_watch_handle) = k8s::watch_live::<Deployment>(client.clone());
    let (node_store, node_feed, _node_watch_handle) = k8s::watch_live::<Node>(client.clone());
    let pod_store = k8s::PodKept::new(pod_reader, pod_feed, k8s::row_for);
    let dep_store = k8s::DeploymentKept::new(dep_reader, dep_feed, k8s::row_for_deployment);
    // Events are many and busy on a big cluster: they wait for the lists the first screen needs.
    let first_lists = {
        let (pods, deployments, nodes) = (pod_store.store.clone(), dep_store.store.clone(), node_store.clone());
        // Checked the way the loading screen does, a few times a second.
        async move {
            use futures::FutureExt;
            let ready = || {
                pods.wait_until_ready().now_or_never().is_some() && deployments.wait_until_ready().now_or_never().is_some() && nodes.wait_until_ready().now_or_never().is_some()
            };
            while !ready() {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    };
    let (event_store, _event_watch_handle) = k8s::watch_store_after::<k8s_openapi::api::core::v1::Event>(client.clone(), first_lists);
    let (node_metrics_rx, _metrics_handle) = metrics::watch_node_metrics(client.clone());
    let (pod_metrics, _pod_metrics_handle) = metrics::watch_pod_metrics(client.clone());
    // Discovery runs beside the first lists; the loading screen waits for all of them.
    let discovery = tokio::spawn({
        let client = client.clone();
        async move { k8s::discover(&client).await }
    });
    let access = tokio::spawn({
        let client = client.clone();
        async move { k8s::access::check_access(&client).await }
    });
    let registry = extensions::Registry::load(&Config::dir());
    let mut catalog = Catalog::spawn(&client, node_store.clone(), node_feed, registry.index());

    let active_context = match context {
        Some(name) => name.to_string(),
        None => k8s::list_contexts().ok().and_then(|c| c.into_iter().find(|c| c.is_current).map(|c| c.name)).unwrap_or_default(),
    };
    let info = k8s::list_contexts().ok().and_then(|c| c.into_iter().find(|c| c.name == active_context));
    let mut header = ui::HeaderInfo {
        context: active_context.clone(),
        cluster: info.as_ref().map(|c| c.cluster.clone()).unwrap_or_default(),
        user: info.map(|c| c.user).unwrap_or_default(),
        role: String::new(),
        read_only: false,
        highlight: false,
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

    // Usually done during the loading screen; a slow API server doesn't hold up the start.
    header.role = match tokio::time::timeout(std::time::Duration::from_secs(3), access).await {
        Ok(Ok(access)) if access != k8s::access::Access::Unknown => access.label(),
        _ => String::new(),
    };
    let stores = app::Stores { pods: &pod_store, deployments: &dep_store, nodes: &node_store, events: &event_store, node_metrics: &node_metrics_rx, pod_metrics: &pod_metrics };
    let session = app::Session { client, config, active_context: &active_context, header: &header, read_only };
    let result = app::run(&mut terminal, stores, &mut catalog, &registry, session, notes);

    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}
