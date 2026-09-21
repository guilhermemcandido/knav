//! Sorting the resource lists by column: which key each column sorts on,
//! and applying it. Column numbers here (0-based) match the order of the
//! table headers in `ui::tables`, which shows them as `(1)NAME`, ...

use crate::*;

/// The column a list is sorted by and which way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SortSpec {
    pub(crate) column: usize,
    pub(crate) descending: bool,
}

impl SortSpec {
    /// What pressing a column's number does: sort by it ascending; pressing
    /// the same column again flips it, ascending <-> descending. A
    /// different column starts ascending.
    pub(crate) fn pressed(current: Option<SortSpec>, column: usize) -> SortSpec {
        match current {
            Some(s) if s.column == column => SortSpec { column, descending: !s.descending },
            _ => SortSpec { column, descending: false },
        }
    }
}

/// Sort state for a popup table (Events, Containers, the pickers, ...):
/// the column/direction and whether sort mode (`s`) is on. The main lists
/// keep theirs in `run` directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ListSort {
    pub(crate) spec: Option<SortSpec>,
    pub(crate) choosing: bool,
}

impl ListSort {
    pub(crate) fn view(self) -> ui::SortState {
        ui::SortState { column: self.spec.map(|s| s.column), descending: self.spec.is_some_and(|s| s.descending), choosing: self.choosing, cursor: None }
    }

    /// Feeds it a key; `true` if it was a sort key (`s` to enter sort
    /// mode; then digits, and `s`/Esc/`q` to leave). `typing` means a text
    /// field has focus, so every key is text.
    pub(crate) fn handle(&mut self, code: KeyCode, columns: usize, typing: bool) -> bool {
        if typing || columns == 0 {
            return false;
        }
        if !self.choosing {
            if code == KeyCode::Char('s') {
                self.choosing = true;
                return true;
            }
            return false;
        }
        match code {
            KeyCode::Char(c @ '0'..='9') => {
                // The digits are columns 0-9.
                let column = c as usize - '0' as usize;
                if column < columns {
                    self.spec = Some(SortSpec::pressed(self.spec, column));
                }
                true
            }
            KeyCode::Char('s' | 'q') | KeyCode::Esc => {
                self.choosing = false;
                true
            }
            _ => false,
        }
    }
}

/// One cell's sort value. A column only ever produces one variant, so
/// comparing across variants never happens.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Num(i64),
    Text(String),
}

fn text(s: &str) -> Key {
    Key::Text(s.to_lowercase())
}

pub(crate) const POD_COLUMNS: usize = 10;
const DEPLOYMENT_COLUMNS: usize = 6;
const NODE_COLUMNS: usize = 9;
const CRD_COLUMNS: usize = 4;

/// How many sortable columns the list for `kind` has. `generic_columns` is
/// the current generic table's width (namespace if any, name, the kind's
/// own columns, age).
pub(crate) fn column_count(kind: ResourceKind, generic_columns: usize, wide: bool) -> usize {
    match kind {
        ResourceKind::Overview => 0,
        ResourceKind::Pods => POD_COLUMNS + if wide { 2 } else { 0 },
        ResourceKind::Deployments => DEPLOYMENT_COLUMNS + usize::from(wide),
        ResourceKind::Nodes => NODE_COLUMNS + if wide { 4 } else { 0 },
        ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => CRD_COLUMNS,
        _ => generic_columns,
    }
}

/// Sorts `items` by `spec` (a no-op when `None`), computing each key once.
pub(crate) fn apply<T>(items: &mut [T], spec: Option<SortSpec>, key: impl Fn(&T, usize) -> Key) {
    let Some(spec) = spec else { return };
    items.sort_by_cached_key(|item| key(item, spec.column));
    if spec.descending {
        items.reverse();
    }
}

/// `"2/3"` as a fraction in thousandths, so 3/3 sorts above 2/3 above 0/3.
fn ready_fraction(ready: &str) -> i64 {
    let mut parts = ready.split('/').filter_map(|p| p.parse::<i64>().ok());
    match (parts.next(), parts.next()) {
        (Some(have), Some(total)) if total > 0 => have * 1000 / total,
        _ => 0,
    }
}

pub(crate) fn pod_key(row: &k8s::PodRow, column: usize, wide: bool) -> Key {
    match column {
        0 => text(&row.namespace),
        1 => text(&row.name),
        2 => Key::Num(ready_fraction(&row.ready)),
        3 => text(&row.phase),
        4 => Key::Num(i64::from(row.restarts)),
        5 => text(&row.controlled_by),
        6 => text(&row.node),
        7 => text(&row.qos),
        8 => Key::Num(row.age_secs),
        9 if wide => text(&row.ip),
        10 if wide => text(&row.images),
        _ => Key::Num(row.containers.len() as i64),
    }
}

