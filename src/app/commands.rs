//! The `:` command line, the command menu's sections, and the context and namespace pickers.

use super::*;

/// The command menu's sections, the same as the Overview's. `dashboard_categories`
/// comes from the caller, which has the loaded extensions.
pub(crate) fn menu_sections(crds: &[k8s::CrdInfo], dashboard_categories: &[&'static str]) -> Vec<ui::MenuSection<'static>> {
    // The whole CRD picker, then one tile per API group, like the Overview's column.
    let mut custom = vec![ResourceKind::CustomResourceList];
    for crd in crds {
        if custom.last() != Some(&ResourceKind::CustomResourceGroup(crd.group)) {
            custom.push(ResourceKind::CustomResourceGroup(crd.group));
        }
    }
    let dashboards = dashboard_categories.iter().copied().map(|category| ui::MenuSection { title: category, tiles: vec![ResourceKind::ExtensionDashboard(category)] });
    let mut sections = vec![
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
    ];
    sections.extend(dashboards);
    sections.push(ui::MenuSection { title: "CustomResources", tiles: custom });
    sections
}

/// Autocomplete for the `:` command line: every kind and command, matched on all their
/// names, best first. An exact alias ranks first; empty input suggests nothing.
/// `custom` is the names of your own commands, in config order. A custom resource
/// shows under its API group, as a small tree.
pub(crate) fn command_suggestions(input: &str, crds: &[k8s::CrdInfo], apis: &[k8s::ApiInfo], dashboard_categories: &[&'static str], custom: &[String]) -> Vec<Suggestion> {
    let input = input.trim().to_lowercase();
    if input.is_empty() {
        return Vec::new();
    }
    let names_of = |cmd: Cmd| match cmd {
        Cmd::Custom(i) => vec![custom[i].to_lowercase()],
        Cmd::Crd(i) => {
            let crd = &crds[i];
            let mut names = vec![crd.plural.to_lowercase()];
            if !crd.kind.eq_ignore_ascii_case(&crd.plural) {
                names.push(crd.kind.to_lowercase());
            }
            names
        }
        _ => cmd.names(),
    };
    let mut scored: Vec<(i64, Suggestion)> = std::iter::once(Cmd::Context)
        .chain(std::iter::once(Cmd::Events))
        .chain(std::iter::once(Cmd::Problems))
        .chain(std::iter::once(Cmd::Theme))
        .chain(std::iter::once(Cmd::Settings))
        .chain(std::iter::once(Cmd::Quit))
        .chain((0..custom.len()).map(Cmd::Custom))
        .chain(menu_sections(crds, dashboard_categories).iter().flat_map(|s| s.tiles.iter().copied()).map(Cmd::Kind))
        .chain((0..crds.len()).map(Cmd::Crd))
        // Every other resource the server lists, unless a built-in kind has that name.
        .chain(apis.iter().enumerate().filter(|(_, a)| ResourceKind::from_command(a.plural).is_none() && !crds.iter().any(|c| c.group == a.group && c.plural == a.plural)).map(|(i, a)| Cmd::Api(i, a.plural, a.kind)))
        .filter_map(|cmd| {
            let names = names_of(cmd);
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
            let label = if alias == primary { primary.clone() } else { format!("{primary} ({alias})") };
            let group = match cmd {
                Cmd::Crd(i) => Some(crds[i].group),
                Cmd::Kind(ResourceKind::CustomResourceGroup(_)) => Some("API group"),
                _ => None,
            };
            Some((score, Suggestion { cmd, label, primary, group }))
        })
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    let ranked: Vec<Suggestion> = scored.into_iter().map(|(_, suggestion)| suggestion).take(10).collect();
    with_group_kinds(ranked, crds, &names_of)
}

/// A matched CRD group is followed by its kinds, so typing a group shows what is in it.
fn with_group_kinds(ranked: Vec<Suggestion>, crds: &[k8s::CrdInfo], names_of: &dyn Fn(Cmd) -> Vec<String>) -> Vec<Suggestion> {
    const KINDS_UNDER_A_GROUP: usize = 6;
    let mut out: Vec<Suggestion> = Vec::new();
    for suggestion in ranked {
        let Cmd::Kind(ResourceKind::CustomResourceGroup(group)) = suggestion.cmd else {
            if !out.iter().any(|s| matches!((s.cmd, suggestion.cmd), (Cmd::Crd(a), Cmd::Crd(b)) if a == b)) {
                out.push(suggestion);
            }
            continue;
        };
        out.push(suggestion);
        for i in (0..crds.len()).filter(|i| crds[*i].group == group).take(KINDS_UNDER_A_GROUP) {
            if !out.iter().any(|s| matches!(s.cmd, Cmd::Crd(j) if j == i)) {
                let primary = names_of(Cmd::Crd(i)).remove(0);
                out.push(Suggestion { cmd: Cmd::Crd(i), label: primary.clone(), primary, group: Some(group) });
            }
        }
    }
    out
}

/// One autocomplete line: what it does, and its label (`namespaces (ns)` when found by alias).
#[derive(Clone)]
pub(crate) struct Suggestion {
    pub(crate) cmd: Cmd,
    pub(crate) label: String,
    /// The name Tab completes to.
    pub(crate) primary: String,
    /// The API group a custom resource kind is from, shown beside it ("API group" on
    /// a group's own row).
    pub(crate) group: Option<&'static str>,
}

impl Suggestion {
    /// The kind's icon, or a drawn icon for commands that aren't a resource.
    pub(crate) fn icon(&self, crds: &[k8s::CrdInfo]) -> ui::SuggestionIcon {
        match self.cmd {
            Cmd::Kind(kind) => ui::SuggestionIcon::Kind(kind),
            Cmd::Crd(index) => ui::SuggestionIcon::Kind(ResourceKind::CustomResource(index, crds.get(index).map_or("", |c| c.kind))),
            Cmd::Api(index, plural, _) => ui::SuggestionIcon::Kind(ResourceKind::Api(index, plural)),
            Cmd::Context => ui::SuggestionIcon::Named("switch"),
            Cmd::Events => ui::SuggestionIcon::Named("bell"),
            Cmd::Problems => ui::SuggestionIcon::Named("bell"),
            Cmd::Custom(_) => ui::SuggestionIcon::Named("gear"),
            Cmd::Theme => ui::SuggestionIcon::Named("palette"),
            Cmd::Settings => ui::SuggestionIcon::Named("gear"),
            Cmd::Quit => ui::SuggestionIcon::Named("door"),
        }
    }

    /// The name Tab completes to: the first of its names.
    pub(crate) fn primary_name(&self) -> String {
        self.primary.clone()
    }
}

/// One `:` command: a view to switch to, the context switcher, the events, or quit.
#[derive(Clone, Copy)]
pub(crate) enum Cmd {
    Kind(ResourceKind),
    /// A discovered resource: its index in the catalog, plural and kind.
    Api(usize, &'static str, &'static str),
    Context,
    Events,
    Problems,
    Theme,
    Settings,
    Quit,
    /// One of your own commands, by its index in the config.
    Custom(usize),
    /// A custom resource kind, by its index in the catalog's CRDs.
    Crd(usize),
}

impl Cmd {
    /// Every lowercase name that runs this command, the one autocomplete shows first.
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
            Cmd::Problems => fixed(&["problems", "problem", "issues", "faults"]),
            // Named from the config and the CRDs, which `command_suggestions` has.
            Cmd::Custom(_) | Cmd::Crd(_) => Vec::new(),
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

/// Opens the context switcher, marking the context actually connected as current,
/// which can differ from the kubeconfig's after `-c`.
pub(crate) fn open_context_switcher(mode: &mut Mode, active_context: &str) {
    let mut contexts = k8s::list_contexts().unwrap_or_default();
    for c in &mut contexts {
        c.is_current = c.name == active_context;
    }
    let back = Box::new(std::mem::replace(mode, Mode::List));
    // Always editing: letters filter at once, with no separate typing mode.
    *mode = Mode::Context { contexts, filter: String::new(), editing: true, state: TableState::default().with_selected(0), error: None, back };
}

/// The key picker for `namespace`, on the key it has or the first free one. Its `back`
/// is the plain list; callers from elsewhere replace it.
pub(crate) fn key_picker(namespace: String, favorites: &Favorites) -> Mode {
    let selected = favorites.key_of(&namespace).map(|k| k - 1).or_else(|| favorites.slots.iter().position(Option::is_none)).unwrap_or(0);
    Mode::Slots { namespace, selected, back: Box::new(Mode::List) }
}

/// Opens the namespace picker (`n`).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Step;

    fn top(input: &str) -> Suggestion {
        command_suggestions(input, &[], &[], &[], &[]).into_iter().next().unwrap_or_else(|| panic!("no suggestion for {input:?}"))
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
        assert!(command_suggestions("  ", &[], &[], &[], &[]).is_empty());
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
        let mut expected = vec![
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
        let registry = extensions::Registry::load(std::path::Path::new("/nonexistent-knav-test-dir"));
        let dashboard_categories = extensions::dashboards::categories(&registry);
        expected.extend(dashboard_categories.iter().copied().map(ResourceKind::ExtensionDashboard));
        let sections = menu_sections(&[], &dashboard_categories);
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
        let sections = menu_sections(&crds, &[]);
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

    fn crd(group: &'static str, kind: &'static str, plural: &str) -> k8s::CrdInfo {
        k8s::CrdInfo { group, kind, plural: plural.into(), version: "v1".into(), namespaced: true }
    }

    #[test]
    fn a_custom_kind_is_found_with_its_group() {
        let crds = [crd("platform.example.com", "Environment", "environments"), crd("platform.example.com", "Team", "teams")];
        let first = command_suggestions("environ", &crds, &[], &[], &[]).into_iter().next().unwrap();
        assert!(matches!(first.cmd, Cmd::Crd(0)));
        assert_eq!(first.group, Some("platform.example.com"));
    }

    #[test]
    fn a_matched_group_lists_its_kinds() {
        let crds = [crd("platform.example.com", "Environment", "environments"), crd("platform.example.com", "Team", "teams")];
        let found = command_suggestions("platform.example", &crds, &[], &[], &[]);
        let at = found.iter().position(|s| s.label == "platform.example.com").expect("the group row");
        let kinds: Vec<&str> = found[at + 1..].iter().take_while(|s| s.group == Some("platform.example.com")).map(|s| s.label.as_str()).collect();
        assert_eq!(kinds, ["environments", "teams"]);
    }
}
