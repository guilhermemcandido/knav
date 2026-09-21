//! Per-frame derivation: what each list shows this instant, from the live
//! stores, the namespace/scope/search filters and the sort.

use super::*;
use crate::k8s::describe::Tone;
use std::sync::Arc;

/// Everything the frame and the input handlers read, recomputed every
/// iteration of the loop.
pub(super) struct Derived {
    pub pods: Vec<Arc<Pod>>,
    pub pod_rows: Vec<k8s::PodRow>,
    pub deployments: Vec<Arc<Deployment>>,
    pub dep_rows: Vec<k8s::DeploymentRow>,
    pub nodes: Vec<Arc<Node>>,
    pub usage: Option<metrics::ClusterUsage>,
    pub node_detail_pods: Vec<Arc<Pod>>,
    pub node_detail_rows: Vec<k8s::PodRow>,
    pub sorted_nodes: Vec<Arc<Node>>,
    pub node_rows: Vec<k8s::NodeRow>,
    pub overview: k8s::Overview,
    pub generic_headers: Vec<&'static str>,
    pub generic_rows_full: Vec<k8s::GenericRow>,
    pub generic_visible: Vec<usize>,
    pub generic_columns: usize,
    pub generic_rows: Vec<k8s::GenericRow>,
    pub crd_rows: Vec<(usize, k8s::CrdInfo)>,
    /// How many objects each CRD kind in `crd_rows` has.
    pub crd_counts: Vec<k8s::Count>,
}

/// The filters/ordering applied to every list.
pub(super) struct Query<'a> {
    pub current_kind: ResourceKind,
    pub namespace: Option<&'a str>,
    pub scope: Option<&'a Scope>,
    pub search: &'a str,
    pub sort: Option<SortSpec>,
    /// Show only rows that need a look (`Ctrl-z`).
    pub faults: bool,
    /// Show the extra columns (`Ctrl-w`), which the sort keys must know about.
    pub wide: bool,
    /// The Overview's category order and hidden entries.
    pub layout: &'a crate::config::OverviewConfig,
}

pub(super) struct Sources<'a> {
    pub pod_store: &'a Store<Pod>,
    pub dep_store: &'a Store<Deployment>,
    pub node_store: &'a Store<Node>,
    pub event_store: &'a Store<k8s_openapi::api::core::v1::Event>,
    pub node_metrics_rx: &'a watch::Receiver<Option<metrics::ClusterUsage>>,
    pub client: &'a Client,
    /// Rows for the Port-forwards list (knav's own, not from the cluster).
    pub forwards: &'a [k8s::GenericRow],
}

