//! Draws one frame for the active `Mode`, with the list or Overview behind it.

use super::*;

/// Everything a frame reads but never changes, gathered once per loop iteration.
pub(super) struct View<'a> {
    pub rows: &'a dyn Fn() -> ui::Rows<'a>,
    pub overview: &'a k8s::Overview,
    pub nodes: &'a [std::sync::Arc<Node>],
    pub usage: Option<&'a metrics::ClusterUsage>,
    pub pod_usage: Option<&'a metrics::PodUsageMap>,
    pub problems: &'a [std::sync::Arc<k8s::problems::Problem>],
    pub node_detail_rows: &'a [std::sync::Arc<k8s::PodRow>],
    pub node_rows: &'a [k8s::NodeRow],
    pub crds: &'a [k8s::CrdInfo],
    pub apis: &'a [k8s::ApiInfo],
    pub extensions: &'a [extensions::Loaded],
    /// Whether Helm found any releases, for the Extensions screen's PRESENT column.
    /// Helm declares no CRDs to check, unlike the other extensions.
    pub helm_present: bool,
    /// Every category and kind the catalog has now, enabled extensions included, for
    /// the Layout tab.
    pub layout_names: &'a [(&'static str, Vec<&'static str>)],
    /// Every category with a dashboard, for the `:` suggestions.
    pub dashboard_categories: &'a [&'static str],
    pub favorites: &'a Favorites,
    pub hints: &'a [(&'a str, &'a str)],
    pub show_hints_panel: bool,
    pub chrome: &'a ui::Chrome,
    pub path: &'a [ui::PathSegment],
    pub header_now: &'a ui::HeaderInfo,
    pub search: &'a str,
    pub sort_view: ui::SortState,
    pub marked: &'a HashSet<String>,
    /// The theme saved in the config, marked in the theme picker.
    pub config_preset: &'a str,
    pub config: &'a Config,
    /// For the Permissions menu: the context, what RBAC allows, and whether changes
    /// are blocked now.
    pub context: &'a str,
    pub role: &'a str,
    pub read_only: bool,
}