pub(crate) fn deployment_key(row: &k8s::DeploymentRow, column: usize, wide: bool) -> Key {
    match column {
        0 => text(&row.namespace),
        1 => text(&row.name),
        2 => Key::Num(ready_fraction(&row.ready)),
        3 => Key::Num(i64::from(row.up_to_date)),
        4 => Key::Num(i64::from(row.available)),
        6 if wide => text(&row.images),
        _ => Key::Num(row.age_secs),
    }
}

pub(crate) fn node_key(row: &k8s::NodeRow, column: usize, wide: bool) -> Key {
    match column {
        0 => text(&row.name),
        // NotReady, then cordoned, then ready, the order you want to see problems in.
        1 => Key::Num(match (row.ready, row.schedulable) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        }),
        2 => text(&row.roles),
        3 => Key::Num(row.taints as i64),
        4 => Key::Num(row.cpu_millicores.unwrap_or(-1)),
        5 => Key::Num(row.memory_bytes.unwrap_or(-1)),
        6 => Key::Num(row.pod_count as i64),
        7 => Key::Num(row.age_secs),
        9 if wide => text(&row.internal_ip),
        10 if wide => text(&row.os_image),
        11 if wide => text(&row.kernel),
        12 if wide => text(&row.runtime),
        _ => text(&row.version),
    }
}

pub(crate) fn generic_key(row: &k8s::GenericRow, column: usize, has_namespace: bool) -> Key {
    // Without the namespace column, everything shifts left by one:
    // namespace, name, the kind's own columns, age.
    let column = column + usize::from(!has_namespace);
    match column {
        0 => text(&row.namespace),
        1 => text(&row.name),
        c if c - 2 < row.extras.len() => {
            let extra = &row.extras[c - 2];
            extra.sort.map_or_else(|| text(&extra.text), Key::Num)
        }
        c if c == row.extras.len() + 2 => Key::Num(row.age_secs),
        // Past AGE: the wide view's LABELS.
        _ => text(&row.labels),
    }
}

pub(crate) fn crd_key(crd: &k8s::CrdInfo, count: k8s::Count, column: usize) -> Key {
    match column {
        0 => text(crd.group),
        1 => text(crd.kind),
        2 => Key::Num(count.sort_key()),
        _ => Key::Num(i64::from(crd.namespaced)),
    }
}

pub(crate) const EVENT_COLUMNS: usize = 6;
pub(crate) const CONTAINER_COLUMNS: usize = 4;
pub(crate) const CONTEXT_COLUMNS: usize = 3;
pub(crate) const NAMESPACE_PICKER_COLUMNS: usize = 2;

pub(crate) fn event_key(e: &k8s::EventEntry, column: usize) -> Key {
    match column {
        0 => Key::Num(match e.severity {
            k8s::EventSeverity::Warning => 0,
            k8s::EventSeverity::Normal => 1,
        }),
        1 => text(&e.reason),
        2 => text(&e.object),
        3 => text(&e.kind),
        4 => text(&e.message),
        _ => Key::Num(e.age_secs),
    }
}

pub(crate) fn container_key(c: &k8s::ContainerInfo, column: usize) -> Key {
    match column {
        // The status dot: problems first.
        0 => Key::Num(match c.status {
            k8s::ContainerStatusKind::Unknown => 0,
            k8s::ContainerStatusKind::Terminated => 1,
            k8s::ContainerStatusKind::Waiting => 2,
            k8s::ContainerStatusKind::Running => 3,
        }),
        1 => text(&c.name),
        2 => text(c.reason.as_deref().unwrap_or("")),
        _ => Key::Num(i64::from(c.restarts)),
    }
}

pub(crate) fn context_key(c: &k8s::ContextInfo, column: usize) -> Key {
    match column {
        0 => text(&c.name),
        1 => text(&c.cluster),
        _ => Key::Num(i64::from(!c.is_current)),
    }
}

pub(crate) fn namespace_key(name: &str, key: Option<usize>, column: usize) -> Key {
    match column {
        0 => text(name),
        // Namespaces without a key last.
        _ => Key::Num(key.map(|k| k as i64).unwrap_or(99)),
    }
}

