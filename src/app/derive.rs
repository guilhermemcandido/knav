//! What each list shows this instant, from the live stores, the filters and the sort.

use super::*;
use crate::k8s::describe::Tone;
use std::sync::Arc;

/// Everything the frame and the handlers read, recomputed each loop iteration.
pub(super) struct Derived {
    pub pods: Vec<Arc<Pod>>,
    pub pod_rows: Vec<Arc<k8s::PodRow>>,
    pub deployments: Vec<Arc<Deployment>>,
    pub dep_rows: Vec<Arc<k8s::DeploymentRow>>,
    pub nodes: Vec<Arc<Node>>,
    pub usage: Option<metrics::ClusterUsage>,
    /// Each pod's usage, when metrics-server answered.
    pub pod_usage: Option<Arc<metrics::PodUsageMap>>,
    pub node_detail_pods: Vec<Arc<Pod>>,
    pub node_detail_rows: Vec<Arc<k8s::PodRow>>,
    pub sorted_nodes: Vec<Arc<Node>>,
    pub node_rows: Vec<k8s::NodeRow>,
    pub overview: k8s::Overview,
    pub generic_headers: Vec<&'static str>,
    pub generic_rows_full: Vec<Arc<k8s::GenericRow>>,
    pub generic_visible: Vec<usize>,
    pub generic_columns: usize,
    pub generic_rows: Vec<Arc<k8s::GenericRow>>,
    pub crd_rows: Vec<(usize, k8s::CrdInfo)>,
    /// How many objects each CRD kind in `crd_rows` has.
    pub crd_counts: Vec<k8s::Count>,
    /// The open extension dashboard's title and lines, built only while it is open.
    pub dashboard: Option<(String, Vec<ratatui::text::Line<'static>>)>,
    /// Everything that needs a look, worst first, built only while Problems is open.
    pub problems: Vec<Arc<k8s::problems::Problem>>,
}

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
    /// Ids of the enabled extensions.
    pub extensions_enabled: &'a [String],
}

pub(super) struct Sources<'a> {
    pub pod_store: &'a k8s::PodKept,
    pub dep_store: &'a k8s::DeploymentKept,
    pub node_store: &'a Store<Node>,
    pub event_store: &'a Store<k8s_openapi::api::core::v1::Event>,
    pub node_metrics_rx: &'a watch::Receiver<Option<metrics::ClusterUsage>>,
    pub pod_usage_rx: &'a watch::Receiver<Option<Arc<metrics::PodUsageMap>>>,
    pub client: &'a Client,
    pub registry: &'a crate::extensions::Registry,
    /// Rows for the Port-forwards list (knav's own, not from the cluster).
    pub forwards: &'a [k8s::GenericRow],
}

