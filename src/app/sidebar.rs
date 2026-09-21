//! The resource sidebar's contents: Home, then each category with its kinds.

use super::*;

/// One row and what choosing it does.
pub(crate) struct Entry {
    pub row: ui::SidebarRow,
    /// The list a kind opens; `None` on a category heading.
    pub kind: Option<ResourceKind>,
    /// The category a row belongs to (a heading's own name on the heading).
    pub section: &'static str,
}

/// Categories start folded when they are long and rarely needed.
pub(crate) fn folded_by_default() -> HashSet<&'static str> {
    HashSet::from(["Custom Resources"])
}

/// Every visible row, in the order and with the entries of the Home catalog (so the Layout
/// settings shape both), with its live counts.
pub(crate) fn entries(current: ResourceKind, folded: &HashSet<&'static str>, catalog: &Catalog, overview: &k8s::Overview) -> Vec<Entry> {
    let mut out = vec![Entry { row: ui::SidebarRow { label: "Home".into(), heading: false, collapsed: false, count: None, current: current == ResourceKind::Overview }, kind: Some(ResourceKind::Overview), section: "" }];
    for (title, items) in &overview.catalog {
        let collapsed = folded.contains(title);
        out.push(Entry { row: ui::SidebarRow { label: (*title).into(), heading: true, collapsed, count: None, current: false }, kind: None, section: title });
        if collapsed {
            continue;
        }
        let mut kinds: Vec<(&'static str, Option<usize>, ResourceKind)> = items.iter().filter_map(|(label, count)| catalog.kind_for_tile_label(label).map(|kind| (*label, Some(*count), kind))).collect();
        // Port-forwards are knav's own, so the catalog has no tile for them.
        if *title == "Network" {
            kinds.push(("Port-forwards", None, ResourceKind::PortForwards));
        }
        // A custom resource group is listed with how many objects its kinds hold, once counted.
        for (_, count, kind) in kinds.iter_mut() {
            if let ResourceKind::CustomResourceGroup(group) = kind {
                *count = object_total(catalog, |c| c.group == *group);
            }
        }
        if *title == "Custom Resources"
            && let Some(first) = kinds.iter_mut().find(|(_, _, k)| *k == ResourceKind::CustomResourceList)
        {
            first.1 = object_total(catalog, |_| true);
        }
        for (label, count, kind) in kinds {
            out.push(Entry { row: ui::SidebarRow { label: label.to_string(), heading: false, collapsed: false, count, current: kind == current }, kind: Some(kind), section: title });
        }
    }
    out
}

/// The objects held by the custom resource kinds `keep` picks, when every one has been counted.
fn object_total(catalog: &Catalog, keep: impl Fn(&k8s::CrdInfo) -> bool) -> Option<usize> {
    let mut total = 0;
    for crd in catalog.crds.iter().filter(|c| keep(c)) {
        match catalog.counts.get(crd.group, &crd.plural) {
            k8s::Count::Known(n) => total += n,
            _ => return None,
        }
    }
    Some(total)
}

/// The row of the list on screen, where the cursor rests when the sidebar has no focus.
pub(crate) fn current_index(entries: &[Entry]) -> usize {
    entries.iter().position(|e| e.row.current).unwrap_or(0)
}
