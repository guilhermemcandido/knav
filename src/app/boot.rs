//! Opening a cluster: the loading screen, shown while the first lists and discovery arrive.

use super::*;
use futures::FutureExt;

/// Waiting longer than this for a step gets a hint on the screen.
const SLOW: Duration = Duration::from_secs(10);
/// The screen stays at least this long so it can be seen; any key skips it.
const MIN_SHOW: Duration = Duration::from_millis(1500);
/// After this long the app opens with whatever loaded, since a forbidden list never arrives.
const GIVE_UP: Duration = Duration::from_secs(45);

pub(crate) enum Boot {
    Ready,
    Quit,
}

fn ready<K>(store: &Store<K>) -> bool
where
    K: kube::Resource + Clone + 'static,
    K::DynamicType: Eq + std::hash::Hash + Clone,
{
    store.wait_until_ready().now_or_never().is_some_and(|r| r.is_ok())
}

/// Draws the loading screen until pods, deployments, nodes and discovery are in, or `q` is pressed.
pub(crate) async fn wait(
    terminal: &mut ratatui::DefaultTerminal,
    context: &str,
    version: &str,
    stores: (&Store<Pod>, &Store<Deployment>, &Store<Node>),
    discovery: tokio::task::JoinHandle<(Vec<k8s::ApiInfo>, Vec<k8s::CrdInfo>)>,
    catalog: &mut Catalog,
) -> Result<Boot> {
    let started = std::time::Instant::now();
    let mut discovery = Some(discovery);
    let mut discovered = false;
    let mut tick = 0;
    // What has arrived, in the order it did, with how long it took.
    let mut finished: Vec<(&'static str, Duration)> = Vec::new();
    loop {
        if discovery.as_ref().is_some_and(|d| d.is_finished())
            && let Some(done) = discovery.take()
        {
            if let Ok((apis, crds)) = done.await {
                catalog.set_types(apis, crds);
            }
            discovered = true;
        }
        let state = [
            ("API types", discovered, discovered.then(|| catalog.apis.len() + catalog.crds.len())),
            ("Nodes", ready(stores.2), Some(stores.2.len())),
            ("Pods", ready(stores.0), Some(stores.0.len())),
            ("Deployments", ready(stores.1), Some(stores.1.len())),
        ];
        for (label, done, _) in &state {
            if *done && !finished.iter().any(|(l, _)| l == label) {
                finished.push((label, started.elapsed()));
            }
        }
        let count_of = |label: &str| state.iter().find(|(l, ..)| *l == label).and_then(|(_, _, count)| *count);
        let mut steps: Vec<ui::Step> = finished.iter().map(|(label, took)| ui::Step { label, took: Some(*took), count: count_of(label) }).collect();
        // Still loading: what has streamed in so far, if anything.
        steps.extend(state.iter().filter(|(_, done, _)| !done).map(|(label, _, count)| ui::Step { label, took: None, count: count.filter(|n| *n > 0) }));
        let steps = steps;
        let waiting: Vec<String> = steps.iter().filter(|s| s.took.is_none()).map(|s| s.label.to_lowercase()).collect();
        if started.elapsed() > GIVE_UP || (waiting.is_empty() && started.elapsed() >= MIN_SHOW) {
            return Ok(Boot::Ready);
        }
        let hint = (started.elapsed() > SLOW).then(|| format!("Still waiting for {}", waiting.join(", ")));
        terminal.draw(|frame| ui::draw_loading(frame, &ui::Loading { context, version, steps: &steps, tick, hint: hint.as_deref() }))?;
        tick += 1;
        if event::poll(Duration::from_millis(80))?
            && let Event::Key(key) = event::read()?
            && key.kind == event::KeyEventKind::Press
        {
            if matches!(key.code, KeyCode::Char('q' | 'Q')) || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)) {
                return Ok(Boot::Quit);
            }
            // Once everything is in, any other key skips the rest of the wait.
            if waiting.is_empty() {
                return Ok(Boot::Ready);
            }
        }
    }
}