pub(super) fn derive(src: &Sources, catalog: &mut Catalog, mode: &Mode, q: &Query) -> Derived {
    let Sources { pod_store, dep_store, node_store, event_store, node_metrics_rx, pod_usage_rx, client, forwards, registry } = *src;
    let pod_usage = pod_usage_rx.borrow().clone();
    let Query { current_kind, namespace, scope, search, sort, faults, wide, layout, extensions_enabled } = *q;
    let namespace = namespace.map(str::to_string);
    let search = search.to_string();
        // The Overview stays cluster-wide even with a namespace set. `search` is cleared
        // on every kind switch, so filtering every list by it is safe.
        let ns_filter: Option<&str> = if current_kind == ResourceKind::Overview { None } else { namespace.as_deref() };
        let in_namespace = |meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta| {
            ns_filter.is_none_or(|ns| meta.namespace.as_deref() == Some(ns))
        };
        let all_pods = pod_store.items();
        // Kept rows are shared, not rebuilt. The fuzzy search over 100k names is the slow
        // part, so it runs across the cores.
        let keep = k8s::par_map(&all_pods, |(p, _)| {
            in_namespace(&p.metadata)
                && (current_kind != ResourceKind::Pods || scope.is_none_or(|s| s.matches_meta(&p.metadata)))
                && meta_matches(&search, &p.metadata)
                && (!(faults && current_kind == ResourceKind::Pods) || k8s::pod_is_fault(p))
        });
        let mut pairs: Vec<k8s::Item<Pod, k8s::PodRow>> = all_pods.iter().zip(keep).filter(|(_, keep)| *keep).map(|(item, _)| item.clone()).collect();
        if current_kind == ResourceKind::Pods {
            apply(&mut pairs, sort, |(_, row), column| pod_key(row, column, wide, pod_usage.as_deref()));
        }
        let (pods, pod_rows): (Vec<Arc<Pod>>, Vec<Arc<k8s::PodRow>>) = pairs.into_iter().unzip();
        let mut dep_pairs: Vec<k8s::Item<Deployment, k8s::DeploymentRow>> = dep_store
            .items()
            .iter()
            .filter(|(d, _)| in_namespace(&d.metadata))
            .filter(|(d, _)| meta_matches(&search, &d.metadata))
            .filter(|(_, row)| !(faults && current_kind == ResourceKind::Deployments) || k8s::ready_is_short(&row.ready))
            .cloned()
            .collect();
        if current_kind == ResourceKind::Deployments {
            apply(&mut dep_pairs, sort, |(_, row), column| deployment_key(row, column, wide));
        }
        let (deployments, dep_rows): (Vec<Arc<Deployment>>, Vec<Arc<k8s::DeploymentRow>>) = dep_pairs.into_iter().unzip();
        let nodes = node_store.state();
        let events = event_store.state();
        let usage = node_metrics_rx.borrow().clone();
        // The pods on the open node, from the whole back-chain so it works while
        // NodeDetail is a dimmed background.
        let mut detail: Vec<k8s::Item<Pod, k8s::PodRow>> = if let Some(name) = node_detail_name(mode) {
            pods.iter().zip(&pod_rows).filter(|(p, _)| p.spec.as_ref().and_then(|s| s.node_name.as_deref()) == Some(name)).map(|(p, r)| (p.clone(), r.clone())).collect()
        } else {
            Vec::new()
        };
        let node_search = node_detail_search(mode).to_string();
        detail.retain(|(p, _)| meta_matches(&node_search, &p.metadata));
        apply(&mut detail, node_detail_sort(mode), |(_, row), column| pod_key(row, column, false, pod_usage.as_deref()));
        let (node_detail_pods, node_detail_rows): (Vec<Arc<Pod>>, Vec<Arc<k8s::PodRow>>) = detail.into_iter().unzip();
        // Nodes are filtered here so handlers indexing `sorted_nodes` match the display.
        // Every pod counts toward its node's PODS, whatever the list is narrowed to.
        let pods_per_node = pods_per_node(&all_pods);
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
        let catalog_sections = crate::app::overview_layout::arrange(catalog.sections(pod_rows.len(), dep_rows.len(), extensions_enabled), layout);
        // Only the opened-up category view shows it, so only work it out then.
        let health = if let Mode::ColumnDetail { col, .. } = mode {
            // Health needs the objects, so start watching this category's kinds.
            for (label, _) in catalog_sections.get(*col).map(|(_, items)| items.as_slice()).unwrap_or(&[]) {
                catalog.ensure_label(label);
            }
            catalog.health([("Pods", k8s::pods_health(&pod_rows)), ("Deployments", k8s::deployments_health(&dep_rows)), ("Nodes", node_health)])
        } else {
            Default::default()
        };
        let report = matches!(mode, Mode::ResourcesDetail).then(|| k8s::report::report(&pod_store.objects()));
        let overview = k8s::overview(&nodes, &events, usage.as_ref(), catalog_sections, health, report);
        // Only for the kind on screen; `resolve` starts a CRD's watch on first open.
        // `generic_visible` maps a display row back to the index `spec_at` needs.
    if let Some(kind) = catalog.resolve(current_kind, client) {
        kind.set_wide(wide);
        kind.set_namespace(ns_filter);
    }
    let generic_headers: Vec<&'static str> = if current_kind == ResourceKind::PortForwards {
        portforward::HEADERS.to_vec()
    } else {
        catalog.resolve(current_kind, client).map(|k| k.headers()).unwrap_or_default()
    };
        let generic_rows_full: Vec<Arc<k8s::GenericRow>> = if current_kind == ResourceKind::PortForwards {
        forwards.iter().cloned().map(Arc::new).collect()
    } else {
        catalog.resolve(current_kind, client).map(|k| k.rows()).unwrap_or_default()
    };
        // Run across the cores: a fuzzy search over 100k rows is the slow part.
        let candidates: Vec<usize> = (0..generic_rows_full.len()).collect();
        let keep = k8s::par_map(&candidates, |&i| {
            let row = &generic_rows_full[i];
            // Cluster-scoped rows (namespace "-") are never hidden by a namespace.
            (ns_filter.is_none_or(|ns| row.namespace == "-" || row.namespace == ns))
                && scope.is_none_or(|s| s.matches_row(row))
                && generic_matches(&search, row)
                && (!faults || matches!(&row.status, Some((crate::k8s::describe::Tone::Warn | crate::k8s::describe::Tone::Bad, _))))
        });
        let mut generic_visible: Vec<usize> = candidates.into_iter().zip(keep).filter(|(_, keep)| *keep).map(|(i, _)| i).collect();
        // Whether the table shows a namespace column, which decides the sort columns.
        let generic_has_namespace = generic_visible.iter().any(|&i| generic_rows_full[i].namespace != "-");
        // The generic table's column count, for what sort digits can reach.
        let generic_columns =
            usize::from(generic_has_namespace) + 1 + generic_headers.len() + 1 + usize::from(wide);
        apply(&mut generic_visible, sort, |&i, column| generic_key(&generic_rows_full[i], column, generic_has_namespace));
        let generic_rows: Vec<Arc<k8s::GenericRow>> = generic_visible.iter().map(|&i| Arc::clone(&generic_rows_full[i])).collect();
        // The CRD picker, whole or scoped to one API group, keeping each kind's index.
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
        // Type lists show object counts, counted once one opens. The Overview counts too,
        // so an extension's category isn't stuck at 0.
        if matches!(current_kind, ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) | ResourceKind::ApiResources | ResourceKind::Overview) {
            catalog.count_instances(namespace.as_deref());
        }
        let mut with_counts: Vec<((usize, k8s::CrdInfo), k8s::Count)> = crd_rows.into_iter().map(|row| { let count = catalog.counts.get(row.1.group, &row.1.plural); (row, count) }).collect();
        apply(&mut with_counts, sort, |((_, crd), count), column| crd_key(crd, *count, column));
        let (crd_rows, crd_counts): (Vec<(usize, k8s::CrdInfo)>, Vec<k8s::Count>) = with_counts.into_iter().unzip();
        // Built only while its dashboard is open, since it fetches whole manifests.
        // Built only while Problems is open: it reads every pod, Deployment, node and
        // a few more kinds, which it starts watching.
        let problems = if matches!(mode, Mode::Problems { .. }) {
            let mut found: Vec<k8s::problems::Problem> = Vec::new();
            found.extend(all_pods.iter().filter(|(p, _)| in_namespace(&p.metadata)).filter_map(|(p, _)| k8s::problems::pod(p)));
            found.extend(dep_store.items().iter().filter(|(d, _)| in_namespace(&d.metadata)).filter_map(|(d, _)| k8s::problems::deployment(d)));
            found.extend(nodes.iter().filter_map(|n| k8s::problems::node(n)));
            for kind in PROBLEM_KINDS {
                catalog.ensure(kind);
                let rows = catalog.resolve(kind, client).map(|k| k.rows()).unwrap_or_default();
                found.extend(rows.iter().filter(|r| ns_filter.is_none_or(|ns| r.namespace == ns)).filter_map(|r| k8s::problems::row(kind, r)));
            }
            k8s::problems::sort(&mut found);
            found.into_iter().map(Arc::new).collect()
        } else {
            Vec::new()
        };
        let dashboard = if let ResourceKind::ExtensionDashboard(category) = current_kind {
            extensions::dashboards::find(category, registry).map(|found| {
                let mut ctx = extensions::dashboards::DashboardContext::new(catalog, client, &sorted_nodes, &node_rows, &overview.events);
                (found.title(), found.lines(&mut ctx))
            })
        } else {
            None
        };

    Derived { pods, pod_rows, deployments, dep_rows, nodes, usage, pod_usage, node_detail_pods, node_detail_rows, sorted_nodes, node_rows, overview, generic_headers, generic_rows_full, generic_visible, generic_columns, generic_rows, crd_rows, crd_counts, dashboard, problems }
}

