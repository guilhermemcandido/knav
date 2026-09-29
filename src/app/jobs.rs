//! Cluster work runs in the background, so the screen keeps redrawing and Esc can
//! cancel it. It shows as a small "Working" popup.

use crate::ops::NoticeTone;
use std::{sync::Arc, time::{Duration, Instant}};

use super::mode::AbortOnDrop;
use crate::ops::{actions, portforward};

/// What a finished job hands back.
pub(crate) enum Done {
    Action(crate::ops::Outcome),
    /// The context that was checked, or why it can't be reached.
    Connect(Result<String, String>),
    Forward(Result<portforward::Forward, String>),
    /// What was being waited for has loaded; press this key again.
    Ready(crossterm::event::KeyEvent),
}

pub(crate) struct Job {
    pub title: String,
    pub started: Instant,
    pub progress: Arc<actions::Progress>,
    /// What Esc says, when there is something worth saying.
    pub cancel_note: Option<&'static str>,
    /// A popup was up when it started, so the screen stays dimmed until the popup
    /// shows. Otherwise the dimming blinks off and on between a question and its answer.
    pub backdrop: bool,
    rx: tokio::sync::oneshot::Receiver<Done>,
    _task: AbortOnDrop,
}

/// Quick jobs finish before this, so they never flash a popup.
const SHOW_AFTER: Duration = Duration::from_millis(250);

impl Job {
    pub(crate) fn spawn(title: impl Into<String>, progress: Arc<actions::Progress>, cancel_note: Option<&'static str>, work: impl std::future::Future<Output = Done> + Send + 'static) -> Job {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = tx.send(work.await);
        });
        Job { title: title.into(), started: Instant::now(), progress, cancel_note, backdrop: false, rx, _task: AbortOnDrop(task) }
    }

    pub(crate) fn poll(&mut self) -> Option<Done> {
        match self.rx.try_recv() {
            Ok(done) => Some(done),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
            Err(_) => Some(Done::Action(crate::ops::Outcome { text: "The task stopped unexpectedly".into(), tone: NoticeTone::Failed })),
        }
    }

    /// Whether it has run long enough to be worth showing.
    pub(crate) fn visible(&self) -> bool {
        self.started.elapsed() >= SHOW_AFTER
    }
}

use super::{mode::Mode, state::State};
use crate::ops::actions::{Action, Target};

/// Starts `job` over `back`, or over the screen up now when `None`, noting whether a
/// popup is up.
fn start(st: &mut State, mut job: Job, back: Option<Box<Mode>>) {
    job.backdrop = !matches!(st.mode, Mode::List);
    let back = back.unwrap_or_else(|| Box::new(std::mem::replace(&mut st.mode, Mode::List)));
    st.mode = Mode::Working { job, back };
}

/// Runs `action` on `targets` in the background, over `back`.
pub(super) fn run_action(st: &mut State, client: &kube::Client, targets: Vec<Target>, action: Action, back: Box<Mode>) {
    let progress = Arc::new(actions::Progress::default());
    let title = actions::working_title(action, &targets);
    let note = (targets.len() > 1).then_some("Cancelled. What was already done stays done.");
    let work = {
        let (client, progress) = (client.clone(), Arc::clone(&progress));
        async move { Done::Action(actions::run_many(client, targets, action, progress).await) }
    };
    start(st, Job::spawn(title, progress, note, work), Some(back));
}

/// Checks in the background that `name` can be reached before the session moves to it.
pub(super) fn check_context(st: &mut State, name: String) {
    let title = format!("Connecting to {name}");
    let work = async move {
        let check = async {
            let client = crate::k8s::connect_to_context(Some(&name)).await?;
            crate::k8s::ensure_reachable(&client, Some(&name)).await
        };
        Done::Connect(match check.await {
            Ok(_) => Ok(name),
            Err(e) => Err(e.to_string().lines().next().unwrap_or("connection failed").to_string()),
        })
    };
    start(st, Job::spawn(title, Arc::default(), None, work), None);
}

/// Starts a port-forward in the background.
pub(super) fn start_forward(st: &mut State, request: portforward::ForwardRequest, back: Box<Mode>) {
    let title = format!("Starting a forward to {}", request.resource);
    let work = portforward::start_in_background(request);
    let work = async move { Done::Forward(work.await.map_err(|e| format!("{e:#}"))) };
    start(st, Job::spawn(title, Arc::default(), None, work), Some(back));
}

/// Moves a finished job's result onto the screen. `Some` when the session should
/// reconnect to another context.
pub(super) fn finish(st: &mut State) -> Option<crate::SessionEnd> {
    let Mode::Working { job, .. } = &mut st.mode else { return None };
    let done = job.poll()?;
    let Mode::Working { back, .. } = std::mem::replace(&mut st.mode, Mode::List) else { return None };
    match done {
        Done::Action(outcome) => {
            if outcome.tone != NoticeTone::Failed {
                st.marked.clear();
            }
            st.mode = Mode::Notice { text: outcome.text, tone: outcome.tone, back };
        }
        Done::Connect(Ok(name)) => return Some(crate::SessionEnd::SwitchContext(name)),
        Done::Connect(Err(reason)) => {
            let mut back = *back;
            if let Mode::Context { error, .. } = &mut back {
                *error = Some(reason);
            }
            st.mode = back;
        }
        Done::Forward(Ok(forward)) => {
            let mut text = format!("Forwarding {} (:pf to stop)", forward.label());
            let url = forward.url();
            st.forwards.push(forward);
            st.mode = if st.config.portforward.open_browser {
                if let Err(e) = portforward::open_in_browser(&url) {
                    text.push_str(&format!("\n{e:#}"));
                }
                Mode::Notice { text, tone: NoticeTone::Done, back }
            } else {
                text.push_str(&format!("\nOpen {url} in the browser?"));
                Mode::OpenUrl { text, url, back }
            };
        }
        Done::Ready(key) => {
            st.mode = *back;
            st.replay = Some(key);
        }
        Done::Forward(Err(reason)) => st.mode = Mode::Notice { text: reason, tone: NoticeTone::Failed, back },
    }
    None
}

/// Waits (up to a limit) for `waits`, then replays `key` on the screen it came from.
pub(super) fn wait_then_replay(st: &mut State, title: &str, waits: Vec<futures::future::BoxFuture<'static, ()>>, key: crossterm::event::KeyEvent) {
    let work = async move {
        let _ = tokio::time::timeout(Duration::from_secs(20), futures::future::join_all(waits)).await;
        Done::Ready(key)
    };
    start(st, Job::spawn(title, Arc::default(), None, work), None);
}
