//! The `:` command line, the sidebar's category layout, and the context-switcher helpers.

use crate::*;

/// The categories and kinds the sidebar lists, the same as the Home catalog.
pub(crate) fn menu_sections(crds: &[k8s::CrdInfo]) -> Vec<ui::MenuSection<'static>> {
    // The whole unfiltered CRD picker, then one tile per discovered API
    // group (`crds` is already sorted by group, so adjacent-dedup keeps
    // order), same shape as the Overview's Custom Resources column.
    let mut custom = vec![ResourceKind::CustomResourceList];
    for crd in crds {
        if custom.last() != Some(&ResourceKind::CustomResourceGroup(crd.group)) {
            custom.push(ResourceKind::CustomResourceGroup(crd.group));
        }
    }
    vec![
        ui::MenuSection { title: "Cluster", tiles: vec![ResourceKind::Overview, ResourceKind::Nodes, ResourceKind::Namespaces, ResourceKind::ApiResources] },
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
            tiles: vec![ResourceKind::Services, ResourceKind::Endpoints, ResourceKind::Ingresses, ResourceKind::NetworkPolicies, ResourceKind::PortForwards],
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
        ui::MenuSection { title: "Helm", tiles: vec![ResourceKind::HelmReleases] },
        ui::MenuSection { title: "Custom Resources", tiles: custom },
    ]
}

/// Autocomplete for the `:` command line: every switchable kind plus `context`,
/// `events` and `quit`, matched against all their names and sorted best-first.
/// An exact alias ranks first; empty input suggests nothing.
pub(crate) fn command_suggestions(input: &str, crds: &[k8s::CrdInfo], apis: &[k8s::ApiInfo]) -> Vec<Suggestion> {
    let input = input.trim().to_lowercase();
    if input.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(i64, Suggestion)> = std::iter::once(Cmd::Context)
        .chain(std::iter::once(Cmd::Events))
        .chain(std::iter::once(Cmd::Theme))
        .chain(std::iter::once(Cmd::Settings))
        .chain(std::iter::once(Cmd::Quit))
        .chain(menu_sections(crds).iter().flat_map(|s| s.tiles.iter().copied()).map(Cmd::Kind))
        // Every other resource the server lists, by plural or kind (`:flowschemas`),
        // unless a built-in kind already answers to that name.
        .chain(apis.iter().enumerate().filter(|(_, a)| ResourceKind::from_command(a.plural).is_none()).map(|(i, a)| Cmd::Api(i, a.plural, a.kind)))
        .filter_map(|cmd| {
            let names = cmd.names();
            let (score, alias) = names
                .iter()
                .filter_map(|alias| {
                    let mut score = fuzzy::score(&input, alias)?;
                    if *alias == input {
                        score += 1000;
                    } else if alias.starts_with(&input) {
                        score += 50;
                    }
                    Some((score, alias.clone()))
                })
                .max_by_key(|(score, _)| *score)?;
            let primary = names[0].clone();
            let label = if alias == primary { primary } else { format!("{primary} ({alias})") };
            Some((score, Suggestion { cmd, label }))
        })
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, suggestion)| suggestion).take(8).collect()
}

/// One line of the autocomplete: what it does, and how it's shown
/// (`namespaces (ns)` when it was found through an alias).
#[derive(Clone)]
pub(crate) struct Suggestion {
    pub(crate) cmd: Cmd,
    pub(crate) label: String,
}

impl Suggestion {
    /// What to show beside it: the kind's icon from the menu, or a drawn
    /// icon for the commands that are not a resource.
    pub(crate) fn icon(&self) -> ui::SuggestionIcon {
        match self.cmd {
            Cmd::Kind(kind) => ui::SuggestionIcon::Kind(kind),
            Cmd::Api(index, plural, _) => ui::SuggestionIcon::Kind(ResourceKind::Api(index, plural)),
            Cmd::Context => ui::SuggestionIcon::Named("switch"),
            Cmd::Events => ui::SuggestionIcon::Named("bell"),
            Cmd::Theme => ui::SuggestionIcon::Named("palette"),
            Cmd::Settings => ui::SuggestionIcon::Named("gear"),
            Cmd::Quit => ui::SuggestionIcon::Named("door"),
        }
    }

    /// The name Tab completes to: the first of its names.
    pub(crate) fn primary_name(&self) -> String {
        self.cmd.names().into_iter().next().unwrap_or_default()
    }
}

/// One entry in the `:` autocomplete, a resource view to switch to, the
/// context switcher, the events browser, or quitting.
#[derive(Clone, Copy)]
pub(crate) enum Cmd {
    Kind(ResourceKind),
    /// A discovered resource: its index in the catalog, plural and kind.
    Api(usize, &'static str, &'static str),
    Context,
    Events,
    Theme,
    Settings,
    Quit,
}