/// The kinds besides pods, Deployments and nodes that Problems looks through.
const PROBLEM_KINDS: [ResourceKind; 4] = [ResourceKind::StatefulSets, ResourceKind::DaemonSets, ResourceKind::Jobs, ResourceKind::Pvcs];

/// How many pods each node runs, counted on several threads for big clusters.
fn pods_per_node(pods: &[k8s::Item<Pod, k8s::PodRow>]) -> HashMap<&str, usize> {
    fn count(part: &[k8s::Item<Pod, k8s::PodRow>]) -> HashMap<&str, usize> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (pod, _) in part {
            if let Some(node) = pod.spec.as_ref().and_then(|s| s.node_name.as_deref()) {
                *counts.entry(node).or_default() += 1;
            }
        }
        counts
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    if pods.len() < 20_000 || threads == 1 {
        return count(pods);
    }
    let parts: Vec<HashMap<&str, usize>> = std::thread::scope(|scope| {
        let workers: Vec<_> = pods.chunks(pods.len().div_ceil(threads)).map(|part| scope.spawn(|| count(part))).collect();
        workers.into_iter().map(|w| w.join().expect("count worker panicked")).collect()
    });
    let mut total = HashMap::new();
    for part in parts {
        for (node, n) in part {
            *total.entry(node).or_default() += n;
        }
    }
    total
}