pub(super) fn derive(src: &Sources, catalog: &mut Catalog, mode: &Mode, q: &Query) -> Derived {
    let Sources { pod_store, dep_store, node_store, event_store, node_metrics_rx, client, forwards } = *src;
    let Query { current_kind, namespace, scope, search, sort, faults, wide, layout } = *q;
    let namespace = namespace.map(str::to_string);
    let search = search.to_string();
        // `search` only applies to the kind it was typed against (it is cleared on every
        // kind switch), so filtering every source list by it is safe.
        // The Overview stays cluster-wide even with a namespace set.
        let ns_filter: Option<&str> = if current_kind == ResourceKind::Overview { None } else { namespace.as_deref() };
        let in_namespace = |meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta| {
            ns_filter.is_none_or(|ns| meta.namespace.as_deref() == Some(ns))
        };
        let all_pods = k8s::snapshot(pod_store);
        let pods: Vec<std::sync::Arc<Pod>> = all_pods
            .iter()
            .cloned()
            .filter(|p| in_namespace(&p.metadata))
            .filter(|p| current_kind != ResourceKind::Pods || scope.is_none_or(|s| s.matches_meta(&p.metadata)))
            .filter(|p| meta_matches(&search, &p.metadata))
            .filter(|p| !(faults && current_kind == ResourceKind::Pods) || k8s::pod_is_fault(p))
            .collect();
        // Rows are built once, across the cores, and the sort reads them instead of rebuilding.
        let mut pairs: Vec<(std::sync::Arc<Pod>, k8s::PodRow)> = k8s::par_map(&pods, |p| (p.clone(), k8s::row_for(p)));
        if current_kind == ResourceKind::Pods {
            apply(&mut pairs, sort, |(_, row), column| pod_key(row, column, wide));
        }
        let (pods, pod_rows): (Vec<std::sync::Arc<Pod>>, Vec<k8s::PodRow>) = pairs.into_iter().unzip();
        let mut deployments: Vec<std::sync::Arc<Deployment>> = k8s::snapshot_deployments(dep_store)
            .into_iter()
            .filter(|d| in_namespace(&d.metadata))
            .filter(|d| meta_matches(&search, &d.metadata))
            .filter(|d| !(faults && current_kind == ResourceKind::Deployments) || k8s::ready_is_short(&k8s::row_for_deployment(d).ready))
            .collect();
        if current_kind == ResourceKind::Deployments {
            apply(&mut deployments, sort, |d, column| deployment_key(&k8s::row_for_deployment(d), column, wide));
        }
        let dep_rows: Vec<k8s::DeploymentRow> = k8s::par_map(&deployments, |d| k8s::row_for_deployment(d));
        let nodes = node_store.state();
        let events = event_store.state();
        let usage = node_metrics_rx.borrow().clone();
        // Only filled while a node's detail is open: the pods scheduled on that node.
        // Searches the whole back-chain so it also works when NodeDetail is a dimmed
        // background layer.
        let mut node_detail_pods: Vec<std::sync::Arc<Pod>> = if let Some(name) = node_detail_name(mode) {
            pods.iter().filter(|p| p.spec.as_ref().and_then(|s| s.node_name.as_deref()) == Some(name)).cloned().collect()
        } else {
            Vec::new()
        };
        let node_search = node_detail_search(mode).to_string();
        node_detail_pods.retain(|p| meta_matches(&node_search, &p.metadata));
        apply(&mut node_detail_pods, node_detail_sort(mode), |p, column| pod_key(&k8s::row_for(p), column, false));
        let node_detail_rows: Vec<k8s::PodRow> = node_detail_pods.iter().map(|p| k8s::row_for(p)).collect();
        // Nodes have their own rows (CPU/Memory in the list). They are filtered here so
        // the handlers indexing into `sorted_nodes` match what is displayed.
        // Every pod counts toward its node's PODS, whatever the list is narrowed to.
        let mut pods_per_node: HashMap<&str, usize> = HashMap::new();
        for pod in &all_pods {
            if let Some(node) = pod.spec.as_ref().and_then(|s| s.node_name.as_deref()) {
                *pods_per_node.entry(node).or_default() += 1;
            }
        }
        let mut node_pairs: Vec<(std::sync::Arc<Node>, k8s::NodeRow)> = k8s::snapshot_generic(node_store)
            .into_iter()
            .filter(|n| row_matches(&search, &n.metadata.name.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|n| {
                let name = n.metadata.name.clone().unwrap_or_default();
                let node_usage = usage.as_ref().and_then(|u| u.for_node(&name));
                let pod_count = pods_per_node.get(name.as_str()).copied().unwrap_or(0);
                let row = k8s::node_row(&n, node_usage, pod_count);
                (n, row)
            })
            .collect();
        let mut node_health = k8s::Health::default();
        for (_, row) in &node_pairs {
            node_health.add(if !row.ready { Tone::Bad } else if !row.schedulable { Tone::Warn } else { Tone::Good });
        }
        if faults && current_kind == ResourceKind::Nodes {
            node_pairs.retain(|(_, row)| !row.ready || !row.schedulable);
        }
        if current_kind == ResourceKind::Nodes {
            apply(&mut node_pairs, sort, |(_, row), column| node_key(row, column, wide));
        }
        let (sorted_nodes, node_rows): (Vec<std::sync::Arc<Node>>, Vec<k8s::NodeRow>) = node_pairs.into_iter().unzip();
        let catalog_sections = k8s::layout::arrange(catalog.sections(pod_rows.len(), dep_rows.len()), layout);
        // Only the opened-up category view shows it, so only work it out then.
        let health = if let Mode::ColumnDetail { col, .. } = mode {
            // Health needs the objects themselves, so start watching this category's kinds.
            for (label, _) in catalog_sections.get(*col).map(|(_, items)| items.as_slice()).unwrap_or(&[]) {
                catalog.ensure_label(label);
            }
            catalog.health([("Pods", k8s::pods_health(&pod_rows)), ("Deployments", k8s::deployments_health(&dep_rows)), ("Nodes", node_health)])
        } else {
            Default::default()
        };
        let report = matches!(mode, Mode::ResourcesDetail).then(|| k8s::report::report(&all_pods));
        let overview = k8s::overview(&nodes, &events, usage.as_ref(), catalog_sections, health, report);
        // Only filled for the kind on screen. `resolve` starts a CRD's watch the first
        // time it is opened. `generic_visible` maps a display position back to the real
        // index that `CatalogKind::spec_at` needs. Table-backed kinds add wide columns on request.
    if let Some(kind) = catalog.resolve(current_kind, client) {
        kind.set_wide(wide);
        kind.set_namespace(ns_filter);
    }
    let generic_headers: Vec<&'static str> = if current_kind == ResourceKind::PortForwards {
        portforward::HEADERS.to_vec()
    } else {
        catalog.resolve(current_kind, client).map(|k| k.headers()).unwrap_or_default()
    };
        let generic_rows_full: Vec<k8s::GenericRow> = if current_kind == ResourceKind::PortForwards {
        forwards.to_vec()
    } else {
        catalog.resolve(current_kind, client).map(|k| k.rows()).unwrap_or_default()
    };
        let mut generic_visible: Vec<usize> = (0..generic_rows_full.len())
            .filter(|&i| {
                let row = &generic_rows_full[i];
                // Cluster-scoped rows (namespace "-") are never hidden by a namespace.
                ns_filter.is_none_or(|ns| row.namespace == "-" || row.namespace == ns)
                    && scope.is_none_or(|s| s.matches_row(row))
            })
            .filter(|&i| generic_matches(&search, &generic_rows_full[i]))
            .filter(|&i| !faults || matches!(&generic_rows_full[i].status, Some((crate::k8s::describe::Tone::Warn | crate::k8s::describe::Tone::Bad, _))))
            .collect();
        // Whether the table will show a namespace column, decides which
        // sort column is which.
        let generic_has_namespace = generic_visible.iter().any(|&i| generic_rows_full[i].namespace != "-");
        // The generic table's width: namespace (if shown), name, the kind's own
        // columns, age, what sort digits can reach.
        let generic_columns =
            usize::from(generic_has_namespace) + 1 + generic_headers.len() + 1 + usize::from(wide);
        apply(&mut generic_visible, sort, |&i, column| generic_key(&generic_rows_full[i], column, generic_has_namespace));
        let generic_rows: Vec<k8s::GenericRow> = generic_visible.iter().map(|&i| generic_rows_full[i].clone()).collect();
        // The CRD picker, unfiltered or scoped to one API group. Each entry keeps its
        // real index into `catalog.crds`.
        let crd_rows: Vec<(usize, k8s::CrdInfo)> = match current_kind {
            ResourceKind::CustomResourceList => catalog
                .crds
                .iter()
                .cloned()
                .enumerate()
                .filter(|(_, c)| row_matches(&search, &format!("{} {}", c.group, c.kind)))
                .collect(),
            ResourceKind::CustomResourceGroup(group) => catalog
                .crds
                .iter()
                .cloned()
                .enumerate()
                .filter(|(_, c)| c.group == group && row_matches(&search, &format!("{} {}", c.group, c.kind)))
                .collect(),
            _ => Vec::new(),
        };
        // Lists of types show how many objects each has; the counting starts when one is opened.
        if matches!(current_kind, ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) | ResourceKind::ApiResources) {
            catalog.count_instances(namespace.as_deref());
        }
        let mut with_counts: Vec<((usize, k8s::CrdInfo), k8s::Count)> = crd_rows.into_iter().map(|row| { let count = catalog.counts.get(row.1.group, &row.1.plural); (row, count) }).collect();
        apply(&mut with_counts, sort, |((_, crd), count), column| crd_key(crd, *count, column));
        let (crd_rows, crd_counts): (Vec<(usize, k8s::CrdInfo)>, Vec<k8s::Count>) = with_counts.into_iter().unzip();

    Derived { pods, pod_rows, deployments, dep_rows, nodes, usage, node_detail_pods, node_detail_rows, sorted_nodes, node_rows, overview, generic_headers, generic_rows_full, generic_visible, generic_columns, generic_rows, crd_rows, crd_counts }
}