impl Cmd {
    /// Every lowercase name that runs this command, the one the
    /// autocomplete shows first.
    pub(crate) fn names(self) -> Vec<String> {
        let fixed = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
        match self {
            Cmd::Kind(k) => {
                let aliases = k.aliases();
                if aliases.is_empty() { vec![k.label().to_lowercase().replace(' ', "")] } else { fixed(aliases) }
            }
            Cmd::Api(_, plural, kind) => {
                let mut names = vec![plural.to_string()];
                if !kind.eq_ignore_ascii_case(plural) {
                    names.push(kind.to_lowercase());
                }
                names
            }
            Cmd::Context => fixed(&["context", "contexts", "ctx"]),
            Cmd::Events => fixed(&["events", "event", "ev"]),
            Cmd::Settings => fixed(&["config", "settings", "preferences", "prefs", "options"]),
            Cmd::Theme => fixed(&["theme", "themes", "skin", "skins", "colors", "colours"]),
            Cmd::Quit => fixed(&["quit", "q", "exit"]),
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
    // Always "editing": there's no separate typing mode here, letters filter
    // immediately (see the Context handler), so this just keeps global
    // shortcuts and the search-box cursor active the whole time it's open.
    *mode = Mode::Context { contexts, filter: String::new(), editing: true, state: TableState::default().with_selected(0), error: None, sort: ListSort::default(), back };
}

/// The key picker for `namespace`, starting on the key it already has, or
/// else the first free one. Its `back` is the plain list; callers that came
/// from somewhere else replace it.
pub(crate) fn key_picker(namespace: String, favorites: &Favorites) -> Mode {
    let selected = favorites.key_of(&namespace).map(|k| k - 1).or_else(|| favorites.slots.iter().position(Option::is_none)).unwrap_or(0);
    Mode::Slots { namespace, selected, back: Box::new(Mode::List) }
}

/// Opens the namespace picker (`n` from any view but the Namespaces list).
pub(crate) fn open_namespace_picker(mode: &mut Mode, names: Vec<String>) {
    let back = Box::new(std::mem::replace(mode, Mode::List));
    *mode = Mode::NamespacePick { names, filter: String::new(), editing: false, state: TableState::default().with_selected(0), sort: ListSort::default(), back };
}

/// Namespaces matching the picker's filter, best match first.
pub(crate) fn filtered_names<'a>(names: &'a [String], filter: &str, sort: ListSort, favorites: &Favorites) -> Vec<&'a String> {
    let mut scored: Vec<(i64, &String)> = names.iter().filter_map(|n| fuzzy::score(filter, n).map(|s| (s, n))).collect();
    scored.sort_by_key(|(score, name)| (std::cmp::Reverse(*score), (*name).clone()));
    let mut shown: Vec<&String> = scored.into_iter().map(|(_, n)| n).collect();
    apply(&mut shown, sort.spec, |name, column| namespace_key(name, favorites.key_of(name), column));
    shown
}

