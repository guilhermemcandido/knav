//! The Overview's categories and kinds in the order and visibility the config
//! asks for: applied to the live catalog, and edited by the settings screen.

use crate::config::OverviewConfig;

/// Every category and its kinds, in the default order. "Custom Resources"
/// lists API groups found at run time, so it has no fixed kinds.
pub const DEFAULT_LAYOUT: &[(&str, &[&str])] = &[
    ("Cluster", &["Nodes", "Namespaces", "API Resources"]),
    ("Workloads", &["Pods", "Deployments", "ReplicaSets", "StatefulSets", "DaemonSets", "Jobs", "CronJobs"]),
    ("Config", &["ConfigMaps", "Secrets", "HPAs"]),
    ("Network", &["Services", "Endpoints", "Ingresses", "NetworkPolicies"]),
    ("Storage", &["PVCs", "PVs", "StorageClasses"]),
    ("Access Control", &["ServiceAccounts", "Roles", "RoleBindings", "ClusterRoles", "ClusterRoleBindings"]),
    ("Custom Resources", &[]),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutItem {
    pub name: String,
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutSection {
    pub name: String,
    pub hidden: bool,
    pub items: Vec<LayoutItem>,
}

/// Position of `name` in `order`, with names not listed after all listed ones
/// (and in their default order, since sorting is stable).
fn rank(order: &[String], name: &str) -> usize {
    order.iter().position(|n| n == name).unwrap_or(usize::MAX)
}

/// The full layout, hidden entries included, in the configured order.
pub fn resolve(config: &OverviewConfig) -> Vec<LayoutSection> {
    let mut sections: Vec<LayoutSection> = DEFAULT_LAYOUT
        .iter()
        .map(|(name, items)| {
            let mut items: Vec<LayoutItem> = items.iter().map(|i| LayoutItem { name: (*i).to_string(), hidden: config.hidden.contains(&format!("{name}/{i}")) }).collect();
            if let Some(order) = config.items.get(*name) {
                items.sort_by_key(|i| rank(order, &i.name));
            }
            LayoutSection { name: (*name).to_string(), hidden: config.hidden.iter().any(|h| h == name), items }
        })
        .collect();
    sections.sort_by_key(|s| rank(&config.sections, &s.name));
    sections
}

/// The config that reproduces `layout` exactly.
pub fn to_config(layout: &[LayoutSection]) -> OverviewConfig {
    let mut config = OverviewConfig { sections: layout.iter().map(|s| s.name.clone()).collect(), ..Default::default() };
    for section in layout {
        if section.hidden {
            config.hidden.push(section.name.clone());
        }
        if !section.items.is_empty() {
            config.items.insert(section.name.clone(), section.items.iter().map(|i| i.name.clone()).collect());
        }
        config.hidden.extend(section.items.iter().filter(|i| i.hidden).map(|i| format!("{}/{}", section.name, i.name)));
    }
    config
}

/// Whether anything of the layout would show: a category shows when it is not
/// hidden and has a visible kind (or is the custom resources one).
pub fn any_visible(layout: &[LayoutSection]) -> bool {
    layout.iter().any(|s| !s.hidden && (s.items.is_empty() || s.items.iter().any(|i| !i.hidden)))
}

/// One row of the editor: a category, or one of its kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlatRow {
    pub section: usize,
    pub item: Option<usize>,
}

pub fn flatten(layout: &[LayoutSection]) -> Vec<FlatRow> {
    let mut rows = Vec::new();
    for (s, section) in layout.iter().enumerate() {
        rows.push(FlatRow { section: s, item: None });
        rows.extend((0..section.items.len()).map(|i| FlatRow { section: s, item: Some(i) }));
    }
    rows
}

/// Moves the row one place up (`-1`) or down (`1`) among its siblings. Returns
/// the row's new place in the flattened list, or `None` if it can't move.
pub fn move_row(layout: &mut [LayoutSection], row: FlatRow, direction: i32) -> Option<usize> {
    let step = |at: usize, len: usize| -> Option<usize> {
        let to = at as i64 + i64::from(direction);
        (to >= 0 && (to as usize) < len).then_some(to as usize)
    };
    match row.item {
        None => {
            let to = step(row.section, layout.len())?;
            layout.swap(row.section, to);
            flatten(layout).iter().position(|r| *r == FlatRow { section: to, item: None })
        }
        Some(item) => {
            let items = &mut layout[row.section].items;
            let to = step(item, items.len())?;
            items.swap(item, to);
            flatten(layout).iter().position(|r| *r == FlatRow { section: row.section, item: Some(to) })
        }
    }
}

/// The live catalog in the configured order, without what is hidden.
pub fn arrange(sections: Vec<(&'static str, Vec<(&'static str, usize)>)>, config: &OverviewConfig) -> Vec<(&'static str, Vec<(&'static str, usize)>)> {
    let mut sections: Vec<_> = sections
        .into_iter()
        .filter(|(name, _)| !config.hidden.iter().any(|h| h == name))
        .map(|(name, mut items)| {
            if let Some(order) = config.items.get(name) {
                items.sort_by_key(|(label, _)| rank(order, label));
            }
            items.retain(|(label, _)| !config.hidden.contains(&format!("{name}/{label}")));
            (name, items)
        })
        .filter(|(_, items)| !items.is_empty())
        .collect();
    sections.sort_by_key(|(name, _)| rank(&config.sections, name));
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(layout: &[LayoutSection]) -> Vec<&str> {
        layout.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn the_default_layout_is_the_built_in_order() {
        let layout = resolve(&OverviewConfig::default());
        assert_eq!(names(&layout)[..3], ["Cluster", "Workloads", "Config"]);
        assert!(!any_hidden(&layout));
    }

    fn any_hidden(layout: &[LayoutSection]) -> bool {
        layout.iter().any(|s| s.hidden || s.items.iter().any(|i| i.hidden))
    }

    #[test]
    fn listed_names_come_first_and_the_rest_keep_their_order() {
        let config = OverviewConfig { sections: vec!["Storage".into(), "Cluster".into()], ..Default::default() };
        let layout = resolve(&config);
        assert_eq!(names(&layout)[..4], ["Storage", "Cluster", "Workloads", "Config"]);
    }

    #[test]
    fn items_are_ordered_within_their_category() {
        let mut config = OverviewConfig::default();
        config.items.insert("Config".into(), vec!["HPAs".into(), "Secrets".into()]);
        let layout = resolve(&config);
        let config_section = layout.iter().find(|s| s.name == "Config").unwrap();
        assert_eq!(config_section.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["HPAs", "Secrets", "ConfigMaps"]);
    }

    #[test]
    fn a_layout_survives_a_round_trip_through_the_config() {
        let mut layout = resolve(&OverviewConfig::default());
        let flat = flatten(&layout);
        let workloads_pods = flat.iter().position(|r| r.item == Some(0) && layout[r.section].name == "Workloads").unwrap();
        move_row(&mut layout, flat[workloads_pods], 1).unwrap();
        layout[0].hidden = true;
        layout[2].items[1].hidden = true;
        assert_eq!(resolve(&to_config(&layout)), layout);
    }

    #[test]
    fn moving_stops_at_the_ends_and_stays_inside_a_category() {
        let mut layout = resolve(&OverviewConfig::default());
        assert_eq!(move_row(&mut layout, FlatRow { section: 0, item: None }, -1), None);
        assert_eq!(move_row(&mut layout, FlatRow { section: 0, item: Some(0) }, -1), None, "first kind can't leave its category");
        let last = layout[0].items.len() - 1;
        assert_eq!(move_row(&mut layout, FlatRow { section: 0, item: Some(last) }, 1), None);
        assert!(move_row(&mut layout, FlatRow { section: 0, item: None }, 1).is_some());
        assert_eq!(names(&layout)[..2], ["Workloads", "Cluster"]);
    }

    #[test]
    fn arrange_orders_and_hides_the_live_catalog() {
        let live = vec![("Cluster", vec![("Nodes", 1), ("Namespaces", 7)]), ("Config", vec![("ConfigMaps", 3), ("Secrets", 2)])];
        let config = OverviewConfig { sections: vec!["Config".into()], hidden: vec!["Config/Secrets".into(), "Cluster/Nodes".into()], ..Default::default() };
        let arranged = arrange(live, &config);
        assert_eq!(arranged, vec![("Config", vec![("ConfigMaps", 3)]), ("Cluster", vec![("Namespaces", 7)])]);
    }

    #[test]
    fn a_category_with_every_kind_hidden_disappears() {
        let live = vec![("Config", vec![("ConfigMaps", 3)])];
        let config = OverviewConfig { hidden: vec!["Config/ConfigMaps".into()], ..Default::default() };
        assert!(arrange(live, &config).is_empty());
    }

    #[test]
    fn hiding_everything_is_detected() {
        let mut layout = resolve(&OverviewConfig::default());
        for s in layout.iter_mut() {
            s.hidden = true;
        }
        assert!(!any_visible(&layout));
    }
}
