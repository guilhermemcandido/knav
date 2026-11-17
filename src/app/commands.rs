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
/// The working extensions that own `cmd`: a dashboard's category, or a custom resource
/// kind one of them declares.
fn owning<'a>(cmd: Cmd, crds: &'a [k8s::CrdInfo], extensions: &'a [crate::extensions::Loaded]) -> impl Iterator<Item = &'a crate::extensions::Loaded> {
    extensions.iter().filter(|l| l.error.is_none()).filter(move |l| match cmd {
        Cmd::Kind(ResourceKind::ExtensionDashboard(category)) => l.kinds.iter().any(|k| k.category == category),
        Cmd::Crd(i) => crds.get(i).is_some_and(|c| l.kinds.iter().any(|k| k.group == c.group && k.kind == c.kind)),
        _ => false,
    })
}

/// The names of the extensions that own `cmd`, as shown beside it.
fn owners(cmd: Cmd, crds: &[k8s::CrdInfo], extensions: &[crate::extensions::Loaded]) -> Vec<String> {
    // knav's own view of Helm's releases, not a resource type of any extension.
    if matches!(cmd, Cmd::Kind(ResourceKind::HelmReleases)) {
        return vec!["Helm · installed releases".into()];
    }
    owning(cmd, crds, extensions).map(|l| l.name.clone()).collect()
}

/// What `ext:` matches against: the ids and names of the extensions that own `cmd`.
fn owner_keys(cmd: Cmd, crds: &[k8s::CrdInfo], extensions: &[crate::extensions::Loaded]) -> Vec<String> {
    if matches!(cmd, Cmd::Kind(ResourceKind::HelmReleases)) {
        return vec!["helm".into()];
    }
    owning(cmd, crds, extensions).flat_map(|l| [l.id.to_lowercase(), l.name.to_lowercase()]).collect()
}

/// How well `pattern` matches `text`, the way the `/` search matches: the text as typed, in one
/// piece and in any case, ranked by how near the start it is, or `=name` for exactly that name.
/// Not the letters scattered anywhere in order, which would let `ingress` match
/// `certificatesigningrequests`.
fn name_score(pattern: &str, text: &str) -> Option<i64> {
    match pattern.strip_prefix('=') {
        Some(exact) => text.eq_ignore_ascii_case(exact).then_some(1000),
        None => fuzzy::contains_score(pattern, text),
    }
}

/// The commands left out of the suggestions because they have keys of their own.
fn hidden_command_of(cmd: Cmd) -> Option<Cmd> {
    matches!(cmd, Cmd::Context | Cmd::Theme | Cmd::Settings | Cmd::Kind(ResourceKind::Overview)).then_some(cmd)
}

/// The hidden command `typed` names exactly (`ctx`, `theme`, `config`, `home`), which still runs.
pub(crate) fn hidden_command(typed: &str) -> Option<Cmd> {
    let typed = typed.trim().to_lowercase();
    [Cmd::Context, Cmd::Theme, Cmd::Settings, Cmd::Kind(ResourceKind::Overview)].into_iter().find(|cmd| cmd.names().contains(&typed))
}

/// What is typed: a name to find, plus filters that accumulate as `key:value` words, the way
/// GitHub's search takes them (`agents group:kagent ext:flux is:custom`).
#[derive(Default)]
struct Query {
    name: String,
    /// `group:` (or `g:`): the API group must match.
    groups: Vec<String>,
    /// `ext:` (or `e:`): an extension that provides the kind, by id or name.
    exts: Vec<String>,
    /// `is:builtin` or `is:custom`.
    is: Vec<String>,
}

impl Query {
    fn has_filters(&self) -> bool {
        !(self.groups.is_empty() && self.exts.is_empty() && self.is.is_empty())
    }
}

fn parse_query(raw: &str) -> Query {
    let mut query = Query::default();
    for word in raw.to_lowercase().split_whitespace() {
        match word.split_once(':') {
            // A filter with nothing after the colon yet is still being typed.
            Some(("group" | "g", value)) => query.groups.extend(Some(value).filter(|v| !v.is_empty()).map(str::to_string)),
            Some(("ext" | "e", value)) => query.exts.extend(Some(value).filter(|v| !v.is_empty()).map(str::to_string)),
            Some(("is", value)) => query.is.extend(Some(value).filter(|v| !v.is_empty()).map(str::to_string)),
            _ if query.name.is_empty() => query.name = word.to_string(),
            _ => {}
        }
    }
    query
}

