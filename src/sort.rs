//! Sorting the resource lists by column: which key each column sorts on,
//! and applying it. Column numbers here (0-based) match the order of the
//! table headers in `ui::tables`, which shows them as `(1)NAME`, ...

use super::*;

/// The column a list is sorted by and which way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SortSpec {
    pub(crate) column: usize,
    pub(crate) descending: bool,
}

impl SortSpec {
    /// What pressing a column's number does, IDE-style: unsorted -> ascending
    /// -> descending -> unsorted; pressing a different column starts it ascending.
    pub(crate) fn pressed(current: Option<SortSpec>, column: usize) -> Option<SortSpec> {
        match current {
            Some(s) if s.column == column && !s.descending => Some(SortSpec { column, descending: true }),
            Some(s) if s.column == column => None,
            _ => Some(SortSpec { column, descending: false }),
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

const POD_COLUMNS: usize = 8;
const DEPLOYMENT_COLUMNS: usize = 6;
const NODE_COLUMNS: usize = 8;
const CRD_COLUMNS: usize = 3;

/// How many sortable columns the list for `kind` has. Generic tables drop
/// the namespace column when every row is cluster-scoped.
pub(crate) fn column_count(kind: ResourceKind, generic_has_namespace: bool) -> usize {
    match kind {
        ResourceKind::Overview => 0,
        ResourceKind::Pods => POD_COLUMNS,
        ResourceKind::Deployments => DEPLOYMENT_COLUMNS,
        ResourceKind::Nodes => NODE_COLUMNS,
        ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => CRD_COLUMNS,
        _ => 2 + usize::from(generic_has_namespace),
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

pub(crate) fn pod_key(row: &k8s::PodRow, column: usize) -> Key {
    match column {
        0 => text(&row.namespace),
        1 => text(&row.name),
        2 => Key::Num(ready_fraction(&row.ready)),
        3 => text(&row.phase),
        4 => Key::Num(i64::from(row.restarts)),
        5 => text(&row.node),
        6 => Key::Num(row.age_secs),
        _ => Key::Num(row.containers.len() as i64),
    }
}

pub(crate) fn deployment_key(row: &k8s::DeploymentRow, column: usize) -> Key {
    match column {
        0 => text(&row.namespace),
        1 => text(&row.name),
        2 => Key::Num(ready_fraction(&row.ready)),
        3 => Key::Num(i64::from(row.up_to_date)),
        4 => Key::Num(i64::from(row.available)),
        _ => Key::Num(row.age_secs),
    }
}

pub(crate) fn node_key(row: &k8s::NodeRow, column: usize) -> Key {
    match column {
        0 => text(&row.name),
        // NotReady, then cordoned, then ready — the order you want to see problems in.
        1 => Key::Num(match (row.ready, row.schedulable) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        }),
        2 => text(&row.roles),
        3 => Key::Num(row.cpu_millicores.unwrap_or(-1)),
        4 => Key::Num(row.memory_bytes.unwrap_or(-1)),
        5 => Key::Num(row.pod_count as i64),
        6 => Key::Num(row.age_secs),
        _ => text(&row.version),
    }
}

pub(crate) fn generic_key(row: &k8s::GenericRow, column: usize, has_namespace: bool) -> Key {
    // Without the namespace column, everything shifts left by one.
    match column + usize::from(!has_namespace) {
        0 => text(&row.namespace),
        1 => text(&row.name),
        _ => Key::Num(row.age_secs),
    }
}

pub(crate) fn crd_key(crd: &k8s::CrdInfo, column: usize) -> Key {
    match column {
        0 => text(crd.group),
        1 => text(crd.kind),
        _ => Key::Num(i64::from(crd.namespaced)),
    }
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
    fn pressing_a_column_cycles_ascending_descending_off() {
        let asc = SortSpec::pressed(None, 2);
        assert_eq!(asc, Some(SortSpec { column: 2, descending: false }));
        let desc = SortSpec::pressed(asc, 2);
        assert_eq!(desc, Some(SortSpec { column: 2, descending: true }));
        assert_eq!(SortSpec::pressed(desc, 2), None);
        // A different column starts over ascending.
        assert_eq!(SortSpec::pressed(desc, 4), Some(SortSpec { column: 4, descending: false }));
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
            uid: String::new(),
            owners: Vec::new(),
        };
        assert_eq!(generic_key(&row, 0, true), Key::Text("ns".into()));
        assert_eq!(generic_key(&row, 0, false), Key::Text("web".into()));
        assert_eq!(generic_key(&row, 1, false), Key::Num(86400));
    }

    #[test]
    fn column_counts_match_the_tables() {
        assert_eq!(column_count(ResourceKind::Pods, false), 8);
        assert_eq!(column_count(ResourceKind::ConfigMaps, true), 3);
        assert_eq!(column_count(ResourceKind::Nodes, false), 8);
        assert_eq!(column_count(ResourceKind::ClusterRoles, false), 2);
        assert_eq!(column_count(ResourceKind::Overview, false), 0);
    }
}
