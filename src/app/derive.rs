//! Per-frame derivation: what each list shows this instant, from the live
//! stores, the namespace/scope/search filters and the sort.

use super::*;
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
    let Query { current_kind, namespace, scope, search, sort, faults } = *q;
    let namespace = namespace.map(str::to_string);
    let search = search.to_string();
        // `search` only ever has an effect on whichever kind it was typed
        // against — it's cleared on every kind switch (see the `search.
        // clear()` calls alongside `current_kind = ...` below) — so
        // filtering every kind's source list by it unconditionally is
        // safe: for every kind other than the one actively being
        // searched, `search` is "" and `row_matches` always returns true.
        // The Overview stays cluster-wide even with a namespace set.
        let ns_filter: Option<&str> = if current_kind == ResourceKind::Overview { None } else { namespace.as_deref() };
        let in_namespace = |meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta| {
            ns_filter.is_none_or(|ns| meta.namespace.as_deref() == Some(ns))
        };
        let mut pods: Vec<std::sync::Arc<Pod>> = k8s::snapshot(pod_store)
            .into_iter()
            .filter(|p| in_namespace(&p.metadata))
            .filter(|p| current_kind != ResourceKind::Pods || scope.is_none_or(|s| s.matches_meta(&p.metadata)))
            .filter(|p| row_matches(&search, &meta_search_text(&p.metadata)))
            .filter(|p| !(faults && current_kind == ResourceKind::Pods) || k8s::pod_is_fault(p))
            .collect();
        if current_kind == ResourceKind::Pods {
            apply(&mut pods, sort, |p, column| pod_key(&k8s::row_for(p), column));
        }
        let pod_rows: Vec<k8s::PodRow> = pods.iter().map(|p| k8s::row_for(p)).collect();
        let mut deployments: Vec<std::sync::Arc<Deployment>> = k8s::snapshot_deployments(dep_store)
            .into_iter()
            .filter(|d| in_namespace(&d.metadata))
            .filter(|d| row_matches(&search, &meta_search_text(&d.metadata)))
            .filter(|d| !(faults && current_kind == ResourceKind::Deployments) || k8s::ready_is_short(&k8s::row_for_deployment(d).ready))
            .collect();
        if current_kind == ResourceKind::Deployments {
            apply(&mut deployments, sort, |d, column| deployment_key(&k8s::row_for_deployment(d), column));
        }
        let dep_rows: Vec<k8s::DeploymentRow> = deployments.iter().map(|d| k8s::row_for_deployment(d)).collect();
        let nodes = node_store.state();
        let events = event_store.state();
        let usage = node_metrics_rx.borrow().clone();
        // Only populated while actually viewing a node's detail — which
        // pod, out of everything on the cluster, is scheduled on this
        // one node. Searches the whole back-chain, not just the top
        // mode, so it's still available when NodeDetail is a dimmed
        // background layer behind Containers/Spec/Logs rather than the
        // focused view itself.
        let mut node_detail_pods: Vec<std::sync::Arc<Pod>> = if let Some(name) = node_detail_name(mode) {
            pods.iter().filter(|p| p.spec.as_ref().and_then(|s| s.node_name.as_deref()) == Some(name)).cloned().collect()
        } else {
            Vec::new()
        };
        let node_search = node_detail_search(mode).to_string();
        node_detail_pods.retain(|p| row_matches(&node_search, &meta_search_text(&p.metadata)));
        apply(&mut node_detail_pods, node_detail_sort(mode), |p, column| pod_key(&k8s::row_for(p), column));
        let node_detail_rows: Vec<k8s::PodRow> = node_detail_pods.iter().map(|p| k8s::row_for(p)).collect();
        // Nodes get their own specialized rows (CPU/Memory visible right
        // in the list) instead of the generic Namespace/Name/Age table.
        // Filtered directly here (not via the generic `catalog`/
        // `generic_rows` path other kinds use) so the 'd'/Enter handlers
        // below, which index straight into `sorted_nodes`, can't drift
        // out of alignment with what's actually displayed.
        // Every pod counts toward its node's PODS, whatever the list's search,
        // namespace or drill-down is currently narrowed to.
        let mut pods_per_node: HashMap<String, usize> = HashMap::new();
        for pod in k8s::snapshot(pod_store) {
            if let Some(node) = pod.spec.as_ref().and_then(|s| s.node_name.clone()) {
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
                let pod_count = pods_per_node.get(&name).copied().unwrap_or(0);
                let row = k8s::node_row(&n, node_usage, pod_count);
                (n, row)
            })
            .collect();
        if faults && current_kind == ResourceKind::Nodes {
            node_pairs.retain(|(_, row)| !row.ready || !row.schedulable);
        }
        if current_kind == ResourceKind::Nodes {
            apply(&mut node_pairs, sort, |(_, row), column| node_key(row, column));
        }
        let (sorted_nodes, node_rows): (Vec<std::sync::Arc<Node>>, Vec<k8s::NodeRow>) = node_pairs.into_iter().unzip();
        let catalog_sections = catalog.sections(pod_rows.len(), dep_rows.len());
        let overview = k8s::overview(&nodes, &events, usage.as_ref(), catalog_sections);
        // Only ever populated for whatever kind is currently on screen —
        // computed unconditionally so every match arm below can just read
        // it, same as `pod_rows`/`dep_rows` are always computed too.
        // `resolve` also lazily starts a CRD's watch the first time it's
        // the current kind — "watch on open", not for every installed CRD.
        // `generic_visible` maps a filtered display position back to its
        // real index in `generic_rows_full`/the catalog's own live
        // snapshot — needed because `CatalogKind::spec_at` (the 'd' key)
        // takes that real index, not the display one.
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
            .filter(|&i| row_matches(&search, &meta_search_text_generic(&generic_rows_full[i])))
            .filter(|&i| !faults || matches!(&generic_rows_full[i].status, Some((crate::describe::Tone::Warn | crate::describe::Tone::Bad, _))))
            .collect();
        // Whether the table will show a namespace column — decides which
        // sort column is which.
        let generic_has_namespace = generic_visible.iter().any(|&i| generic_rows_full[i].namespace != "-");
        // The generic table's width: namespace (if shown), name, the kind's own
        // columns, age — what sort digits can reach.
        let generic_columns =
            usize::from(generic_has_namespace) + 1 + generic_headers.len() + 1;
        apply(&mut generic_visible, sort, |&i, column| generic_key(&generic_rows_full[i], column, generic_has_namespace));
        let generic_rows: Vec<k8s::GenericRow> = generic_visible.iter().map(|&i| generic_rows_full[i].clone()).collect();
        // The CRD picker, unfiltered or scoped to one API group — each
        // entry keeps its real index into `catalog.crds` (needed to open
        // the right one on Enter even though this may be a filtered
        // subset of the full list).
        let mut crd_rows: Vec<(usize, k8s::CrdInfo)> = match current_kind {
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
        apply(&mut crd_rows, sort, |(_, crd), column| crd_key(crd, column));

    Derived { pods, pod_rows, deployments, dep_rows, nodes, usage, node_detail_pods, node_detail_rows, sorted_nodes, node_rows, overview, generic_headers, generic_rows_full, generic_visible, generic_columns, generic_rows, crd_rows }
}
