//! The sidebar's rows: Home, then each category with its kinds.

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
    HashSet::from(["CustomResources"])
}

/// Every visible row with its live count, in the Overview's order, so the Layout
/// settings shape both.
pub(crate) fn entries(current: ResourceKind, folded: &HashSet<&'static str>, catalog: &Catalog, overview: &k8s::Overview) -> Vec<Entry> {
    // A custom resource or API type opened from a list belongs to that list's row.
    let current = match current {
        ResourceKind::CustomResource(index, _) => catalog.crds.get(index).map_or(ResourceKind::CustomResourceList, |c| ResourceKind::CustomResourceGroup(c.group)),
        ResourceKind::Api(..) => ResourceKind::ApiResources,
        other => other,
    };
    let mut out = vec![Entry { row: ui::SidebarRow { label: "Home".into(), heading: false, collapsed: false, count: None, current: current == ResourceKind::Overview }, kind: Some(ResourceKind::Overview), section: "" }];
    for (title, items) in &overview.catalog {
        let collapsed = folded.contains(title);
        let mut kinds: Vec<(&'static str, Option<usize>, ResourceKind)> = items.iter().filter_map(|(label, count)| catalog.kind_for_tile_label(label).map(|kind| (*label, Some(*count), kind))).collect();
        // Port-forwards are knav's own, so the catalog has no tile for them.
        if *title == "Network" {
            kinds.push(("Port-forwards", None, ResourceKind::PortForwards));
        }
        // A folded category holds the current list: its heading stands for it.
        let holds_current = collapsed && kinds.iter().any(|(_, _, k)| *k == current);
        out.push(Entry { row: ui::SidebarRow { label: (*title).into(), heading: true, collapsed, count: None, current: holds_current }, kind: None, section: title });
        if collapsed {
            continue;
        }
        for (label, count, kind) in kinds {
            out.push(Entry { row: ui::SidebarRow { label: label.to_string(), heading: false, collapsed: false, count, current: kind == current }, kind: Some(kind), section: title });
        }
    }
    out
}

/// The row of the list on screen, where the cursor rests without focus.
pub(crate) fn current_index(entries: &[Entry]) -> usize {
    entries.iter().position(|e| e.row.current).unwrap_or(0)
}
