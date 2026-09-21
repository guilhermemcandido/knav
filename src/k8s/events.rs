
use futures::StreamExt;
use k8s_openapi::api::core::v1::{Event, Node};
use kube::{
    Client,
    api::Api,
    runtime::{WatchStreamExt, reflector, watcher},
};
use tokio::task::JoinHandle;

use super::*;

/// Kubernetes only defines two event severities — there's no distinct
/// "Error" type, just `Normal`/`Warning` — so filtering/coloring can only
/// ever be grounded in these two, not a fabricated third bucket.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EventSeverity {
    Normal,
    Warning,
}

/// Which events the dedicated Events browser shows — cycled with a/w/n.
/// Only `Normal`/`Warning` exist because that's all Kubernetes itself
/// defines for an Event's `type`; there's no separate "Error" to filter
/// on.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum EventFilter {
    #[default]
    All,
    Warnings,
    Normal,
}

impl EventFilter {
    pub fn matches(self, entry: &EventEntry) -> bool {
        match self {
            EventFilter::All => true,
            EventFilter::Warnings => entry.severity == EventSeverity::Warning,
            EventFilter::Normal => entry.severity == EventSeverity::Normal,
        }
    }
}

/// The events the browser shows: the severity filter, then the `/` text
/// search — a case-insensitive substring of the reason, object, kind or
/// message (prose, so substring rather than fuzzy).
pub fn filter_events<'a>(events: &'a [EventEntry], filter: EventFilter, search: &str, sort: Option<crate::sort::SortSpec>) -> Vec<&'a EventEntry> {
    let needle = search.to_lowercase();
    let mut shown: Vec<&EventEntry> = events
        .iter()
        .filter(|e| filter.matches(e))
        .filter(|e| {
            needle.is_empty()
                || [&e.reason, &e.object, &e.kind, &e.message].iter().any(|field| field.to_lowercase().contains(&needle))
        })
        .collect();
    crate::sort::apply(&mut shown, sort, |e, column| crate::sort::event_key(e, column));
    shown
}

/// One row in the Events feed: every cluster Event in chronological
/// order (not filtered to Warnings — see `EventFilter` for how the
/// dedicated Events browser narrows that down), plus each node's own
/// problem conditions folded in as synthetic Warning-severity entries
/// (there's no Event object for "this node has been NotReady for an
/// hour," but it's exactly the kind of thing this feed should surface).
#[derive(Clone)]
pub struct EventEntry {
    pub message: String,
    pub reason: String,
    pub object: String,
    pub kind: String,
    pub age: String,
    pub age_secs: i64,
    pub severity: EventSeverity,
}

/// Node conditions worth surfacing: `Ready != True`, or any pressure/
/// unavailable condition that's `True`. A healthy node's only condition
/// is `Ready: True` — everything else here is inherently a problem
/// signal, unlike Pod phases which are fine in most states.
pub fn node_warnings(node: &Node) -> Vec<EventEntry> {
    let name = node.metadata.name.clone().unwrap_or_default();
    let conditions = node.status.as_ref().and_then(|s| s.conditions.clone()).unwrap_or_default();

    conditions
        .into_iter()
        .filter_map(|c| {
            let is_problem = match c.type_.as_str() {
                "Ready" => c.status != "True",
                _ => c.status == "True",
            };
            if !is_problem {
                return None;
            }
            let age = c
                .last_transition_time
                .as_ref()
                .map(|t| humanize_age(t.0))
                .unwrap_or_else(|| "-".into());
            let age_secs = c
                .last_transition_time
                .as_ref()
                .map(|t| k8s_openapi::jiff::Timestamp::now().as_second() - t.0.as_second())
                .unwrap_or(0);
            Some(EventEntry {
                message: c.message.unwrap_or_else(|| c.type_.clone()),
                reason: c.type_.clone(),
                object: name.clone(),
                kind: "Node".to_string(),
                age,
                age_secs,
                severity: EventSeverity::Warning,
            })
        })
        .collect()
}

/// Every cluster Event, Normal and Warning alike — unlike the old
/// warnings-only feed, this doesn't filter by type at all, so the Events
/// panel reflects what's actually happening on the cluster, not just what
/// went wrong.
pub fn event_entry(event: &Event) -> EventEntry {
    let severity = if event.type_.as_deref() == Some("Warning") { EventSeverity::Warning } else { EventSeverity::Normal };
    // For a repeated/aggregated event (the common case — the same
    // message firing over and over, e.g. a crash loop), `series.
    // lastObservedTime` is the real "last seen" moment. `eventTime`/
    // `lastTimestamp` only capture the *first* occurrence — using those
    // alone made an event from hours ago look current if it kept
    // recurring, which is exactly backwards for a "what's happening
    // right now" feed.
    let timestamp = event
        .series
        .as_ref()
        .and_then(|s| s.last_observed_time.as_ref())
        .map(|t| t.0)
        .or(event.last_timestamp.as_ref().map(|t| t.0))
        .or(event.event_time.as_ref().map(|t| t.0))
        .or(event.first_timestamp.as_ref().map(|t| t.0));
    let age = timestamp.map(humanize_age).unwrap_or_else(|| "-".into());
    let age_secs =
        timestamp.map(|t| k8s_openapi::jiff::Timestamp::now().as_second() - t.as_second()).unwrap_or(0);

    EventEntry {
        message: event.message.clone().unwrap_or_default(),
        reason: event.reason.clone().unwrap_or_default(),
        object: event.involved_object.name.clone().unwrap_or_default(),
        kind: event.involved_object.kind.clone().unwrap_or_default(),
        age,
        age_secs,
        severity,
    }
}

pub fn watch_events(client: Client) -> (reflector::Store<Event>, JoinHandle<()>) {
    let api: Api<Event> = Api::all(client);
    let (reader, writer) = reflector::store();
    let stream = watcher(api, watcher::Config::default()).default_backoff().reflect(writer).applied_objects();
    let handle = tokio::spawn(async move {
        let mut stream = stream.boxed();
        while stream.next().await.is_some() {}
    });
    (reader, handle)
}
