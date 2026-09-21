//! A watched kind kept sorted with each object's row built once: watch events change only
//! the entries they touch, and ages are refreshed where the shown value moves on.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kube::{Resource, runtime::reflector};

use super::watch::{Feed, Key};

/// A row that shows an age, so it can be kept current without rebuilding it.
pub trait AgeRow: Clone + Send + Sync + 'static {
    fn age_secs(&self) -> i64;
    fn set_age(&mut self, age: String, secs: i64);
}

/// An object with the row built from it.
pub type Item<K, R> = (Arc<K>, Arc<R>);

/// Ages are looked at this often.
const AGE_EVERY: Duration = Duration::from_secs(1);

pub struct Kept<K: Resource<DynamicType = ()> + 'static, R> {
    pub store: reflector::Store<K>,
    feed: Arc<Feed>,
    build: fn(&K) -> R,
    state: Mutex<State<K, R>>,
}

struct State<K, R> {
    items: Arc<Vec<Item<K, R>>>,
    aged: Instant,
}

/// The value the AGE column shows, so only rows where it changed are touched.
fn shown(secs: i64) -> (u8, i64) {
    match secs {
        ..60 => (0, secs),
        60..3600 => (1, secs / 60),
        3600..86400 => (2, secs / 3600),
        _ => (3, secs / 86400),
    }
}

fn order(meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta) -> (Option<&str>, &str) {
    (meta.namespace.as_deref(), meta.name.as_deref().unwrap_or_default())
}

