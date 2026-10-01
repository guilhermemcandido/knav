//! Sorting the resource lists by column: which key each column sorts on,
//! and applying it. Column numbers are 0-based, in table header order.

use crate::ResourceKind;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortSpec {
    pub column: usize,
    pub descending: bool,
}

impl SortSpec {
    /// Pressing a column's number: sort by it ascending, or flip it if it is already
    /// the sort column.
    pub fn pressed(current: Option<SortSpec>, column: usize) -> SortSpec {
        match current {
            Some(s) if s.column == column => SortSpec { column, descending: !s.descending },
            _ => SortSpec { column, descending: false },
        }
    }
}

/// One cell's sort value. A column only ever produces one variant.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Num(i64),
    Text(String),
}

fn text(s: &str) -> Key {
    Key::Text(s.to_lowercase())
}

pub const POD_COLUMNS: usize = 12;
const DEPLOYMENT_COLUMNS: usize = 6;
const NODE_COLUMNS: usize = 9;
const CRD_COLUMNS: usize = 4;

/// How many sortable columns the list for `kind` has. `generic_columns` is the
/// generic table's width.
pub fn column_count(kind: ResourceKind, generic_columns: usize, wide: bool) -> usize {
    match kind {
        ResourceKind::Overview => 0,
        ResourceKind::Pods => POD_COLUMNS + if wide { 2 } else { 0 },
        ResourceKind::Deployments => DEPLOYMENT_COLUMNS + if wide { 2 } else { 0 },
        ResourceKind::Nodes => NODE_COLUMNS + if wide { 4 } else { 0 },
        ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => CRD_COLUMNS,
        _ => generic_columns,
    }
}

/// The AGE column of the list for `kind`, if it has one (for `A`).
pub fn age_column(kind: ResourceKind, generic_columns: usize, wide: bool) -> Option<usize> {
    match kind {
        ResourceKind::Overview | ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => None,
        ResourceKind::Pods => Some(5),
        ResourceKind::Deployments => Some(5),
        ResourceKind::Nodes => Some(7),
        // Namespace (if any), name, the kind's columns, then AGE, then LABELS when wide.
        _ => generic_columns.checked_sub(1 + usize::from(wide)),
    }
}

/// Sorts `items` by `spec` (a no-op when `None`), computing each key once.
pub fn apply<T>(items: &mut [T], spec: Option<SortSpec>, key: impl Fn(&T, usize) -> Key) {
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

/// `usage` gives CPU and MEM; pods without metrics sort below every measured one.
pub fn pod_key(row: &crate::PodRow, column: usize, wide: bool, usage: Option<&crate::metrics::PodUsageMap>) -> Key {
    let used = || usage.and_then(|u| u.get(&row.namespace, &row.name));
    match column {
        0 => text(&row.namespace),
        1 => text(&row.name),
        2 => Key::Num(ready_fraction(&row.ready)),
        3 => text(&row.phase),
        4 => Key::Num(i64::from(row.restarts)),
        5 => Key::Num(row.age_secs),
        6 => Key::Num(used().map_or(-1, |u| u.cpu_millicores)),
        7 => Key::Num(used().map_or(-1, |u| u.memory_bytes)),
        8 => text(&row.controlled_by),
        9 => text(&row.node),
        10 => text(&row.qos),
        11 if wide => text(&row.ip),
        12 if wide => text(&row.images),
        _ => Key::Num(row.containers.len() as i64),
    }
}

pub fn deployment_key(row: &crate::DeploymentRow, column: usize, wide: bool) -> Key {
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

pub fn node_key(row: &crate::NodeRow, column: usize, wide: bool) -> Key {
    match column {
        0 => text(&row.name),
        // NotReady, then cordoned, then ready: problems first.
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

pub fn generic_key(row: &crate::GenericRow, column: usize, has_namespace: bool) -> Key {
    // Without a namespace column, the others shift left by one.
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

pub fn crd_key(crd: &crate::CrdInfo, count: crate::Count, column: usize) -> Key {
    match column {
        0 => text(crd.group),
        1 => text(crd.kind),
        2 => Key::Num(count.sort_key()),
        _ => Key::Num(i64::from(crd.namespaced)),
    }
}

pub const EVENT_COLUMNS: usize = 6;
pub const CONTAINER_COLUMNS: usize = 4;
pub const NAMESPACE_PICKER_COLUMNS: usize = 2;

pub fn event_key(e: &crate::EventEntry, column: usize) -> Key {
    match column {
        0 => Key::Num(match e.severity {
            crate::EventSeverity::Warning => 0,
            crate::EventSeverity::Normal => 1,
        }),
        1 => text(&e.reason),
        2 => text(&e.object),
        3 => text(&e.kind),
        4 => text(&e.message),
        _ => Key::Num(e.age_secs),
    }
}

pub fn container_key(c: &crate::ContainerInfo, column: usize) -> Key {
    match column {
        // The status dot: problems first.
        0 => Key::Num(match c.status {
            crate::ContainerStatusKind::Unknown => 0,
            crate::ContainerStatusKind::Terminated => 1,
            crate::ContainerStatusKind::Waiting => 2,
            crate::ContainerStatusKind::Running => 3,
        }),
        1 => text(&c.name),
        2 => text(c.reason.as_deref().unwrap_or("")),
        _ => Key::Num(i64::from(c.restarts)),
    }
}

pub fn namespace_key(name: &str, key: Option<usize>, column: usize) -> Key {
    match column {
        0 => text(name),
        // Namespaces without a key last.
        _ => Key::Num(key.map(|k| k as i64).unwrap_or(99)),
    }
}

pub fn sorted_containers(containers: &[crate::ContainerInfo], spec: Option<SortSpec>) -> Vec<crate::ContainerInfo> {
    let mut sorted = containers.to_vec();
    apply(&mut sorted, spec, container_key);
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let row = crate::GenericRow {
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
        use crate::describe::{Col, Tone};
        let row = crate::GenericRow {
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

#[cfg(test)]
mod age_tests {
    use super::*;

    #[test]
    fn each_list_knows_where_its_age_column_is() {
        assert_eq!(age_column(ResourceKind::Pods, 0, false), Some(5));
        assert_eq!(age_column(ResourceKind::Deployments, 0, false), Some(5));
        assert_eq!(age_column(ResourceKind::Nodes, 0, false), Some(7));
        // Namespace, name, two kind columns, age: index 4; wide adds LABELS after it.
        assert_eq!(age_column(ResourceKind::Services, 5, false), Some(4));
        assert_eq!(age_column(ResourceKind::Services, 6, true), Some(4));
        assert_eq!(age_column(ResourceKind::Overview, 0, false), None);
    }
}
