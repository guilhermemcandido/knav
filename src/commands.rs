//! The `:` command line, the `m` menu's layout, and the context-switcher helpers.

use super::*;

/// The `m` menu's layout — same six categories as the Overview catalog.
/// One shared function so the popup's render pass and its keyboard/Enter
/// handling can't drift apart.
pub(crate) fn menu_sections(crds: &[k8s::CrdInfo]) -> Vec<ui::MenuSection<'static>> {
    // The whole unfiltered CRD picker, then one tile per discovered API
    // group (`crds` is already sorted by group, so adjacent-dedup keeps
    // order) — same shape as the Overview's Custom Resources column.
    let mut custom = vec![ResourceKind::CustomResourceList];
    for crd in crds {
        if custom.last() != Some(&ResourceKind::CustomResourceGroup(crd.group)) {
            custom.push(ResourceKind::CustomResourceGroup(crd.group));
        }
    }
    vec![
        ui::MenuSection { title: "Cluster", tiles: vec![ResourceKind::Overview, ResourceKind::Nodes, ResourceKind::Namespaces] },
        ui::MenuSection {
            title: "Workloads",
            tiles: vec![
                ResourceKind::Pods,
                ResourceKind::Deployments,
                ResourceKind::ReplicaSets,
                ResourceKind::StatefulSets,
                ResourceKind::DaemonSets,
                ResourceKind::Jobs,
                ResourceKind::CronJobs,
            ],
        },
        ui::MenuSection { title: "Config", tiles: vec![ResourceKind::ConfigMaps, ResourceKind::Secrets, ResourceKind::Hpas] },
        ui::MenuSection {
            title: "Network",
            tiles: vec![ResourceKind::Services, ResourceKind::Endpoints, ResourceKind::Ingresses, ResourceKind::NetworkPolicies],
        },
        ui::MenuSection { title: "Storage", tiles: vec![ResourceKind::Pvcs, ResourceKind::Pvs, ResourceKind::StorageClasses] },
        ui::MenuSection {
            title: "Access Control",
            tiles: vec![
                ResourceKind::ServiceAccounts,
                ResourceKind::Roles,
                ResourceKind::RoleBindings,
                ResourceKind::ClusterRoles,
                ResourceKind::ClusterRoleBindings,
            ],
        },
        ui::MenuSection { title: "Custom Resources", tiles: custom },
    ]
}

/// Live autocomplete for the `:` command line — every switchable
/// resource kind, fuzzy-scored against whatever's typed so far and
/// sorted best-first, same scorer the search/filter and cluster picker
/// already use. Empty input suggests nothing (an empty command bar with
/// a giant list under it isn't "autocomplete," it's just the menu).
pub(crate) fn command_suggestions(input: &str, crds: &[k8s::CrdInfo]) -> Vec<Cmd> {
    if input.trim().is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(i64, Cmd)> = std::iter::once(Cmd::Context)
        .chain(menu_sections(crds).iter().flat_map(|s| s.tiles.iter().copied()).map(Cmd::Kind))
        .filter_map(|cmd| fuzzy::score(input, &cmd.name()).map(|score| (score, cmd)))
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, cmd)| cmd).take(8).collect()
}

/// One entry in the `:` autocomplete — a resource view to switch to, or
/// the context switcher.
#[derive(Clone, Copy)]
pub(crate) enum Cmd {
    Kind(ResourceKind),
    Context,
}

impl Cmd {
    /// The lowercase name you'd type (`pods`, `configmaps`, `context`) —
    /// what the autocomplete shows and matches against.
    pub(crate) fn name(self) -> String {
        match self {
            Cmd::Kind(k) => k.label().to_lowercase().replace(' ', ""),
            Cmd::Context => "context".to_string(),
        }
    }
}

/// Whether a typed `:` command is the context switcher (`:ctx`, ...).
pub(crate) fn is_context_command(cmd: &str) -> bool {
    matches!(cmd, "ctx" | "context" | "contexts")
}

/// Opens the context switcher, listing every kubeconfig context with
/// the one actually connected marked as current (the kubeconfig's own
/// `current-context` can differ, e.g. after `-c`).
pub(crate) fn open_context_switcher(mode: &mut Mode, active_context: &str) {
    let mut contexts = k8s::list_contexts().unwrap_or_default();
    for c in &mut contexts {
        c.is_current = c.name == active_context;
    }
    let back = Box::new(std::mem::replace(mode, Mode::List));
    *mode = Mode::Context { contexts, filter: String::new(), editing: false, state: TableState::default().with_selected(0), error: None, back };
}

/// Whether choosing `name` should reconnect: `Ok(false)` if it's already
/// the connected context, `Err` (a one-line reason) if it can't be
/// reached. Checked *before* tearing the session down, so a dead
/// cluster leaves you where you are instead of nowhere.
pub(crate) fn switch_target(name: &str, active_context: &str) -> std::result::Result<bool, String> {
    if name == active_context {
        return Ok(false);
    }
    let check = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            let client = k8s::connect_to_context(Some(name)).await?;
            k8s::ensure_reachable(&client, Some(name)).await
        })
    });
    match check {
        Ok(_) => Ok(true),
        Err(e) => Err(e.to_string().lines().next().unwrap_or("connection failed").to_string()),
    }
}

/// Contexts matching the browser's filter, best match first.
pub(crate) fn filtered_contexts<'a>(contexts: &'a [k8s::ContextInfo], filter: &str) -> Vec<&'a k8s::ContextInfo> {
    let mut scored: Vec<(i64, &k8s::ContextInfo)> =
        contexts.iter().filter_map(|c| fuzzy::score(filter, &c.name).map(|s| (s, c))).collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, c)| c).collect()
}

