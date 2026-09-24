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

/// The full layout, hidden entries included, in the configured order. `live`
/// is every category and its kinds as the catalog currently has them —
/// built-ins plus whatever's enabled (Helm, Flux, ...) — so a category that
/// only exists once its extension is turned on still shows up here to be
/// reordered or hidden, not just the fixed built-in set. Falls back to
/// `DEFAULT_LAYOUT` only when `live` is empty (e.g. before the catalog's
/// first read).
pub fn resolve(config: &OverviewConfig, live: &[(&str, Vec<&str>)]) -> Vec<LayoutSection> {
    let fallback: Vec<(&str, Vec<&str>)> = DEFAULT_LAYOUT.iter().map(|(n, items)| (*n, items.to_vec())).collect();
    let source = if live.is_empty() { &fallback } else { live };
    let mut sections: Vec<LayoutSection> = source
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

/// Puts the category at `from` at place `to` (both 0-based), shifting the others.
pub fn move_section_to(layout: &mut Vec<LayoutSection>, from: usize, to: usize) -> usize {
    let to = to.min(layout.len().saturating_sub(1));
    let section = layout.remove(from);
    layout.insert(to, section);
    to
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
        let layout = resolve(&OverviewConfig::default(), &[]);
        assert_eq!(names(&layout)[..3], ["Cluster", "Workloads", "Config"]);
        assert!(!any_hidden(&layout));
    }

    fn any_hidden(layout: &[LayoutSection]) -> bool {
        layout.iter().any(|s| s.hidden || s.items.iter().any(|i| i.hidden))
    }

    #[test]
    fn listed_names_come_first_and_the_rest_keep_their_order() {
        let config = OverviewConfig { sections: vec!["Storage".into(), "Cluster".into()], ..Default::default() };
        let layout = resolve(&config, &[]);
        assert_eq!(names(&layout)[..4], ["Storage", "Cluster", "Workloads", "Config"]);
    }

    #[test]
    fn items_are_ordered_within_their_category() {
        let mut config = OverviewConfig::default();
        config.items.insert("Config".into(), vec!["HPAs".into(), "Secrets".into()]);
        let layout = resolve(&config, &[]);
        let config_section = layout.iter().find(|s| s.name == "Config").unwrap();
        assert_eq!(config_section.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["HPAs", "Secrets", "ConfigMaps"]);
    }

    #[test]
    fn a_layout_survives_a_round_trip_through_the_config() {
        let mut layout = resolve(&OverviewConfig::default(), &[]);
        layout.swap(1, 2);
        layout[0].hidden = true;
        layout[2].items[1].hidden = true;
        assert_eq!(resolve(&to_config(&layout), &[]), layout);
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
    fn a_category_that_only_exists_once_enabled_is_still_editable() {
        // Helm, Flux, ... aren't in DEFAULT_LAYOUT: they only exist in what the
        // catalog currently has, so the editor has to read that, not the fixed set.
        let live: Vec<(&str, Vec<&str>)> = vec![("Cluster", vec!["Nodes"]), ("Helm", vec!["Helm Releases"])];
        let layout = resolve(&OverviewConfig::default(), &live);
        assert_eq!(names(&layout), ["Cluster", "Helm"]);
        let helm = layout.iter().find(|s| s.name == "Helm").unwrap();
        assert_eq!(helm.items[0].name, "Helm Releases");
    }

    #[test]
    fn a_category_can_be_put_at_a_numbered_place() {
        let mut layout = resolve(&OverviewConfig::default(), &[]);
        assert_eq!(move_section_to(&mut layout, 3, 0), 0);
        assert_eq!(names(&layout)[..3], ["Network", "Cluster", "Workloads"]);
        assert_eq!(move_section_to(&mut layout, 0, 99), layout.len() - 1, "clamped to the last place");
    }

    #[test]
    fn hiding_everything_is_detected() {
        let mut layout = resolve(&OverviewConfig::default(), &[]);
        for s in layout.iter_mut() {
            s.hidden = true;
        }
        assert!(!any_visible(&layout));
    }
}