impl<K, R> Kept<K, R>
where
    K: Resource<DynamicType = ()> + Clone + Send + Sync + 'static,
    R: AgeRow,
{
    pub fn new(store: reflector::Store<K>, feed: Arc<Feed>, build: fn(&K) -> R) -> Self {
        Kept { store, feed, build, state: Mutex::new(State { items: Arc::default(), aged: Instant::now() }) }
    }

    /// Everything, ordered by namespace then name. Cheap unless a watch reported something.
    pub fn items(&self) -> Arc<Vec<Item<K, R>>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match self.feed.take() {
            Some(keys) if !keys.is_empty() => {
                self.apply(&mut state, keys);
                // A safety net: the store and the list must agree on how many there are.
                if state.items.len() != self.store.len() {
                    self.rebuild(&mut state);
                }
            }
            Some(_) => {}
            None => self.rebuild(&mut state),
        }
        if state.aged.elapsed() >= AGE_EVERY {
            Self::age(&mut state);
        }
        Arc::clone(&state.items)
    }

    /// The objects alone, for readers that need no rows.
    pub fn objects(&self) -> Vec<Arc<K>> {
        self.items().iter().map(|(o, _)| Arc::clone(o)).collect()
    }

    fn rebuild(&self, state: &mut State<K, R>) {
        let objects = super::watch::sorted(&self.store);
        let build = self.build;
        let rows = super::par_map(&objects, |o| Arc::new(build(o)));
        state.items = Arc::new(objects.into_iter().zip(rows).collect());
        state.aged = Instant::now();
    }

    fn apply(&self, state: &mut State<K, R>, keys: Vec<Key>) {
        let items = Arc::make_mut(&mut state.items);
        for (namespace, name) in keys {
            let at = items.binary_search_by(|(o, _)| order(o.meta()).cmp(&(namespace.as_deref(), name.as_str())));
            let mut reference = reflector::ObjectRef::<K>::new(&name);
            if let Some(namespace) = &namespace {
                reference = reference.within(namespace);
            }
            match (at, self.store.get(&reference)) {
                (Ok(i), Some(object)) => {
                    if !Arc::ptr_eq(&items[i].0, &object) {
                        let row = Arc::new((self.build)(&object));
                        items[i] = (object, row);
                    }
                }
                (Ok(i), None) => {
                    items.remove(i);
                }
                (Err(i), Some(object)) => {
                    let row = Arc::new((self.build)(&object));
                    items.insert(i, (object, row));
                }
                (Err(_), None) => {}
            }
        }
    }

    /// Rewrites the age of the rows whose shown value moved on (a row nobody else holds is
    /// changed in place).
    fn age(state: &mut State<K, R>) {
        let now = k8s_openapi::jiff::Timestamp::now().as_second();
        let items = Arc::make_mut(&mut state.items);
        for (object, row) in items.iter_mut() {
            let Some(created) = object.meta().creation_timestamp.as_ref() else { continue };
            let secs = (now - created.0.as_second()).max(0);
            if shown(secs) != shown(row.age_secs()) {
                let age = super::humanize_age(created.0);
                Arc::make_mut(row).set_age(age, secs);
            }
        }
        state.aged = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::ConfigMap;
    use kube::{ResourceExt, runtime::watcher::Event};

    #[derive(Clone)]
    struct Row {
        name: String,
        age: String,
        secs: i64,
    }

    impl AgeRow for Row {
        fn age_secs(&self) -> i64 {
            self.secs
        }
        fn set_age(&mut self, age: String, secs: i64) {
            (self.age, self.secs) = (age, secs);
        }
    }

    fn map(namespace: &str, name: &str, data: &str) -> ConfigMap {
        serde_json::from_value(serde_json::json!({"metadata": {"name": name, "namespace": namespace, "creationTimestamp": "2020-01-01T00:00:00Z"}, "data": {"k": data}})).unwrap()
    }

    fn build(o: &ConfigMap) -> Row {
        Row { name: format!("{}={}", o.name_any(), o.data.as_ref().and_then(|d| d.get("k")).cloned().unwrap_or_default()), age: "-".into(), secs: 0 }
    }

    /// A kept list, and the store and queue a watch would feed.
    fn setup() -> (Kept<ConfigMap, Row>, reflector::store::Writer<ConfigMap>, Arc<Feed>) {
        let (store, writer) = reflector::store::<ConfigMap>();
        let feed = Arc::new(Feed::default());
        (Kept::new(store, Arc::clone(&feed), build), writer, feed)
    }

    fn send(writer: &mut reflector::store::Writer<ConfigMap>, feed: &Feed, event: Event<ConfigMap>) {
        writer.apply_watcher_event(&event);
        feed.note(&event);
    }

    fn names(kept: &Kept<ConfigMap, Row>) -> Vec<String> {
        kept.items().iter().map(|(_, r)| r.name.clone()).collect()
    }

    #[test]
    fn events_insert_replace_and_remove_in_order() {
        let (kept, mut writer, feed) = setup();
        send(&mut writer, &feed, Event::Apply(map("b", "x", "1")));
        assert_eq!(names(&kept), ["x=1"]);
        send(&mut writer, &feed, Event::Apply(map("a", "y", "1")));
        send(&mut writer, &feed, Event::Apply(map("b", "w", "1")));
        assert_eq!(names(&kept), ["y=1", "w=1", "x=1"]);
        send(&mut writer, &feed, Event::Apply(map("b", "w", "2")));
        assert_eq!(names(&kept), ["y=1", "w=2", "x=1"]);
        send(&mut writer, &feed, Event::Delete(map("a", "y", "1")));
        assert_eq!(names(&kept), ["w=2", "x=1"]);
    }

    #[test]
    fn only_the_changed_object_is_rebuilt() {
        let (kept, mut writer, feed) = setup();
        for i in 0..5 {
            send(&mut writer, &feed, Event::Apply(map("n", &format!("c{i}"), "1")));
        }
        let before = kept.items();
        send(&mut writer, &feed, Event::Apply(map("n", "c2", "2")));
        let after = kept.items();
        for i in [0, 1, 3, 4] {
            assert!(Arc::ptr_eq(&before[i].1, &after[i].1), "row {i} was rebuilt");
        }
        assert!(!Arc::ptr_eq(&before[2].1, &after[2].1));
    }

    #[test]
    fn a_relist_rebuilds_everything() {
        let (kept, mut writer, feed) = setup();
        send(&mut writer, &feed, Event::Apply(map("n", "a", "1")));
        assert_eq!(names(&kept), ["a=1"]);
        for event in [Event::Init, Event::InitApply(map("n", "b", "1")), Event::InitDone] {
            send(&mut writer, &feed, event);
        }
        assert_eq!(names(&kept), ["b=1"]);
    }

    #[test]
    fn ages_move_on_without_a_watch_event() {
        let (kept, mut writer, feed) = setup();
        send(&mut writer, &feed, Event::Apply(map("n", "a", "1")));
        kept.items();
        kept.state.lock().unwrap().aged = Instant::now() - AGE_EVERY;
        let row = Arc::clone(&kept.items()[0].1);
        assert!(row.secs > 86400 * 365 && row.age.ends_with('d'));
    }
}
