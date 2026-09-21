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

/// Every visible row, with live counts from the Home catalog.
pub(crate) fn entries(current: ResourceKind, folded: &HashSet<&'static str>, crds: &[k8s::CrdInfo], overview: &k8s::Overview) -> Vec<Entry> {
    let counts: HashMap<&str, usize> = overview.catalog.iter().flat_map(|(_, items)| items.iter().copied()).collect();
    let mut out = vec![Entry { row: ui::SidebarRow { label: "Home".into(), heading: false, collapsed: false, count: None, current: current == ResourceKind::Overview }, kind: Some(ResourceKind::Overview), section: "" }];
    for section in commands::menu_sections(crds) {
        let collapsed = folded.contains(section.title);
        let title: &'static str = section.title;
        out.push(Entry { row: ui::SidebarRow { label: title.into(), heading: true, collapsed, count: None, current: false }, kind: None, section: title });
        if collapsed {
            continue;
        }
        for kind in section.tiles.into_iter().filter(|k| *k != ResourceKind::Overview) {
            let label = kind.label();
            out.push(Entry { row: ui::SidebarRow { label: label.to_string(), heading: false, collapsed: false, count: counts.get(label).copied(), current: kind == current }, kind: Some(kind), section: title });
        }
    }
    out
}

/// The row of the list on screen, where the cursor rests when the sidebar has no focus.
pub(crate) fn current_index(entries: &[Entry]) -> usize {
    entries.iter().position(|e| e.row.current).unwrap_or(0)
}