/// The last derivation and what it came from, so idle iterations (a mouse move, a
/// redraw tick) reuse it.
pub(super) struct Cache {
    key: String,
    changes: u64,
    at: std::time::Instant,
    /// How long it took, so a big cluster isn't recomputed faster than it can be.
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
        "{:?}|{:?}|{:?}|{}|{:?}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}|{forwards:?}",
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
        matches!(mode, Mode::Problems { .. }),
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
        let t = Instant::now();
        let serial = rows.iter().filter(|r| generic_matches("cm-9", r)).count();
        println!("search 100k rows (serial)    {:?}", t.elapsed());
        let t = Instant::now();
        let parallel = k8s::par_map(&rows, |r| generic_matches("cm-9", r)).into_iter().filter(|k| *k).count();
        println!("search 100k rows (parallel)  {:?}", t.elapsed());
        assert_eq!(serial, parallel);
        assert_eq!(rows2.len() + cloned.len(), 200_000);
    }

    /// `cargo test --release bench_ -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_pods() {
        let n = 100_000;
        let (store, mut writer) = reflector::store::<Pod>();
        let feed = std::sync::Arc::new(k8s::Feed::default());
        let kept = k8s::PodKept::new(store, feed.clone(), k8s::row_for);
        for i in 0..n {
            writer.apply_watcher_event(&watcher::Event::Apply(pod(i)));
        }
        let t = Instant::now();
        let all = kept.items();
        println!("first build (sort + rows)      {:?}", t.elapsed());
        // One pod changes, the way a watch would report it.
        let event = watcher::Event::Apply(pod(4242));
        writer.apply_watcher_event(&event);
        feed.note(&event);
        let t = Instant::now();
        let all = { drop(all); kept.items() };
        println!("one pod changed of {n}         {:?}", t.elapsed());
        let t = Instant::now();
        let mut pairs: Vec<_> = all.iter().filter(|(p, _)| meta_matches("", &p.metadata)).cloned().collect();
        println!("filter (no search)             {:?}", t.elapsed());
        let t = Instant::now();
        apply(&mut pairs, Some(SortSpec { column: 3, descending: true }), |(_, r), c| pod_key(r, c, false, None));
        println!("sort by column                 {:?}", t.elapsed());
        let t = Instant::now();
        let kept_rows = k8s::par_map(&all, |(p, _)| meta_matches("web-9", &p.metadata)).into_iter().filter(|k| *k).count();
        println!("fuzzy search ({kept_rows})            {:?}", t.elapsed());
        let t = Instant::now();
        let per_node = pods_per_node(&all);
        println!("pods per node ({})            {:?}", per_node.len(), t.elapsed());
        let t = Instant::now();
        kept.items();
        println!("nothing changed                {:?}", t.elapsed());
        assert_eq!(pairs.len(), n);
    }
}