/// `containers` in display order for `sort`.
pub(crate) fn sorted_containers(containers: &[k8s::ContainerInfo], sort: ListSort) -> Vec<k8s::ContainerInfo> {
    let mut sorted = containers.to_vec();
    apply(&mut sorted, sort.spec, container_key);
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_sort_mode_enters_with_s_cycles_digits_and_leaves() {
        let mut sort = ListSort::default();
        assert!(!sort.handle(KeyCode::Char('j'), 4, false), "other keys are not ours outside the mode");
        assert!(sort.handle(KeyCode::Char('s'), 4, false));
        assert!(sort.choosing);
        assert!(sort.handle(KeyCode::Char('2'), 4, false));
        assert_eq!(sort.spec, Some(SortSpec { column: 2, descending: false }));
        assert!(sort.handle(KeyCode::Char('2'), 4, false));
        assert_eq!(sort.spec, Some(SortSpec { column: 2, descending: true }));
        // Past the last column: consumed, but nothing changes.
        assert!(sort.handle(KeyCode::Char('9'), 4, false));
        assert_eq!(sort.spec, Some(SortSpec { column: 2, descending: true }));
        assert!(sort.choosing);
        assert!(sort.handle(KeyCode::Esc, 4, false));
        assert!(!sort.choosing);
        assert_eq!(sort.spec, Some(SortSpec { column: 2, descending: true }), "leaving keeps the sort");
    }

    #[test]
    fn popup_sort_ignores_keys_while_typing() {
        let mut sort = ListSort::default();
        assert!(!sort.handle(KeyCode::Char('s'), 4, true));
        assert!(!sort.choosing);
    }

    fn sorted(spec: Option<SortSpec>) -> Vec<i64> {
        let mut v = vec![3, 1, 2];
        apply(&mut v, spec, |n, _| Key::Num(*n));
        v
    }

    #[test]
    fn pressing_a_column_toggles_ascending_and_descending() {
        let asc = SortSpec::pressed(None, 2);
        assert_eq!(asc, SortSpec { column: 2, descending: false });
        let desc = SortSpec::pressed(Some(asc), 2);
        assert_eq!(desc, SortSpec { column: 2, descending: true });
        assert_eq!(SortSpec::pressed(Some(desc), 2), asc, "back to ascending, never off");
        // A different column starts over ascending.
        assert_eq!(SortSpec::pressed(Some(desc), 4), SortSpec { column: 4, descending: false });
    }

    #[test]
    fn apply_sorts_ascending_descending_and_leaves_none_alone() {
        assert_eq!(sorted(None), [3, 1, 2]);
        assert_eq!(sorted(Some(SortSpec { column: 0, descending: false })), [1, 2, 3]);
        assert_eq!(sorted(Some(SortSpec { column: 0, descending: true })), [3, 2, 1]);
    }

    #[test]
    fn ready_fraction_orders_fuller_pods_higher() {
        assert!(ready_fraction("3/3") > ready_fraction("2/3"));
        assert!(ready_fraction("2/3") > ready_fraction("0/3"));
        assert_eq!(ready_fraction("0/0"), 0);
        assert_eq!(ready_fraction("junk"), 0);
    }

    #[test]
    fn text_keys_ignore_case() {
        assert!(text("Alpha") < text("beta"));
    }

    #[test]
    fn generic_columns_shift_left_without_a_namespace() {
        let row = k8s::GenericRow {
            namespace: "ns".into(),
            name: "web".into(),
            age: "1d".into(),
            age_secs: 86400,
            extras: Vec::new(),
            status: None,
            uid: String::new(),
            owners: Vec::new(),
            labels: String::new(),
        };
        assert_eq!(generic_key(&row, 0, true), Key::Text("ns".into()));
        assert_eq!(generic_key(&row, 0, false), Key::Text("web".into()));
        assert_eq!(generic_key(&row, 1, false), Key::Num(86400));
    }

    #[test]
    fn a_kinds_own_columns_sort_between_name_and_age_numerically_when_they_can() {
        use crate::k8s::describe::{Col, Tone};
        let row = k8s::GenericRow {
            namespace: "ns".into(),
            name: "web".into(),
            age: "1d".into(),
            age_secs: 5,
            extras: vec![
                Col { header: "READY", text: "10".into(), tone: Tone::Good, sort: Some(10) },
                Col { header: "TYPE", text: "Opaque".into(), tone: Tone::Plain, sort: None },
            ],
            status: None,
            uid: String::new(),
            owners: Vec::new(),
            labels: String::new(),
        };
        assert_eq!(generic_key(&row, 2, true), Key::Num(10), "numeric, so 10 sorts after 9");
        assert_eq!(generic_key(&row, 3, true), Key::Text("opaque".into()));
        assert_eq!(generic_key(&row, 4, true), Key::Num(5), "age is last");
    }

    #[test]
    fn column_counts_match_the_tables() {
        assert_eq!(column_count(ResourceKind::Pods, 0, false), POD_COLUMNS);
        assert_eq!(column_count(ResourceKind::Pods, 0, true), POD_COLUMNS + 2);
        assert_eq!(column_count(ResourceKind::ConfigMaps, 5, false), 5);
        assert_eq!(column_count(ResourceKind::Nodes, 0, false), NODE_COLUMNS);
        assert_eq!(column_count(ResourceKind::Overview, 3, true), 0);
    }
}