/// Contexts matching the browser's filter, best match first.
pub(crate) fn filtered_contexts<'a>(contexts: &'a [k8s::ContextInfo], filter: &str, sort: ListSort) -> Vec<&'a k8s::ContextInfo> {
    let mut scored: Vec<(i64, &k8s::ContextInfo)> =
        contexts.iter().filter_map(|c| fuzzy::score(filter, &c.name).map(|s| (s, c))).collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    let mut shown: Vec<&k8s::ContextInfo> = scored.into_iter().map(|(_, c)| c).collect();
    apply(&mut shown, sort.spec, |c, column| context_key(c, column));
    shown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Step;

    fn top(input: &str) -> Suggestion {
        command_suggestions(input, &[], &[]).into_iter().next().unwrap_or_else(|| panic!("no suggestion for {input:?}"))
    }

    #[test]
    fn an_exact_alias_ranks_first_and_shows_the_full_name() {
        let s = top("ns");
        assert!(matches!(s.cmd, Cmd::Kind(ResourceKind::Namespaces)));
        assert_eq!(s.label, "namespaces (ns)");
        assert!(matches!(top("dp").cmd, Cmd::Kind(ResourceKind::Deployments)));
        assert!(matches!(top("po").cmd, Cmd::Kind(ResourceKind::Pods)));
        assert!(matches!(top("svc").cmd, Cmd::Kind(ResourceKind::Services)));
        assert!(matches!(top("sa").cmd, Cmd::Kind(ResourceKind::ServiceAccounts)));
    }

    #[test]
    fn the_full_name_shows_without_an_alias_suffix() {
        assert_eq!(top("pods").label, "pods");
        assert_eq!(top("namespaces").label, "namespaces");
    }

    #[test]
    fn quit_context_and_events_are_commands_too() {
        for input in ["q", "quit", "exit"] {
            assert!(matches!(top(input).cmd, Cmd::Quit), "{input}");
        }
        assert!(matches!(top("ctx").cmd, Cmd::Context));
        assert!(matches!(top("context").cmd, Cmd::Context));
        assert!(matches!(top("ev").cmd, Cmd::Events));
    }

    #[test]
    fn namespace_filter_ranks_matches_and_keeps_alphabetical_order_when_empty() {
        let names: Vec<String> = ["kube-system", "default", "kube-public"].iter().map(|s| s.to_string()).collect();
        assert_eq!(filtered_names(&names, "", ListSort::default(), &Favorites::empty()).into_iter().cloned().collect::<Vec<_>>(), ["default", "kube-public", "kube-system"]);
        assert_eq!(filtered_names(&names, "sys", ListSort::default(), &Favorites::empty()).into_iter().cloned().collect::<Vec<_>>(), ["kube-system"]);
        assert!(filtered_names(&names, "zzz", ListSort::default(), &Favorites::empty()).is_empty());
    }

    #[test]
    fn empty_input_suggests_nothing() {
        assert!(command_suggestions("  ", &[], &[]).is_empty());
    }

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
            ResourceKind::HelmReleases,
            ResourceKind::PortForwards,
            ResourceKind::ApiResources,
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
    pub(crate) fn empty_search_matches_everything() {
        assert!(row_matches("", "anything at all"));
    }

    #[test]
    pub(crate) fn search_narrows_to_fuzzy_matches_only() {
        assert!(row_matches("traefik", "kube-system traefik-9bcdbbd9-x2767"));
        assert!(!row_matches("traefik", "kube-system coredns-8db54c48d-nhwx7"));
    }

    #[test]
    pub(crate) fn path_of_a_plain_list_is_one_segment() {
        assert_eq!(full_path(&Mode::List, location(ResourceKind::Pods, &[], None)), vec![plain_segment("Pods")]);
    }

    #[test]
    fn a_popup_naming_the_selected_thing_replaces_its_list() {
        let node_detail = Mode::NodeDetail { name: "worker-1".into(), state: TableState::default(), sort: ListSort::default(), search: String::new(), editing: false, back: Box::new(Mode::List) };
        let rendered: Vec<String> = full_path(&node_detail, location(ResourceKind::Nodes, &[], None))
            .into_iter()
            .map(|s| match s.value { Some(v) => format!("{}[{v}]", s.kind), None => s.kind })
            .collect();
        assert_eq!(rendered.join(">>"), "Node[worker-1]");
    }

    #[test]
    fn location_names_each_drilled_thing_once_then_the_list() {
        let deployment = Scope::Owner { uid: "d".into(), kind: "Deployment".into(), name: "web".into() };
        let replicaset = Scope::Owner { uid: "r".into(), kind: "ReplicaSet".into(), name: "web-5d9d".into() };
        let trail = [Step::List(ResourceKind::Deployments, None, 0), Step::List(ResourceKind::ReplicaSets, Some(deployment.clone()), 1)];
        let render = |segments: Vec<ui::PathSegment>| -> String {
            segments.into_iter().map(|s| match s.value { Some(v) => format!("{}[{v}]", s.kind), None => s.kind }).collect::<Vec<_>>().join(">>")
        };
        assert_eq!(render(location(ResourceKind::Pods, &trail, Some(&replicaset))), "Deployment[web]>>ReplicaSet[web-5d9d]>>Pods");
        assert_eq!(render(location(ResourceKind::ReplicaSets, &trail[..1], Some(&deployment))), "Deployment[web]>>ReplicaSets");
        assert_eq!(render(location(ResourceKind::Nodes, &[], None)), "Nodes");
        let ns = Scope::Namespace { name: "kube-system".into() };
        assert_eq!(render(location(ResourceKind::Pods, &[Step::List(ResourceKind::Namespaces, None, 0)], Some(&ns))), "Namespace[kube-system]>>Pods");
    }

    #[test]
    pub(crate) fn path_walks_the_whole_back_chain_oldest_first() {
        let node_detail = Mode::NodeDetail { name: "worker-1".into(), state: TableState::default(), sort: ListSort::default(), search: String::new(), editing: false, back: Box::new(Mode::List) };
        let containers = Mode::Containers {
            title: "default/web-1".into(),
            namespace: "default".into(),
            pod: "web-1".into(),
            containers: vec![],
            state: TableState::default(),
            sort: ListSort::default(),
            back: Box::new(node_detail),
        };
        let rendered: Vec<String> = full_path(&containers, location(ResourceKind::Overview, &[], None))
            .into_iter()
            .map(|s| match s.value {
                Some(v) => format!("{}[{v}]", s.kind),
                None => s.kind,
            })
            .collect();
        assert_eq!(rendered.join(">>"), "Home>>Node[worker-1]>>Pod[default/web-1]");
    }
}
