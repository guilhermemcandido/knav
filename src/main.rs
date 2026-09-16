mod config;
mod k8s;
mod ui;

use std::io::stdout;
use std::time::Duration;

use anyhow::Result;
use config::{Config, TimestampFormat};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind};
use crossterm::execute;
use k8s_openapi::api::{apps::v1::Deployment, core::v1::Pod};
use k8s::ResourceKind;
use kube::{Client, runtime::reflector::Store};
use ratatui::{layout::Rect, widgets::TableState};
use tokio::sync::mpsc;
use tui_tree_widget::{TreeItem, TreeState};

enum Mode {
    List,
    Menu { selected: usize },
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

    // Block until each reflector's initial list-and-watch has populated
    // its store at least once, so the first frame isn't just empty.
    pod_store.wait_until_ready().await?;
    dep_store.wait_until_ready().await?;
    node_store.wait_until_ready().await?;
    event_store.wait_until_ready().await?;

    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;

    let result = run(&mut terminal, &pod_store, &dep_store, &node_store, &event_store, client, &config);

    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    pod_store: &Store<Pod>,
    dep_store: &Store<Deployment>,
    node_store: &Store<k8s_openapi::api::core::v1::Node>,
    event_store: &Store<k8s_openapi::api::core::v1::Event>,
    client: Client,
    config: &Config,
) -> Result<()> {
    let mut table_state = TableState::default().with_selected(0);
    let mut mode = Mode::List;
    let mut hovered: Option<ui::Hover> = None;
    let mut current_kind = ResourceKind::Overview;

    loop {
        let pods = k8s::snapshot(pod_store);
        let pod_rows: Vec<k8s::PodRow> = pods.iter().map(|p| k8s::row_for(p)).collect();
        let deployments = k8s::snapshot_deployments(dep_store);
        let dep_rows: Vec<k8s::DeploymentRow> = deployments.iter().map(|d| k8s::row_for_deployment(d)).collect();
        let nodes = node_store.state();
        let events = event_store.state();
        let overview = k8s::overview(&pods, &deployments, &nodes, &events);

        let row_count = match current_kind {
            ResourceKind::Overview => overview.warnings.len(),
            ResourceKind::Pods => pod_rows.len(),
            ResourceKind::Deployments => dep_rows.len(),
        };
        // Selection can't outrun the list as pods/deployments come and go
        // underneath it.
        if row_count > 0 {
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
            ResourceKind::Overview => ui::Rows::Overview(&overview),
            ResourceKind::Pods => ui::Rows::Pods(&pod_rows),
            ResourceKind::Deployments => ui::Rows::Deployments(&dep_rows),
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
                    let sections = [
                        ui::MenuSection { title: "Cluster", tiles: &[ResourceKind::Overview] },
                        ui::MenuSection { title: "Workloads", tiles: &[ResourceKind::Pods, ResourceKind::Deployments] },
                    ];
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
            (Event::Mouse(mouse), Mode::List) if mouse.kind == MouseEventKind::Moved => {
                hovered = ui::row_at(frame_area, &table_state, row_count, mouse.column, mouse.row).map(|row| {
                    ui::Hover { row, column: mouse.column, row_on_screen: mouse.row }
                });
            }
            (Event::Key(key), Mode::List) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('j') | KeyCode::Down => select_next(&mut table_state, row_count),
                KeyCode::Char('k') | KeyCode::Up => select_prev(&mut table_state, row_count),
                KeyCode::Char('m') => {
                    let selected = ResourceKind::ALL.iter().position(|k| *k == current_kind).unwrap_or(0);
                    mode = Mode::Menu { selected };
                }
                KeyCode::Char('d') => match current_kind {
                    ResourceKind::Overview => {} // no single "selected object" concept here yet
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
            (Event::Key(key), Mode::Menu { selected }) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => mode = Mode::List,
                KeyCode::Char('h') | KeyCode::Left => *selected = selected.saturating_sub(1),
                KeyCode::Char('l') | KeyCode::Right => *selected = (*selected + 1).min(ResourceKind::ALL.len() - 1),
                KeyCode::Enter => {
                    current_kind = ResourceKind::ALL[*selected];
                    table_state.select(Some(0));
                    mode = Mode::List;
                }
                _ => {}
            },
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
    let value = k8s::manifest_value(item);
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
}