/// The last derivation and what it was made from, so idle iterations (a mouse move,
/// a redraw tick) reuse it instead of re-sorting every store.
pub(super) struct Cache {
    key: String,
    changes: u64,
    at: std::time::Instant,
    /// How long making it took, so a big cluster is not recomputed faster than it can be.
    took: std::time::Duration,
    derived: Derived,
}

/// A watch that changed something is picked up at most this often.
const MIN_REFRESH: std::time::Duration = std::time::Duration::from_millis(250);
/// Ages and metrics move without any watch event, so refresh at least this often.
const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(1);

/// What `derive` reads besides the stores.
fn key_of(q: &Query, mode: &Mode, forwards: &[k8s::GenericRow]) -> String {
    let forwards: Vec<&str> = forwards.iter().map(|f| f.name.as_str()).collect();
    format!(
        "{:?}|{:?}|{:?}|{}|{:?}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{forwards:?}",
        q.current_kind,
        q.namespace,
        q.scope,
        q.search,
        q.sort,
        q.faults,
        q.wide,
        q.layout,
        node_detail_name(mode),
        node_detail_search(mode),
        node_detail_sort(mode),
        if let Mode::ColumnDetail { col, .. } = mode { Some(*col) } else { None },
        matches!(mode, Mode::ResourcesDetail),
    )
}