/// Where a `ResourceKind` sits in the menu grid, so opening the menu
/// starts with the currently-viewed kind selected instead of always
/// resetting to the top-left tile.
pub(crate) fn menu_position_for(kind: ResourceKind, crds: &[k8s::CrdInfo]) -> (usize, usize) {
    let sections = menu_sections(crds);
    for (section_idx, section) in sections.iter().enumerate() {
        if let Some(tile_idx) = section.tiles.iter().position(|k| *k == kind) {
            return (section_idx, tile_idx);
        }
    }
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub(crate) fn next_stops_at_bottom_instead_of_wrapping() {
        let mut state = TableState::default().with_selected(0);
        for _ in 0..5 {
            select_next(&mut state, 3);
        }
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    pub(crate) fn prev_stops_at_top_instead_of_wrapping() {
        let mut state = TableState::default().with_selected(1);
        select_prev(&mut state, 3);
        select_prev(&mut state, 3);
        select_prev(&mut state, 3);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    pub(crate) fn empty_list_does_not_panic_or_select() {
        let mut state = TableState::default();
        select_next(&mut state, 0);
        select_prev(&mut state, 0);
        assert_eq!(state.selected(), None);
    }

    #[test]
    pub(crate) fn single_item_list_stays_put() {
        let mut state = TableState::default().with_selected(0);
        select_next(&mut state, 1);
        assert_eq!(state.selected(), Some(0));
        select_prev(&mut state, 1);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    pub(crate) fn menu_sections_cover_every_resource_kind_exactly_once() {
        let expected = [
            ResourceKind::Overview,
            ResourceKind::Nodes,
            ResourceKind::Namespaces,
            ResourceKind::Pods,
            ResourceKind::Deployments,
            ResourceKind::ReplicaSets,
            ResourceKind::StatefulSets,
            ResourceKind::DaemonSets,
            ResourceKind::Jobs,
            ResourceKind::CronJobs,
            ResourceKind::ConfigMaps,
            ResourceKind::Secrets,
            ResourceKind::Hpas,
            ResourceKind::Services,
            ResourceKind::Endpoints,
            ResourceKind::Ingresses,
            ResourceKind::NetworkPolicies,
            ResourceKind::Pvcs,
            ResourceKind::Pvs,
            ResourceKind::StorageClasses,
            ResourceKind::ServiceAccounts,
            ResourceKind::Roles,
            ResourceKind::RoleBindings,
            ResourceKind::ClusterRoles,
            ResourceKind::ClusterRoleBindings,
            ResourceKind::CustomResourceList,
        ];
        let sections = menu_sections(&[]);
        let total: usize = sections.iter().map(|s| s.tiles.len()).sum();
        assert_eq!(total, expected.len(), "a kind is missing from (or duplicated in) the menu");
        for kind in expected {
            assert!(sections.iter().any(|s| s.tiles.contains(&kind)), "{} missing from menu_sections", kind.label());
        }
    }

    #[test]
    pub(crate) fn menu_lists_one_tile_per_crd_group() {
        let crd = |group: &'static str, kind: &'static str| k8s::CrdInfo {
            group,
            kind,
            plural: kind.to_lowercase(),
            version: "v1".into(),
            namespaced: true,
        };
        let crds = [crd("a.io", "One"), crd("a.io", "Two"), crd("b.io", "Three")];
        let sections = menu_sections(&crds);
        let custom = &sections.last().unwrap().tiles;
        assert_eq!(
            custom,
            &[ResourceKind::CustomResourceList, ResourceKind::CustomResourceGroup("a.io"), ResourceKind::CustomResourceGroup("b.io")]
        );
    }

    #[test]
    pub(crate) fn menu_position_for_finds_the_matching_tile() {
        let sections = menu_sections(&[]);
        let pos = menu_position_for(ResourceKind::ConfigMaps, &[]);
        assert_eq!(sections[pos.0].tiles[pos.1], ResourceKind::ConfigMaps);
    }

    #[test]
    pub(crate) fn empty_search_matches_everything() {
        assert!(row_matches("", "anything at all"));
    }

    #[test]
    pub(crate) fn search_narrows_to_fuzzy_matches_only() {
        assert!(row_matches("traefik", "kube-system traefik-9bcdbbd9-x2767"));
        assert!(!row_matches("traefik", "kube-system coredns-8db54c48d-nhwx7"));
    }

    #[test]
    pub(crate) fn breadcrumb_is_none_for_plain_list() {
        assert_eq!(breadcrumb(&Mode::List, ResourceKind::Pods), None);
    }

    #[test]
    pub(crate) fn breadcrumb_walks_the_whole_back_chain_oldest_first() {
        let node_detail = Mode::NodeDetail { name: "worker-1".into(), state: TableState::default(), back: Box::new(Mode::List) };
        let containers = Mode::Containers {
            title: "default/web-1".into(),
            namespace: "default".into(),
            pod: "web-1".into(),
            containers: vec![],
            state: TableState::default(),
            back: Box::new(node_detail),
        };
        let rendered: Vec<String> = breadcrumb(&containers, ResourceKind::Overview)
            .unwrap()
            .into_iter()
            .map(|s| match s.value {
                Some(v) => format!("{}[{v}]", s.kind),
                None => s.kind,
            })
            .collect();
        assert_eq!(rendered.join(">>"), "Overview>>Node[worker-1]>>Pod[default/web-1]");
    }
}