pub(crate) fn command_suggestions(input: &str, crds: &[k8s::CrdInfo], apis: &[k8s::ApiInfo], dashboard_categories: &[&'static str], extensions: &[crate::extensions::Loaded], custom: &[String]) -> Vec<Suggestion> {
    let query = parse_query(input);
    let input = query.name.clone();
    if input.is_empty() && !query.has_filters() {
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
            // The form kubectl takes, which tells two kinds of one name apart. Only once a dot
            // is typed, or `agent` would match every kind of a group called `kagent.dev`.
            if input.contains('.') {
                names.push(format!("{}.{}", crd.plural.to_lowercase(), crd.group.to_lowercase()));
            }
            names
        }
        // Another group's resource of the same name is told apart by `plural.group`, once a dot is typed.
        Cmd::Api(i, ..) => {
            let mut names = cmd.names();
            if input.contains('.')
                && let Some(api) = apis.get(i).filter(|a| !a.group.is_empty())
            {
                names.push(format!("{}.{}", api.plural.to_lowercase(), api.group.to_lowercase()));
            }
            names
        }
        // A dashboard answers to its extensions' names too, so `flux` finds GitOps.
        Cmd::Kind(ResourceKind::ExtensionDashboard(category)) => {
            let mut names = cmd.names();
            for l in extensions.iter().filter(|l| l.error.is_none() && l.kinds.iter().any(|k| k.category == category)) {
                for name in [l.id.to_lowercase(), l.name.to_lowercase().replace(' ', "")] {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
            names
        }
        _ => cmd.names(),
    };
    let crd_keys: std::collections::HashSet<(&str, &str)> = crds.iter().map(|c| (c.group, c.plural.as_str())).collect();
    let mut scored: Vec<(i64, Suggestion)> = std::iter::once(Cmd::Context)
        .chain(std::iter::once(Cmd::Events))
        .chain(std::iter::once(Cmd::Problems))
        .chain(std::iter::once(Cmd::Theme))
        .chain(std::iter::once(Cmd::Settings))
        .chain(std::iter::once(Cmd::Quit))
        .chain((0..custom.len()).map(Cmd::Custom))
        .chain(menu_sections(crds, dashboard_categories).iter().flat_map(|s| s.tiles.iter().copied()).map(Cmd::Kind))
        // A group's name is never matched on its own; its kinds still sit under it. The context
        // switcher, settings, themes and Home have keys of their own (C , T H), so they aren't
        // listed; typing one's exact name still runs it (`hidden_command`).
        .filter(|cmd| !matches!(cmd, Cmd::Kind(ResourceKind::CustomResourceGroup(_))) && hidden_command_of(*cmd).is_none())
        .chain((0..crds.len()).map(Cmd::Crd))
        // Every other resource the server lists, unless a built-in kind has that name or it is a
        // custom resource (looked up in a set, not by scanning every CRD per resource).
        .chain(apis.iter().enumerate().filter(|(_, a)| ResourceKind::from_command(a.plural).is_none() && !crd_keys.contains(&(a.group, a.plural))).map(|(i, a)| Cmd::Api(i, a.plural, a.kind)))
        .filter_map(|cmd| {
            let names = names_of(cmd);
            let native = match cmd {
                Cmd::Api(i, ..) => apis.get(i).map(|a| (a.group, a.kind)),
                // Only kinds that are Kubernetes resources: knav's own views (Helm releases,
                // port-forwards, the Overview, dashboards) have no API group.
                Cmd::Kind(k) if !k.aliases().is_empty() && !matches!(k, ResourceKind::HelmReleases | ResourceKind::PortForwards | ResourceKind::ApiResources | ResourceKind::Overview | ResourceKind::CustomResourceList) => native_of(&names, apis),
                _ => None,
            };
            // The filters keep only kinds that satisfy every one of them.
            let group = match cmd {
                Cmd::Crd(i) => Some(crds[i].group),
                _ => native.map(|(group, _)| group),
            };
            let mut group_score = 0;
            for filter in &query.groups {
                let group = group?;
                group_score += name_score(filter, if group.is_empty() { "core" } else { group })?;
            }
            if !query.exts.is_empty() {
                let keys = owner_keys(cmd, crds, extensions);
                if !query.exts.iter().all(|filter| keys.iter().any(|key| name_score(filter, key).is_some())) {
                    return None;
                }
            }
            for wanted in &query.is {
                // A start of the word, not any letters in order: `is:b` is built-in, `is:c` custom.
                let built_in = ("builtin".starts_with(wanted.as_str()) || "built-in".starts_with(wanted.as_str())) && native.is_some();
                let custom = "custom".starts_with(wanted.as_str()) && matches!(cmd, Cmd::Crd(_));
                if !(built_in || custom) {
                    return None;
                }
            }
            let kind_score = if input.is_empty() {
                0
            } else {
                names
                    .iter()
                    .filter_map(|alias| {
                        let mut score = name_score(&input, alias)?;
                        if *alias == input {
                            score += 1000;
                        } else if alias.starts_with(&input) {
                            score += 50;
                        }
                        Some(score)
                    })
                    .max()?
            };
            let primary = names[0].clone();
            // A row shows its plain name, whichever of its other names matched.
            let label = primary.clone();
            let ext = Some(owners(cmd, crds, extensions).join(" · ")).filter(|o| !o.is_empty());
            Some((kind_score + group_score, Suggestion { cmd, label, primary, ext, depth: 0, kind_name: None, best: false, native, built_in: false, last: false }))
        })
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    // Everything that matched is grouped first and the menu is cut after, so a group's `+N more`
    // counts what is really left and a block is never cut in half by an arbitrary top ten.
    let mut ranked: Vec<Suggestion> = scored.into_iter().map(|(_, suggestion)| suggestion).take(MAX_MATCHES).collect();
    if let Some(first) = ranked.first_mut() {
        first.best = true;
    }
    // Tab completes a resource whose name another entry shares (or that was found through a
    // group filter) to its full `plural.group`, so Enter can't land on a different row.
    let labels: Vec<String> = ranked.iter().map(|r| r.label.clone()).collect();
    for (i, row) in ranked.iter_mut().enumerate() {
        let shared = labels.iter().enumerate().any(|(j, other)| j != i && *other == labels[i]);
        let full = match row.cmd {
            Cmd::Crd(index) if shared || query.has_filters() => Some((crds[index].plural.to_lowercase(), crds[index].group.to_lowercase())),
            Cmd::Api(index, ..) if shared || input.contains('.') => apis.get(index).filter(|a| !a.group.is_empty()).map(|a| (a.plural.to_lowercase(), a.group.to_lowercase())),
            _ => None,
        };
        if let Some((plural, group)) = full {
            row.primary = format!("{plural}.{group}");
        }
    }
    grouped(ranked, crds, extensions, &names_of)
}

/// The API group and kind a built-in kind is, found in the cluster's own resource list by
/// its names. A kind that also matches by name wins over a metrics or other lookalike.
fn native_of(names: &[String], apis: &[k8s::ApiInfo]) -> Option<(&'static str, &'static str)> {
    let by_plural = || apis.iter().filter(|a| names.iter().any(|n| n == a.plural));
    by_plural().find(|a| names.iter().any(|n| n.eq_ignore_ascii_case(a.kind))).or_else(|| by_plural().next()).map(|a| (a.group, a.kind))
}

/// How many matches are grouped before the menu is cut.
const MAX_MATCHES: usize = 400;
/// The most rows of kinds that belong to no group, and the most group blocks, one menu shows.
const MAX_FLAT_ROWS: usize = 10;
const MAX_BLOCKS: usize = 8;

/// How many kinds are listed under one API group before `+N more`.
const KINDS_UNDER_A_GROUP: usize = 6;

/// Which API group a matched row sits under: a custom resource's, or a built-in's.
#[derive(Clone, Copy, PartialEq)]
enum Bucket {
    Custom(&'static str),
    Native(&'static str),
}

/// Gathers the resource matches into a tree: each API group once, as a header at the place of
/// its best match, then its kinds. Custom groups list what matched, or all their kinds when
/// the group's own name matched; built-in groups list what matched, under a `built-in` header.
fn grouped(ranked: Vec<Suggestion>, crds: &[k8s::CrdInfo], extensions: &[crate::extensions::Loaded], names_of: &dyn Fn(Cmd) -> Vec<String>) -> Vec<Suggestion> {
    let bucket_of = |s: &Suggestion| match s.cmd {
        Cmd::Kind(ResourceKind::CustomResourceGroup(group)) => Some(Bucket::Custom(group)),
        Cmd::Crd(i) => crds.get(i).map(|c| Bucket::Custom(c.group)),
        _ => s.native.map(|(group, _)| Bucket::Native(group)),
    };
    let mut out: Vec<Suggestion> = Vec::new();
    let mut seen: Vec<Bucket> = Vec::new();
    let (mut flat, mut blocks) = (0, 0);
    for suggestion in &ranked {
        let Some(bucket) = bucket_of(suggestion) else {
            if flat < MAX_FLAT_ROWS {
                flat += 1;
                out.push(suggestion.clone());
            }
            continue;
        };
        if seen.contains(&bucket) {
            continue;
        }
        seen.push(bucket);
        if blocks >= MAX_BLOCKS {
            continue;
        }
        blocks += 1;
        let (header, mut rows) = match bucket {
            Bucket::Custom(group) => {
                let all: Vec<usize> = (0..crds.len()).filter(|&i| crds[i].group == group).collect();
                let owned = extensions.iter().filter(|l| l.error.is_none() && l.kinds.iter().any(|k| k.group == group)).map(|l| l.name.clone()).collect::<Vec<_>>().join(" · ");
                let header = Suggestion {
                    cmd: Cmd::Kind(ResourceKind::CustomResourceGroup(group)),
                    label: group.to_string(),
                    primary: group.to_string(),
                    ext: Some(owned).filter(|o| !o.is_empty()),
                    depth: 0,
                    kind_name: None,
                    best: suggestion.best && matches!(suggestion.cmd, Cmd::Kind(_)),
                    native: None,
                    built_in: false,
                    last: false,
                };
                // The kinds that matched are all that is listed, so `:agents` doesn't bring
                // along every other kind of the group. A group matched by its own name lists
                // its kinds.
                let mut rows: Vec<Suggestion> = ranked.iter().filter(|r| matches!(r.cmd, Cmd::Crd(i) if all.contains(&i))).cloned().collect();
                if rows.is_empty() {
                    for &i in &all {
                        let primary = names_of(Cmd::Crd(i)).remove(0);
                        let ext = Some(owners(Cmd::Crd(i), crds, extensions).join(" · ")).filter(|o| !o.is_empty());
                        rows.push(Suggestion { cmd: Cmd::Crd(i), label: primary.clone(), primary, ext, depth: 1, kind_name: None, best: false, native: None, built_in: false, last: false });
                    }
                }
                for row in &mut rows {
                    row.kind_name = match row.cmd {
                        Cmd::Crd(i) => Some(crds[i].kind),
                        _ => None,
                    };
                }
                (header, rows)
            }
            Bucket::Native(group) => {
                // A built-in group has no view of its own, so its header opens the list of every
                // resource type, which names the groups.
                let header = Suggestion {
                    cmd: Cmd::Kind(ResourceKind::ApiResources),
                    label: if group.is_empty() { "core".to_string() } else { group.to_string() },
                    primary: "apiresources".to_string(),
                    ext: None,
                    depth: 0,
                    kind_name: None,
                    best: false,
                    native: None,
                    built_in: true,
                    last: false,
                };
                let mut rows: Vec<Suggestion> = ranked.iter().filter(|r| bucket_of(r) == Some(bucket)).cloned().collect();
                for row in &mut rows {
                    row.kind_name = row.native.map(|(_, kind)| kind);
                }
                (header, rows)
            }
        };
        let more_cmd = header.cmd;
        let more_primary = header.primary.clone();
        out.push(header);
        let hidden = rows.len().saturating_sub(KINDS_UNDER_A_GROUP);
        rows.truncate(KINDS_UNDER_A_GROUP);
        for mut row in rows {
            row.depth = 1;
            out.push(row);
        }
        let more = hidden > 0;
        if more {
            out.push(Suggestion { cmd: more_cmd, label: format!("+{hidden} more"), primary: more_primary, ext: None, depth: 1, kind_name: None, best: false, native: None, built_in: false, last: false });
        }
        // The block's last row closes its connector.
        if let Some(last) = out.last_mut()
            && last.depth == 1
        {
            last.last = true;
        }
    }
    out
}

/// One autocomplete line: what it does, and its label (the kind's plain name).
#[derive(Clone)]
pub(crate) struct Suggestion {
    pub(crate) cmd: Cmd,
    pub(crate) label: String,
    /// The name Tab completes to.
    pub(crate) primary: String,
    /// The extensions that provide this kind or dashboard, shown beside it.
    pub(crate) ext: Option<String>,
    /// 0 for a row of its own, 1 for a kind listed under its API group.
    pub(crate) depth: u8,
    /// A custom resource's kind as the API names it (`Agent` for `agents`).
    pub(crate) kind_name: Option<&'static str>,
    /// The match ranked first, which starts out highlighted even when a header sits above it.
    pub(crate) best: bool,
    /// A built-in or discovered resource's API group and kind, from the cluster's own list.
    pub(crate) native: Option<(&'static str, &'static str)>,
    /// A header for the built-in kinds of one API group.
    pub(crate) built_in: bool,
    /// The last row of an API group's block, drawn with a closing connector.
    pub(crate) last: bool,
}

/// The row to highlight: the best match, which a group header may sit above.
pub(crate) fn best_row(suggestions: &[Suggestion]) -> usize {
    suggestions.iter().position(|s| s.best).unwrap_or(0)
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

    /// What shows at the row's right end: a header says what sort of group it is, a kind says
    /// what the API calls it, and extensions say they provide the row.
    pub(crate) fn tags(&self) -> Vec<(String, ui::TagRole)> {
        let mut tags = Vec::new();
        // A row of knav's own says what sort it is: none of these is a Kubernetes API resource.
        let own = match self.cmd {
            Cmd::Context | Cmd::Theme | Cmd::Settings | Cmd::Quit => Some("Command"),
            Cmd::Custom(_) => Some("Your command"),
            Cmd::Events | Cmd::Problems => Some("View"),
            Cmd::Kind(ResourceKind::Overview | ResourceKind::PortForwards | ResourceKind::ApiResources | ResourceKind::CustomResourceList | ResourceKind::HelmReleases) => Some("View"),
            Cmd::Kind(ResourceKind::ExtensionDashboard(_)) => Some("Dashboard"),
            _ => None,
        };
        if let Some(own) = own.filter(|_| self.depth == 0 && !self.is_header()) {
            tags.push((own.to_string(), ui::TagRole::Knav));
        }
        match self.cmd {
            Cmd::Kind(ResourceKind::CustomResourceGroup(_)) if self.depth == 0 => tags.push(("API group".to_string(), ui::TagRole::GroupRow)),
            _ if self.built_in && self.depth == 0 => tags.push(("Built-in".to_string(), ui::TagRole::Native)),
            _ => {}
        }
        if let Some(kind) = self.kind_name {
            tags.push((kind.to_string(), ui::TagRole::Kind));
        }
        if let Some(ext) = &self.ext {
            tags.push((ext.clone(), ui::TagRole::Extension));
        }
        tags
    }

    /// An API group's header, which Tab and Enter treat differently from a kind.
    pub(crate) fn is_header(&self) -> bool {
        self.depth == 0 && (self.built_in || matches!(self.cmd, Cmd::Kind(ResourceKind::CustomResourceGroup(_))))
    }

    /// The `+N more` row closing a group's block.
    pub(crate) fn is_more(&self) -> bool {
        self.depth == 1 && self.label.starts_with('+') && matches!(self.cmd, Cmd::Kind(_))
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
    let mut scored: Vec<(i64, &String)> = names.iter().filter_map(|n| fuzzy::contains_score(filter, n).map(|s| (s, n))).collect();
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
        command_suggestions(input, &[], &[], &[], &[], &[]).into_iter().next().unwrap_or_else(|| panic!("no suggestion for {input:?}"))
    }

    #[test]
    fn an_exact_alias_ranks_first_and_shows_the_plain_name() {
        let s = top("ns");
        assert!(matches!(s.cmd, Cmd::Kind(ResourceKind::Namespaces)));
        assert_eq!(s.label, "namespaces");
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
    fn quit_and_events_are_commands_too() {
        for input in ["q", "quit", "exit"] {
            assert!(matches!(top(input).cmd, Cmd::Quit), "{input}");
        }
        assert!(matches!(top("ev").cmd, Cmd::Events));
    }

    #[test]
    fn what_has_a_key_of_its_own_is_not_listed_but_still_runs_when_typed() {
        for input in ["ctx", "context", "theme", "config", "settings", "overview", "home"] {
            let found = command_suggestions(input, &[], &[], &[], &[], &[]);
            assert!(!found.iter().any(|s| matches!(s.cmd, Cmd::Context | Cmd::Theme | Cmd::Settings | Cmd::Kind(ResourceKind::Overview))), "{input} is listed: {:?}", found.iter().map(|s| &s.label).collect::<Vec<_>>());
        }
        assert!(matches!(hidden_command("ctx"), Some(Cmd::Context)));
        assert!(matches!(hidden_command("CONTEXT "), Some(Cmd::Context)));
        assert!(matches!(hidden_command("theme"), Some(Cmd::Theme)));
        assert!(matches!(hidden_command("config"), Some(Cmd::Settings)));
        assert!(matches!(hidden_command("home"), Some(Cmd::Kind(ResourceKind::Overview))));
        assert!(hidden_command("pods").is_none() && hidden_command("quit").is_none());
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
        assert!(command_suggestions("  ", &[], &[], &[], &[], &[]).is_empty());
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
    pub(crate) fn search_narrows_to_matches_of_the_whole_text() {
        assert!(row_matches("traefik", "kube-system traefik-9bcdbbd9-x2767"));
        assert!(!row_matches("tfk", "kube-system traefik-9bcdbbd9-x2767"), "scattered letters are not a match");
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

    fn snap(kind: ResourceKind, scope: Option<Scope>) -> crate::app::state::ListSnapshot {
        crate::app::state::ListSnapshot { kind, scope, search: String::new(), sort: None, hscroll: 0, selected: 0 }
    }

    #[test]
    fn location_names_each_drilled_thing_once_then_the_list() {
        let deployment = Scope::Owner { uid: "d".into(), kind: "Deployment".into(), name: "web".into(), namespace: Some("shop".into()) };
        let replicaset = Scope::Owner { uid: "r".into(), kind: "ReplicaSet".into(), name: "web-5d9d".into(), namespace: Some("shop".into()) };
        let trail = [Step::List(snap(ResourceKind::Deployments, None)), Step::List(snap(ResourceKind::ReplicaSets, Some(deployment.clone())))];
        let render = |segments: Vec<ui::PathSegment>| -> String {
            segments.into_iter().map(|s| match s.value { Some(v) => format!("{}[{v}]", s.kind), None => s.kind }).collect::<Vec<_>>().join(">>")
        };
        assert_eq!(render(location(ResourceKind::Pods, &trail, Some(&replicaset))), "Deployment[web]>>ReplicaSet[web-5d9d]>>Pods");
        assert_eq!(render(location(ResourceKind::ReplicaSets, &trail[..1], Some(&deployment))), "Deployment[web]>>ReplicaSets");
        assert_eq!(render(location(ResourceKind::Nodes, &[], None)), "Nodes");
        let ns = Scope::Namespace { name: "kube-system".into() };
        assert_eq!(render(location(ResourceKind::Pods, &[Step::List(snap(ResourceKind::Namespaces, None))], Some(&ns))), "Namespace[kube-system]>>Pods");
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
    fn a_custom_kind_is_found_under_its_group_header() {
        let crds = [crd("platform.example.com", "Environment", "environments"), crd("platform.example.com", "Team", "teams")];
        let found = command_suggestions("environ", &crds, &[], &[], &[], &[]);
        assert!(matches!(found[0].cmd, Cmd::Kind(ResourceKind::CustomResourceGroup("platform.example.com"))), "the group comes first");
        assert_eq!(found[0].depth, 0);
        assert_eq!(found[0].tags()[0].0, "API group");
        assert!(matches!(found[1].cmd, Cmd::Crd(0)), "the match is first under it");
        assert_eq!(found[1].depth, 1);
        assert_eq!(found[1].tags()[0].0, "Environment");
        assert_eq!(found.len(), 2, "the group's other kinds don't come along: {:?}", found.iter().map(|s| s.label.as_str()).collect::<Vec<_>>());
    }

    #[test]
    fn a_matched_group_lists_its_kinds() {
        let crds = [crd("platform.example.com", "Environment", "environments"), crd("platform.example.com", "Team", "teams")];
        let found = command_suggestions("platform.example", &crds, &[], &[], &[], &[]);
        let at = found.iter().position(|s| s.label == "platform.example.com").expect("the group row");
        let mut kinds: Vec<&str> = found[at + 1..].iter().take_while(|s| s.depth == 1).map(|s| s.label.as_str()).collect();
        kinds.sort_unstable();
        assert_eq!(kinds, ["environments", "teams"]);
    }

    #[test]
    fn similar_groups_each_get_their_own_header_and_kinds() {
        let crds = [crd("api.kagent.dev", "Agent", "agents"), crd("api.kagent.dev", "ModelConfig", "modelconfigs"), crd("kagent.dev", "Agent", "agents")];
        let found = command_suggestions("agent", &crds, &[], &[], &[], &[]);
        let layout: Vec<(u8, &str)> = found.iter().map(|s| (s.depth, s.label.as_str())).collect();
        let api = layout.iter().position(|r| *r == (0, "api.kagent.dev")).expect("api.kagent.dev header");
        let plain = layout.iter().position(|r| *r == (0, "kagent.dev")).expect("kagent.dev header");
        assert!(layout[api + 1..].iter().take_while(|r| r.0 == 1).any(|r| r.1 == "agents"), "{layout:?}");
        assert!(layout[plain + 1..].iter().take_while(|r| r.0 == 1).any(|r| r.1 == "agents"), "{layout:?}");
        assert!(!layout.iter().any(|r| r.1.starts_with("modelconfigs")), "only what matched is listed: {layout:?}");
        // Each header stands once, however many of its kinds matched.
        assert_eq!(layout.iter().filter(|r| r.0 == 0 && r.1 == "api.kagent.dev").count(), 1);
    }

    #[test]
    fn a_big_group_is_cut_with_a_more_row() {
        let names: Vec<String> = (0..9).map(|i| format!("thing{i}")).collect();
        let crds: Vec<k8s::CrdInfo> = names.iter().map(|n| crd("many.example.com", Box::leak(n.clone().into_boxed_str()), &format!("{n}s"))).collect();
        let found = command_suggestions("many.example", &crds, &[], &[], &[], &[]);
        assert_eq!(found.iter().filter(|s| s.depth == 1 && matches!(s.cmd, Cmd::Crd(_))).count(), 6);
        let more = found.last().expect("rows");
        assert_eq!(more.label, "+3 more");
        assert!(matches!(more.cmd, Cmd::Kind(ResourceKind::CustomResourceGroup("many.example.com"))));
    }

    #[test]
    fn an_extension_name_finds_its_dashboard_and_tags_its_kinds() {
        let registry = crate::extensions::Registry::load(std::path::Path::new("/nonexistent"));
        let categories = extensions::dashboards::categories(&registry);
        let found = command_suggestions("flux", &[], &[], &categories, &registry.loaded, &[]);
        let first = found.first().expect("flux finds something");
        assert!(matches!(first.cmd, Cmd::Kind(ResourceKind::ExtensionDashboard("GitOps"))));
        assert_eq!(first.ext.as_deref().map(|e| e.contains("Flux")), Some(true));

        let crds = [crd("helm.toolkit.fluxcd.io", "HelmRelease", "helmreleases")];
        let found = command_suggestions("helmrelease", &crds, &[], &categories, &registry.loaded, &[]);
        let release = found.iter().find(|s| matches!(s.cmd, Cmd::Crd(0))).expect("the HelmRelease kind");
        assert_eq!(release.tags().iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["HelmRelease", "Flux"]);
    }

    #[test]
    fn two_kinds_of_one_name_are_told_apart() {
        let registry = crate::extensions::Registry::load(std::path::Path::new("/nonexistent"));
        let crds = [crd("helm.toolkit.fluxcd.io", "HelmRelease", "helmreleases")];
        let found = command_suggestions("helmreleases", &crds, &[], &[], &registry.loaded, &[]);
        let native = found.iter().find(|s| matches!(s.cmd, Cmd::Kind(ResourceKind::HelmReleases))).expect("the built-in row");
        assert_eq!((native.depth, native.label.as_str()), (0, "helmreleases"));
        assert_eq!(native.tags().iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["View", "Helm · installed releases"]);
        // Flux's is a kind under its own group, and Tab completes it to the unambiguous name.
        let flux = found.iter().find(|s| matches!(s.cmd, Cmd::Crd(0))).expect("the Flux kind");
        assert_eq!((flux.depth, flux.label.as_str(), flux.primary.as_str()), (1, "helmreleases", "helmreleases.helm.toolkit.fluxcd.io"));
        // Typing the full name highlights that kind, not the group header above it.
        let typed = command_suggestions("helmreleases.helm.toolkit.fluxcd.io", &crds, &[], &[], &registry.loaded, &[]);
        assert!(matches!(typed[best_row(&typed)].cmd, Cmd::Crd(0)));
    }

    #[test]
    fn an_exact_kind_name_highlights_the_kind_not_its_group_header() {
        let crds = [crd("platform.example.com", "Environment", "environments"), crd("platform.example.com", "Team", "teams")];
        let found = command_suggestions("teams", &crds, &[], &[], &[], &[]);
        assert!(matches!(found[0].cmd, Cmd::Kind(ResourceKind::CustomResourceGroup(_))), "the header sits on top");
        assert!(matches!(found[best_row(&found)].cmd, Cmd::Crd(1)));
    }

    #[test]
    fn typing_agents_lists_only_agents_not_the_groups_other_kinds() {
        let crds = [
            crd("api.kagent.dev", "Agent", "agents"),
            crd("api.kagent.dev", "ModelConfig", "modelconfigs"),
            crd("api.kagent.dev", "ModelProvider", "modelproviders"),
            crd("kagent.dev", "Agent", "agents"),
        ];
        let found = command_suggestions("agents", &crds, &[], &[], &[], &[]);
        let labels: Vec<String> = found.iter().map(|s| format!("{}{}", "  ".repeat(usize::from(s.depth)), s.label)).collect();
        assert!(!labels.iter().any(|l| l.contains("model")), "{labels:?}");
        assert_eq!(found.iter().filter(|s| matches!(s.cmd, Cmd::Crd(_))).count(), 2, "{labels:?}");
    }

    #[test]
    fn a_group_name_is_never_matched_as_a_name() {
        let crds = [crd("kagent.dev", "Agent", "agents"), crd("kagent.dev", "Tool", "tools")];
        // `kagent` is a group, not a type, so a plain search finds nothing.
        assert!(command_suggestions("kagent", &crds, &[], &[], &[], &[]).is_empty());
        // The group filter lists the kinds in matching groups, under their header.
        let found = command_suggestions("group:kagent", &crds, &[], &[], &[], &[]);
        assert!(matches!(found[0].cmd, Cmd::Kind(ResourceKind::CustomResourceGroup("kagent.dev"))));
        assert_eq!(found.iter().filter(|s| matches!(s.cmd, Cmd::Crd(_))).count(), 2);
        // Combined with a name it narrows to that kind.
        let narrowed = command_suggestions("agents g:kagent", &crds, &[], &[], &[], &[]);
        assert_eq!(narrowed.iter().filter(|s| matches!(s.cmd, Cmd::Crd(_))).count(), 1);
        // A filter still being typed (nothing after the colon) filters nothing yet.
        assert_eq!(command_suggestions("agents group:", &crds, &[], &[], &[], &[]).iter().filter(|s| matches!(s.cmd, Cmd::Crd(_))).count(), 1);
    }

    #[test]
    fn filters_accumulate_and_each_must_hold() {
        let crds = [crd("kagent.dev", "Agent", "agents"), crd("other.example.com", "Agent", "agents")];
        let both = command_suggestions("agents group:kagent group:dev", &crds, &[], &[], &[], &[]);
        assert_eq!(both.iter().filter(|s| matches!(s.cmd, Cmd::Crd(0))).count(), 1);
        assert_eq!(both.iter().filter(|s| matches!(s.cmd, Cmd::Crd(1))).count(), 0);
        let apis = [api("", "Pod", "pods")];
        let custom_only = command_suggestions("agents is:custom", &crds, &apis, &[], &[], &[]);
        assert_eq!(custom_only.iter().filter(|s| matches!(s.cmd, Cmd::Crd(_))).count(), 2);
        let built_in_only = command_suggestions("agents is:builtin", &crds, &apis, &[], &[], &[]);
        assert!(built_in_only.iter().all(|s| !matches!(s.cmd, Cmd::Crd(_))));
    }

    #[test]
    fn ext_keeps_only_what_an_extension_provides() {
        let registry = crate::extensions::Registry::load(std::path::Path::new("/nonexistent"));
        let crds = [crd("helm.toolkit.fluxcd.io", "HelmRelease", "helmreleases"), crd("kagent.dev", "Agent", "agents")];
        let found = command_suggestions("ext:flux", &crds, &[], &[], &registry.loaded, &[]);
        assert!(found.iter().any(|s| matches!(s.cmd, Cmd::Crd(0))));
        assert!(!found.iter().any(|s| matches!(s.cmd, Cmd::Crd(1))));
    }

    #[test]
    fn a_matching_kind_still_shows_its_group_as_a_header() {
        let crds = [crd("apps.knav.dev", "WebApp", "webapps")];
        let found = command_suggestions("web", &crds, &[], &[], &[], &[]);
        let layout: Vec<(u8, &str)> = found.iter().map(|s| (s.depth, s.label.as_str())).collect();
        assert_eq!(layout, [(0, "apps.knav.dev"), (1, "webapps")]);
        assert!(found[1].last, "the only kind closes its connector");
    }

    fn api(group: &'static str, kind: &'static str, plural: &'static str) -> k8s::ApiInfo {
        k8s::ApiInfo { group, version: "v1".into(), kind, plural, namespaced: true, verbs: vec!["list".into()] }
    }

    #[test]
    fn built_in_kinds_sit_under_their_api_group_marked_built_in() {
        let apis = [
            api("networking.k8s.io", "Ingress", "ingresses"),
            api("networking.k8s.io", "IngressClass", "ingressclasses"),
            api("certificates.k8s.io", "CertificateSigningRequest", "certificatesigningrequests"),
            api("metrics.k8s.io", "PodMetrics", "pods"),
            api("", "Pod", "pods"),
        ];
        let found = command_suggestions("ingress", &[], &apis, &[], &[], &[]);
        let layout: Vec<(u8, &str)> = found.iter().map(|s| (s.depth, s.label.as_str())).collect();
        let header = layout.iter().position(|r| *r == (0, "networking.k8s.io")).expect("networking header");
        assert_eq!(found[header].tags()[0].0, "Built-in", "{layout:?}");
        let under: Vec<&str> = layout[header + 1..].iter().take_while(|r| r.0 == 1).map(|r| r.1).collect();
        assert!(under.iter().any(|l| l.starts_with("ingresses")) && under.iter().any(|l| l.starts_with("ingressclasses")), "{layout:?}");
        let ingress = found.iter().find(|s| s.label.starts_with("ingresses")).unwrap();
        assert_eq!(ingress.tags().iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["Ingress"]);
        // A header sits above the best match, which is still what starts out highlighted.
        assert!(found[best_row(&found)].label.starts_with("ingresses"));
    }

    #[test]
    fn a_built_in_kind_is_not_mistaken_for_its_metrics_lookalike() {
        let apis = [api("metrics.k8s.io", "PodMetrics", "pods"), api("", "Pod", "pods")];
        let found = command_suggestions("pods", &[], &apis, &[], &[], &[]);
        let header = found.iter().find(|s| s.depth == 0 && s.built_in).expect("a built-in header");
        assert_eq!(header.label, "core");
        let pods = found.iter().find(|s| matches!(s.cmd, Cmd::Kind(ResourceKind::Pods))).unwrap();
        assert_eq!(pods.native, Some(("", "Pod")));
    }

    #[test]
    fn knavs_own_helm_view_is_not_taken_for_flux_s_helmreleases() {
        let apis = [api("helm.toolkit.fluxcd.io", "HelmRelease", "helmreleases")];
        let found = command_suggestions("helmreleases", &[], &apis, &[], &[], &[]);
        let helm = found.iter().find(|s| matches!(s.cmd, Cmd::Kind(ResourceKind::HelmReleases))).expect("the Helm view");
        assert!(helm.native.is_none() && helm.depth == 0, "knav's view has no API group");
        // And `ext:helm` still finds it.
        let by_ext = command_suggestions("ext:helm", &[], &[], &[], &[], &[]);
        assert!(by_ext.iter().any(|s| matches!(s.cmd, Cmd::Kind(ResourceKind::HelmReleases))));
    }

    #[test]
    fn more_counts_everything_that_matched_not_just_the_first_ten() {
        let names: Vec<String> = (0..15).map(|i| format!("thing{i}")).collect();
        let crds: Vec<k8s::CrdInfo> = names.iter().map(|n| crd("many.example.com", Box::leak(n.clone().into_boxed_str()), &format!("{n}s"))).collect();
        let found = command_suggestions("thing", &crds, &[], &[], &[], &[]);
        assert_eq!(found.last().expect("rows").label, "+9 more");
    }

    #[test]
    fn the_same_plural_in_two_groups_completes_to_the_full_name() {
        let apis = [api("", "Event", "events"), api("events.k8s.io", "Event", "events")];
        let found = command_suggestions("events.events", &[], &apis, &[], &[], &[]);
        let full: Vec<&str> = found.iter().filter(|s| matches!(s.cmd, Cmd::Api(..))).map(|s| s.primary.as_str()).collect();
        assert!(full.contains(&"events.events.k8s.io"), "{full:?}");
        let plain = command_suggestions("events", &[], &apis, &[], &[], &[]);
        let apis_rows: Vec<&str> = plain.iter().filter(|s| matches!(s.cmd, Cmd::Api(..))).map(|s| s.primary.as_str()).collect();
        assert!(apis_rows.contains(&"events.events.k8s.io"), "the second one is told apart: {apis_rows:?}");
    }

    #[test]
    fn is_filters_take_the_start_of_a_word_and_an_unknown_one_matches_nothing() {
        let crds = [crd("kagent.dev", "Agent", "agents")];
        let apis = [api("", "Pod", "pods")];
        assert!(command_suggestions("is:c", &crds, &apis, &[], &[], &[]).iter().any(|s| matches!(s.cmd, Cmd::Crd(_))));
        assert!(!command_suggestions("is:b", &crds, &apis, &[], &[], &[]).iter().any(|s| matches!(s.cmd, Cmd::Crd(_))));
        assert!(command_suggestions("is:foo", &crds, &apis, &[], &[], &[]).is_empty());
    }

    #[test]
    fn rows_of_knavs_own_say_what_sort_they_are() {
        let registry = crate::extensions::Registry::load(std::path::Path::new("/nonexistent"));
        let categories = extensions::dashboards::categories(&registry);
        let tag = |input: &str| -> Vec<String> {
            let found = command_suggestions(input, &[], &[], &categories, &registry.loaded, &["describe".to_string()]);
            found.iter().find(|s| s.depth == 0).map(|s| s.tags().into_iter().map(|(t, _)| t).collect()).unwrap_or_default()
        };
        assert_eq!(tag("portforwards"), ["View"]);
        assert_eq!(tag("quit"), ["Command"]);
        assert_eq!(tag("problems"), ["View"]);
        assert_eq!(tag("describe"), ["Your command"]);
        let gitops = tag("gitops");
        assert_eq!(gitops.first().map(String::as_str), Some("Dashboard"), "{gitops:?}");
    }

    #[test]
    fn the_text_must_appear_in_one_piece_like_in_the_slash_search() {
        let apis = [api("networking.k8s.io", "Ingress", "ingresses"), api("networking.k8s.io", "IngressClass", "ingressclasses"), api("certificates.k8s.io", "CertificateSigningRequest", "certificatesigningrequests")];
        let labels = |input: &str| -> Vec<String> { command_suggestions(input, &[], &apis, &[], &[], &[]).into_iter().filter(|s| s.depth == 1).map(|s| s.label).collect() };
        // The letters of `ingress` are scattered through `certificatesigningrequests`, but not in one piece.
        let found = labels("ingress");
        assert!(found.iter().any(|l| l == "ingresses") && found.iter().any(|l| l == "ingressclasses"), "{found:?}");
        assert!(!found.iter().any(|l| l.contains("certificate")), "{found:?}");
        // Any part of a name matches, not only its start.
        assert!(labels("class").iter().any(|l| l == "ingressclasses"));
        // Letters in order but apart match nothing.
        assert!(labels("igs").is_empty(), "{:?}", labels("igs"));
        // `=name` is exactly that name, like in `/`.
        assert_eq!(labels("=ingresses"), ["ingresses"]);
        // An alias counts as a name (`ingress`), a part of one doesn't.
        assert_eq!(labels("=ingress"), ["ingresses"]);
        assert!(labels("=ingres").is_empty());
        // And the nearer the start, the higher it ranks.
        let ranked = labels("ing");
        assert_eq!(ranked.first().map(String::as_str), Some("ingresses"), "{ranked:?}");
    }

    #[test]
    fn filters_match_in_one_piece_too() {
        let crds = [crd("kagent.dev", "Agent", "agents"), crd("other.example.com", "Thing", "things")];
        assert_eq!(command_suggestions("group:kagent", &crds, &[], &[], &[], &[]).iter().filter(|s| matches!(s.cmd, Cmd::Crd(_))).count(), 1);
        // `kgt` is in `kagent.dev` in order but not in one piece.
        assert!(command_suggestions("group:kgt", &crds, &[], &[], &[], &[]).is_empty());
    }
}