impl Cache {
    /// The derivation for `q`: the cached one while it is still good, else a fresh one.
    pub(super) fn take_or_derive(cache: Option<Cache>, src: &Sources, catalog: &mut Catalog, mode: &Mode, q: &Query) -> Cache {
        let key = key_of(q, mode, src.forwards);
        let changes = k8s::changes();
        if let Some(cache) = cache
            && cache.key == key
            && cache.at.elapsed() < MAX_AGE.max(cache.took * 4)
            && (cache.changes == changes || cache.at.elapsed() < MIN_REFRESH.max(cache.took * 4))
        {
            return cache;
        }
        static DERIVATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // The tables cache their column widths per derivation.
        ui::set_data_version(DERIVATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1);
        let started = std::time::Instant::now();
        let derived = derive(src, catalog, mode, q);
        Cache { key, changes, at: std::time::Instant::now(), took: started.elapsed(), derived }
    }

    pub(super) fn derived(&self) -> &Derived {
        &self.derived
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use kube::runtime::{reflector, watcher};
    use std::time::Instant;

    fn pod(i: usize) -> Pod {
        serde_json::from_value(serde_json::json!({
            "metadata": {"name": format!("web-{i}-abcde"), "namespace": format!("ns-{}", i % 500), "creationTimestamp": "2020-01-01T00:00:00Z", "labels": {"app": "web"},
                "ownerReferences": [{"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "rs", "uid": "u", "controller": true}]},
            "spec": {"nodeName": format!("node-{}", i % 300), "containers": [{"name": "a", "image": "nginx"}, {"name": "b", "image": "envoy"}]},
            "status": {"phase": "Running", "containerStatuses": [{"name": "a", "ready": true, "restartCount": 0, "image": "nginx", "imageID": "x", "state": {"running": {"startedAt": "2020-01-01T00:00:00Z"}}}]}
        }))
        .unwrap()
    }

    /// Generic rows for 100k ConfigMaps: building them, then the clone `derive` makes.
    #[test]
    #[ignore]
    fn bench_generic_rows() {
        use k8s_openapi::api::core::v1::ConfigMap;
        let items: Vec<ConfigMap> = (0..100_000)
            .map(|i| serde_json::from_value(serde_json::json!({"metadata": {"name": format!("cm-{i}"), "namespace": format!("ns-{}", i % 500), "creationTimestamp": "2020-01-01T00:00:00Z", "labels": {"app": "x", "tier": "y"}}, "data": {"a": "1", "b": "2"}})).unwrap())
            .collect();
        let t = Instant::now();
        let rows: Vec<k8s::GenericRow> = items.iter().map(k8s::generic_row).collect();
        println!("generic_row x100k (serial)   {:?}", t.elapsed());
        let t = Instant::now();
        let rows2: Vec<k8s::GenericRow> = k8s::par_map(&items, k8s::generic_row);
        println!("generic_row x100k (parallel) {:?}", t.elapsed());
        let t = Instant::now();
        let visible: Vec<usize> = (0..rows.len()).collect();
        let cloned: Vec<k8s::GenericRow> = visible.iter().map(|&i| rows[i].clone()).collect();
        println!("clone of all rows            {:?}", t.elapsed());
        assert_eq!(rows2.len() + cloned.len(), 200_000);
    }

    /// `cargo test --release bench_ -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_pods() {
        let n = 100_000;
        let (store, mut writer) = reflector::store::<Pod>();
        for i in 0..n {
            writer.apply_watcher_event(&watcher::Event::Apply(pod(i)));
        }
        let t = Instant::now();
        let all = k8s::snapshot(&store);
        println!("sorted snapshot     {:?}", t.elapsed());
        let t = Instant::now();
        let pods: Vec<_> = all.iter().filter(|p| meta_matches("", &p.metadata)).cloned().collect();
        println!("filter (no search)  {:?}", t.elapsed());
        let t = Instant::now();
        let mut pairs: Vec<(std::sync::Arc<Pod>, k8s::PodRow)> = k8s::par_map(&pods, |p| (p.clone(), k8s::row_for(p)));
        println!("rows x{n} (parallel) {:?}", t.elapsed());
        let t = Instant::now();
        apply(&mut pairs, Some(SortSpec { column: 3, descending: true }), |(_, r), c| pod_key(r, c, false));
        println!("sort by column      {:?}", t.elapsed());
        let rows = pairs;
        let t = Instant::now();
        let kept = pods.iter().filter(|p| meta_matches("web-9", &p.metadata)).count();
        println!("fuzzy search ({kept})  {:?}", t.elapsed());
        assert_eq!(rows.len(), n);
    }
}