/// Draws one frame and returns the screen area it used, for mapping mouse positions.
pub(super) fn draw_mode(
    terminal: &mut ratatui::DefaultTerminal,
    mode: &mut Mode,
    view: &View,
    table_state: &mut TableState,
    hovered: Option<ui::Hover>,
    icons: &mut icons::IconCache,
    hscroll: &mut usize,
) -> Result<Rect> {
    let View { rows, overview, nodes, usage, pod_usage, problems, node_detail_rows, node_rows, crds, apis, extensions, helm_present, layout_names, dashboard_categories, favorites, hints, show_hints_panel, chrome, path, header_now, search, sort_view, marked, config_preset, config, context, role, read_only } = view;
    let (show_hints_panel, sort_view) = (*show_hints_panel, *sort_view);
    let rows_view = rows;
    let mut frame_area = Rect::default();
    // Every mode draws the same base screen, with its own overlay on top.
    let mut paint = |frame: &mut ratatui::Frame, hover: Option<ui::Hover>, background: Option<ui::Overlay>, overlay: Option<ui::Overlay>, editing: bool| {
        let screen = ui::Screen { rows: rows_view(), table_state: &mut *table_state, hints, show_hints_panel, path: Some(path), header: header_now, search: ui::Search { text: search, editing }, sort: sort_view, hscroll: &mut *hscroll, marked, chrome };
        ui::draw(frame, screen, ui::Layers { hover, background, overlay }, icons)
    };
        match mode {
            Mode::List => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    paint(frame, hovered, None, None, false);
                })?;
            }
            Mode::Command { input, selected, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let found = command_suggestions(input, crds, apis, dashboard_categories, &config.commands.iter().map(|c| c.name.clone()).collect::<Vec<_>>());
                    let suggestions: Vec<ui::SuggestionView> = found.into_iter().map(|s| ui::SuggestionView { icon: s.icon(crds), group: s.group, label: s.label }).collect();
                    let selected = (*selected).min(suggestions.len().saturating_sub(1));
                    let overlay = ui::Overlay::Command { input, suggestions: &suggestions, selected };
                    paint(frame, hovered, None, Some(overlay), false);
                })?;
            }
            Mode::Context { contexts, filter, state, error, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let items = ui::context_matches(contexts, filter);
                    let overlay = ui::Overlay::Context(ui::ContextView { items: &items, total: contexts.len(), filter, state, error: error.as_deref(), leave: "back" });
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::NamespacePick { names, filter, editing, state, sort: popup_sort, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let items: Vec<(String, Option<usize>)> =
                        filtered_names(names, filter, *popup_sort, favorites).into_iter().map(|n| (n.clone(), favorites.key_of(n))).collect();
                    let overlay = ui::Overlay::NamespacePicker { items: &items, total: names.len(), filter, editing: *editing, state, sort: popup_sort.view() };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Slots { namespace, selected, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Slots { namespace, slots: &favorites.slots, selected: *selected };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Notice { text, tone, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Notice { text, tone: *tone };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Working { job, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let (done, total) = job.progress.get();
                    let overlay = if job.visible() {
                        Some(ui::Overlay::Working { title: &job.title, elapsed: job.started.elapsed(), done, total, cancellable: true })
                    } else {
                        job.backdrop.then_some(ui::Overlay::Backdrop)
                    };
                    paint(frame, None, None, overlay, false);
                })?;
            }
            Mode::Confirm { spec, yes, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Confirm { spec, yes: *yes };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Settings { tab, settings, state, editing, capture, error, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let current_theme = crate::theme::theme();
                    let rows: Vec<ui::SettingView> = settings
                        .iter()
                        .enumerate()
                        .map(|(i, setting)| {
                            let is_editing = editing.is_some() && state.selected() == Some(i);
                            let shown = if is_editing { editing.clone().unwrap_or_default() } else { crate::app::settings::current(config, &current_theme, setting) };
                            let swatch = matches!(setting.kind, crate::app::settings::Kind::Color).then(|| crate::theme::parse_color(&shown).or_else(|| current_theme.get(setting.path.trim_start_matches("theme.colors."))).unwrap_or_default());
                            ui::SettingView {
                                section: setting.section,
                                label: setting.label.clone(),
                                value: shown,
                                swatch,
                                customised: crate::app::settings::is_customised(config, setting),
                                restart: setting.restart,
                                editing: is_editing,
                                help: crate::app::settings::describe(setting),
                            }
                        })
                        .collect();
                    let capture_view = capture.as_ref().and_then(|c| {
                        let setting = state.selected().and_then(|i| settings.get(i))?;
                        let keys = crate::app::settings::current(config, &current_theme, setting).split(", ").map(String::from).collect();
                        let stage = match (c.step, &c.pressed) {
                            (crate::app::mode::CaptureStep::Menu, _) => ui::CaptureStage::Menu,
                            (crate::app::mode::CaptureStep::Pick { .. }, None) => ui::CaptureStage::Waiting,
                            (crate::app::mode::CaptureStep::Pick { replace }, Some(key)) => ui::CaptureStage::Confirm { key: key.clone(), replace },
                        };
                        Some(ui::CaptureView { label: setting.label.clone(), keys, stage, problem: c.problem.clone() })
                    });
                    let layout_rows: Vec<ui::LayoutRow> = if *tab == ui::SettingsTab::Overview {
                        crate::app::overview_layout::resolve(&config.overview, layout_names).into_iter().enumerate().map(|(i, s)| ui::LayoutRow { name: s.name, number: i + 1, hidden: s.hidden }).collect()
                    } else {
                        Vec::new()
                    };
                    let overlay = ui::Overlay::Settings { tab: *tab, rows: &rows, layout: &layout_rows, state, error: error.as_deref(), capture: capture_view };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Extensions { filter, filter_editing, state, error, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let extension_rows: Vec<ui::ExtensionRow> = crate::extensions::visible_order(extensions, filter)
                        .into_iter()
                        .map(|i| {
                            let l = &extensions[i];
                            let enabled = config.extensions.enabled.iter().any(|e| e == &l.id);
                            let crd_present = l.kinds.iter().any(|k| crds.iter().any(|c| c.group == k.group && c.kind == k.kind));
                            let present = (enabled && l.error.is_none()).then(|| crd_present || (l.id == "helm" && *helm_present));
                            ui::ExtensionRow { name: l.name.clone(), description: l.description.clone(), enabled, bundled: l.bundled, present, error: l.error.clone() }
                        })
                        .collect();
                    let overlay = ui::Overlay::Extensions { rows: &extension_rows, state, error: error.as_deref(), filter, filter_editing: *filter_editing };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::ThemePicker { entries, state, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::ThemePicker { entries, state, saved: config_preset };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Shell { title, session, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let inner = ui::shell_inner(frame.area());
                    session.resize(inner.height, inner.width);
                    let exited = session.exited();
                    session.with_screen(|screen| {
                        let overlay = ui::Overlay::Shell { title, screen, exited };
                        paint(frame, None, None, Some(overlay), false);
                    });
                })?;
            }
            Mode::Details { manifest, sections, scroll, hscroll: details_hscroll, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let title = crate::app::mode::object_title(manifest);
                    let overlay = ui::Overlay::Details { title: &title, sections, scroll: *scroll, hscroll: *details_hscroll };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Relations { target, graph, selected, zoom, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let title = crate::app::mode::object_title(target);
                    let overlay = ui::Overlay::Relations { title: &title, graph, selected: *selected, zoom: *zoom };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Yaml { title, text, scroll, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Yaml { title, text, scroll: *scroll };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::OpenUrl { text, url, yes, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let spec = crate::ops::actions::open_url_spec(text, url);
                    let overlay = ui::Overlay::Confirm { spec: &spec, yes: *yes };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Permissions { tab, cursor, input, error, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let view = ui::PermissionsView {
                        tab: *tab,
                        cursor: *cursor,
                        context,
                        role,
                        read_only: *read_only,
                        everywhere: config.read_only.enabled,
                        contexts: if *tab == 2 { &config.highlight.contexts } else { &config.read_only.contexts },
                        input: input.as_ref().map(|(at, text)| (*at, text.as_str())),
                        error: error.as_deref(),
                    };
                    paint(frame, None, None, Some(ui::Overlay::Permissions(view)), false);
                })?;
            }
            Mode::Problems { state, search, editing, .. } => {
                let shown = crate::app::handlers::problems_matching(problems, search);
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let view = ui::ProblemsView { problems: &shown, total: problems.len(), search, editing: *editing, state };
                    paint(frame, None, None, Some(ui::Overlay::Problems(view)), *editing);
                })?;
            }
            Mode::History { target, revisions, cursor, scroll, .. } => {
                let running = revisions.iter().find(|r| r.current).map(|r| r.template.as_str()).unwrap_or_default();
                let diff = revisions.get(*cursor).map(|r| crate::ops::edit::diff(running, &r.template)).unwrap_or_default();
                let title = target.label();
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let view = ui::HistoryView { title: &title, revisions, cursor: *cursor, diff: &diff, scroll: *scroll, read_only: *read_only };
                    paint(frame, None, None, Some(ui::Overlay::History(view)), false);
                })?;
            }
            Mode::EditReview { draft, diff, scroll, focus, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::EditReview(ui::EditReviewView { title: &draft.title, diff, scroll: *scroll, focus: *focus, error: draft.error.as_deref() });
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Scale { targets, input, yes, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Scale(scale_view(targets, input, *yes));
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Ports { target, form, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let title = target.label();
                    let overlay = ui::Overlay::PortForward { title: &title, form };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Search => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    paint(frame, None, None, None, true);
                })?;
            }
            Mode::Spec { title, items, state, viewing, back, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    // Outside the `if let` below, since the background overlay borrows it.
                    let back_node_name: Option<String> = match &**back {
                        Mode::NodeDetail { name, .. } => Some(name.clone()),
                        _ => None,
                    };
                    let back_found_node = back_node_name.as_deref().and_then(|n| nodes.iter().find(|node| node.metadata.name.as_deref() == Some(n)));
                    let back_capacity = back_found_node.map(|n| k8s::node_capacity(n));
                    let back_detail_info = back_found_node.map(|n| k8s::node_detail_info(n));
                    let back_node_usage = back_node_name.as_deref().and_then(|n| usage.as_ref().and_then(|u| u.for_node(n)));
                    let node_background = if let Mode::NodeDetail { state: nd_state, .. } = &mut **back {
                        Some(ui::Overlay::NodeDetail(ui::NodeDetailView {
                            name: back_node_name.as_deref().unwrap_or(""),
                            cpu_usage: back_node_usage.map(|u| u.cpu_millicores),
                            cpu_capacity: back_capacity.as_ref().map(|c| c.cpu_millicores).unwrap_or(0),
                            memory_usage: back_node_usage.map(|u| u.memory_bytes),
                            memory_capacity: back_capacity.as_ref().map(|c| c.memory_bytes).unwrap_or(0),
                            pod_capacity: back_capacity.as_ref().map(|c| c.pods).unwrap_or(0),
                            info: back_detail_info.as_ref(),
                            pods: node_detail_rows,
                            pod_usage: *pod_usage,
                            state: nd_state,
                            sort: ui::SortState::default(),
                            search: ui::Search::default(),
                        }))
                    } else {
                        None
                    };
                    // While a leaf's value is shown, the tree becomes the dimmed background.
                    let (background, overlay) = match viewing {
                        Some((label, value)) => {
                            (Some(ui::Overlay::Spec { title, items, state }), ui::Overlay::ValueDetail { label, value })
                        }
                        None => (node_background, ui::Overlay::Spec { title, items, state }),
                    };
                    paint(frame, None, background, Some(overlay), false);
                })?;
            }
            Mode::Containers { title, containers, state, sort: popup_sort, back, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let back_node_name: Option<String> = match &**back {
                        Mode::NodeDetail { name, .. } => Some(name.clone()),
                        _ => None,
                    };
                    let back_found_node = back_node_name.as_deref().and_then(|n| nodes.iter().find(|node| node.metadata.name.as_deref() == Some(n)));
                    let back_capacity = back_found_node.map(|n| k8s::node_capacity(n));
                    let back_detail_info = back_found_node.map(|n| k8s::node_detail_info(n));
                    let back_node_usage = back_node_name.as_deref().and_then(|n| usage.as_ref().and_then(|u| u.for_node(n)));
                    let background = if let Mode::NodeDetail { state: nd_state, .. } = &mut **back {
                        Some(ui::Overlay::NodeDetail(ui::NodeDetailView {
                            name: back_node_name.as_deref().unwrap_or(""),
                            cpu_usage: back_node_usage.map(|u| u.cpu_millicores),
                            cpu_capacity: back_capacity.as_ref().map(|c| c.cpu_millicores).unwrap_or(0),
                            memory_usage: back_node_usage.map(|u| u.memory_bytes),
                            memory_capacity: back_capacity.as_ref().map(|c| c.memory_bytes).unwrap_or(0),
                            pod_capacity: back_capacity.as_ref().map(|c| c.pods).unwrap_or(0),
                            info: back_detail_info.as_ref(),
                            pods: node_detail_rows,
                            pod_usage: *pod_usage,
                            state: nd_state,
                            sort: ui::SortState::default(),
                            search: ui::Search::default(),
                        }))
                    } else {
                        None
                    };
                    let shown = sorted_containers(containers, popup_sort.spec);
                    let overlay = ui::Overlay::Containers { title, containers: &shown, state, sort: popup_sort.view() };
                    paint(frame, None, background, Some(overlay), false);
                })?;
            }
            Mode::NodeDetail { name, state, sort: popup_sort, search: nd_search, editing: nd_editing, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let found_node = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str()));
                    let capacity = found_node.map(|n| k8s::node_capacity(n));
                    let detail_info = found_node.map(|n| k8s::node_detail_info(n));
                    let node_usage = usage.as_ref().and_then(|u| u.for_node(name));
                    let overlay = ui::Overlay::NodeDetail(ui::NodeDetailView {
                        name: name.as_str(),
                        cpu_usage: node_usage.map(|u| u.cpu_millicores),
                        cpu_capacity: capacity.as_ref().map(|c| c.cpu_millicores).unwrap_or(0),
                        memory_usage: node_usage.map(|u| u.memory_bytes),
                        memory_capacity: capacity.as_ref().map(|c| c.memory_bytes).unwrap_or(0),
                        pod_capacity: capacity.as_ref().map(|c| c.pods).unwrap_or(0),
                        info: detail_info.as_ref(),
                        pods: node_detail_rows,
                        pod_usage: *pod_usage,
                        state,
                        sort: popup_sort.view(),
                        search: ui::Search { text: nd_search, editing: *nd_editing },
                    });
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::Events { filter, search: event_search, editing, state, sort: popup_sort } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Events(ui::EventsView { events: &overview.events, filter: *filter, search: event_search, editing: *editing, state, sort: popup_sort.view() });
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::EventDetail { entry, back } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let background = match &mut **back {
                        Mode::Events { filter, search, editing, state, sort: popup_sort } => {
                            Some(ui::Overlay::Events(ui::EventsView { events: &overview.events, filter: *filter, search, editing: *editing, state, sort: popup_sort.view() }))
                        }
                        _ => None,
                    };
                    let overlay = ui::Overlay::EventDetail { entry };
                    paint(frame, None, background, Some(overlay), false);
                })?;
            }
            Mode::ResourcesDetail => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::ResourcesDetail { overview, nodes: node_rows };
                    paint(frame, None, None, Some(overlay), false);
                })?;
            }
            Mode::ColumnDetail { col, selected, row_scroll } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    if let Some((title, items)) = overview.catalog.get(*col) {
                        let overlay = ui::Overlay::ColumnDetail { title, items, health: &overview.health, selected: *selected, row_scroll: *row_scroll };
                        paint(frame, None, None, Some(overlay), false);
                    } else {
                        paint(frame, None, None, None, false);
                    }
                })?;
            }
            Mode::Logs { title, lines, scroll, follow, timestamp_format, order, filter, filter_editing, back, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let shown_containers;
                    let background = match &mut **back {
                        Mode::Containers { title, containers, state, sort: popup_sort, .. } => {
                            shown_containers = sorted_containers(containers, popup_sort.spec);
                            Some(ui::Overlay::Containers { title, containers: &shown_containers, state, sort: popup_sort.view() })
                        }
                        _ => None,
                    };
                    let overlay = ui::Overlay::Logs(ui::LogsView {
                        title,
                        lines,
                        scroll: *scroll,
                        follow: *follow,
                        timestamp_format: *timestamp_format,
                        order: *order,
                        filter,
                        filter_editing: *filter_editing,
                    });
                    paint(frame, None, background, Some(overlay), false);
                })?;
            }
        }
    Ok(frame_area)
}

/// What the scale dialog shows for `targets`.
pub(super) fn scale_view<'a>(targets: &[crate::ops::actions::Target], input: &'a str, yes: bool) -> ui::ScaleView<'a> {
    let subjects = targets.iter().map(|t| (t.kind.clone(), t.namespace.as_ref().map_or(t.name.clone(), |ns| format!("{ns}/{}", t.name)), t.ready_text())).collect();
    let current = targets.first().map(|t| t.replicas()).filter(|now| targets.iter().all(|t| t.replicas() == *now));
    ui::ScaleView { subjects, value: input, current, yes }
}
