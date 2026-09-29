//! What is selected on the list: its manifest, the objects around it, and the pod views.

use crate::ops::NoticeTone;
use super::*;

/// Scrolls the overview's columns so the selected tile is on screen.
pub(super) fn keep_overview_selection_visible(st: &mut State, overview: &k8s::Overview, frame_area: Rect) {
    let columns_area = ui::columns_area(ui::beside_sidebar(frame_area, false, &st.chrome), overview);
    if let ui::OverviewSelection::Header(c) | ui::OverviewSelection::Item(c, _) = st.overview_selection {
        let cols_visible = ui::visible_columns(columns_area.width, overview.catalog.len());
        st.overview_col_scroll = ui::scroll_columns_to_show(st.overview_col_scroll, cols_visible, c);
        let target_item = match st.overview_selection {
            ui::OverviewSelection::Item(_, i) => i,
            _ => 0,
        };
        let items_visible = ui::visible_items_per_column(columns_area.height, ui::column_item_height(overview, c));
        st.overview_item_scroll = ui::scroll_columns_to_show(st.overview_item_scroll, items_visible, target_item);
    }
}

/// The manifest of the row the cursor is on, for every kind with rows.
pub(crate) fn selected_manifest(st: &State, d: &Derived, catalog: &mut Catalog, client: &Client) -> Option<serde_yaml::Value> {
    let selected = st.table_state.selected()?;
    match st.current_kind {
        ResourceKind::Overview | ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => None,
        ResourceKind::Pods => d.pods.get(selected).map(|p| k8s::manifest_value(p.as_ref())),
        ResourceKind::Deployments => d.deployments.get(selected).map(|x| k8s::manifest_value(x.as_ref())),
        ResourceKind::Nodes => d.sorted_nodes.get(selected).map(|n| k8s::manifest_value(n.as_ref())),
        kind => {
            let real = *d.generic_visible.get(selected)?;
            catalog.resolve(kind, client).and_then(|k| k.spec_at(real))
        }
    }
}

/// The kinds the relations view reads besides Pods and Deployments.
pub(super) const RELATED_KINDS: [ResourceKind; 15] = [
    ResourceKind::Nodes,
    ResourceKind::ReplicaSets,
    ResourceKind::StatefulSets,
    ResourceKind::DaemonSets,
    ResourceKind::Jobs,
    ResourceKind::CronJobs,
    ResourceKind::ConfigMaps,
    ResourceKind::Secrets,
    ResourceKind::Hpas,
    ResourceKind::Services,
    ResourceKind::Ingresses,
    ResourceKind::Pvcs,
    ResourceKind::Pvs,
    ResourceKind::StorageClasses,
    ResourceKind::ServiceAccounts,
];

/// The manifests around `target`: its namespace plus cluster-wide objects it may use,
/// with ConfigMap and Secret payloads dropped.
pub(super) fn surrounding_manifests(pod_store: &k8s::PodKept, dep_store: &k8s::DeploymentKept, catalog: &mut Catalog, target: &serde_yaml::Value) -> Vec<serde_yaml::Value> {
    use kube::ResourceExt;
    let kind = target.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    let namespace = target.get("metadata").and_then(|m| m.get("namespace")).and_then(|n| n.as_str()).map(String::from);
    // A cluster-scoped target (a Node, a PV) can be used from any namespace.
    let filter = if matches!(kind, "Node" | "PersistentVolume" | "StorageClass") { None } else { namespace.as_deref() };
    let mut all: Vec<serde_yaml::Value> = Vec::new();
    all.extend(pod_store.objects().iter().filter(|p| filter.is_none() || p.namespace().as_deref() == filter).map(|p| k8s::manifest_value(p.as_ref())));
    all.extend(dep_store.objects().iter().filter(|d| filter.is_none() || d.namespace().as_deref() == filter).map(|d| k8s::manifest_value(d.as_ref())));
    for kind in RELATED_KINDS {
        if let Some(k) = catalog.get(kind) {
            all.extend(k.manifests(filter));
        }
    }
    all.into_iter().map(k8s::relations::slim).collect()
}

/// What to do with a pod's container.
pub(super) enum PodView {
    Shell,
    Logs { previous: bool },
}

/// Opens a shell or logs in a pod's container: at once for a single container, else
/// the container list.
pub(super) fn open_pod(st: &mut State, cx: &mut Cx, target: &Target, view: PodView) {
    let Ok(pod) = serde_yaml::from_value::<k8s_openapi::api::core::v1::Pod>(target.manifest.clone()) else { return };
    let containers = k8s::containers_for(&pod);
    let namespace = target.namespace.clone().unwrap_or_default();
    // The previous run only exists for a container that restarted.
    if let PodView::Logs { previous: true } = view {
        let restarted: Vec<&k8s::ContainerInfo> = containers.iter().filter(|c| c.restarts > 0).collect();
        match restarted.as_slice() {
            [] => {
                st.mode = Mode::Notice { text: format!("{} has not restarted, so there is no previous run to show", target.name), tone: NoticeTone::Info, back: Box::new(Mode::List) };
                return;
            }
            [one] => {
                let name = one.name.clone();
                st.mode = logs_mode(cx, &namespace, &target.name, &name, true, Mode::List);
                return;
            }
            _ => {}
        }
    }
    match (containers.as_slice(), view) {
        ([only], PodView::Shell) => {
            let name = only.name.clone();
            open_shell(st, cx, &namespace, &target.name, &name);
        }
        ([only], PodView::Logs { previous }) => {
            st.mode = logs_mode(cx, &namespace, &target.name, &only.name, previous, Mode::List);
        }
        _ => {
            st.mode = Mode::Containers {
                title: title_for(Some(&namespace), Some(&target.name)),
                namespace,
                pod: target.name.clone(),
                containers,
                state: TableState::default().with_selected(0),
                sort: ListSort::default(),
                back: Box::new(Mode::List),
            };
        }
    }
}

/// Every row of the current list, in order, with its manifest.
pub(super) fn visible_manifests(st: &State, d: &Derived, catalog: &mut Catalog, client: &Client) -> Vec<serde_yaml::Value> {
    match st.current_kind {
        ResourceKind::Overview | ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => Vec::new(),
        ResourceKind::Pods => d.pods.iter().map(|p| k8s::manifest_value(p.as_ref())).collect(),
        ResourceKind::Deployments => d.deployments.iter().map(|x| k8s::manifest_value(x.as_ref())).collect(),
        ResourceKind::Nodes => d.sorted_nodes.iter().map(|n| k8s::manifest_value(n.as_ref())).collect(),
        kind => match catalog.resolve(kind, client) {
            Some(k) => d.generic_visible.iter().filter_map(|&real| k.spec_at(real)).collect(),
            None => Vec::new(),
        },
    }
}

/// The marked rows of the current list as action targets.
pub(super) fn marked_targets(st: &State, d: &Derived, catalog: &mut Catalog, client: &Client) -> Vec<Target> {
    visible_manifests(st, d, catalog, client)
        .iter()
        .filter_map(Target::from_manifest)
        .filter(|t| st.marked.contains(&ui::mark_key(t.namespace.as_deref().unwrap_or("-"), &t.name)))
        .collect()
}
