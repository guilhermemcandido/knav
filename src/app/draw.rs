//! Drawing: one frame for whatever `Mode` is active, with the list (or the
//! overview) behind it.

use super::*;

/// Everything a frame reads but never changes, gathered once per loop
/// iteration so the drawing code takes one argument instead of twenty.
pub(super) struct View<'a> {
    pub rows: &'a dyn Fn() -> ui::Rows<'a>,
    pub overview: &'a k8s::Overview,
    pub nodes: &'a [std::sync::Arc<Node>],
    pub usage: Option<&'a metrics::ClusterUsage>,
    pub node_detail_rows: &'a [k8s::PodRow],
    pub node_rows: &'a [k8s::NodeRow],
    pub crds: &'a [k8s::CrdInfo],
    pub apis: &'a [k8s::ApiInfo],
    pub favorites: &'a Favorites,
    pub hints: &'a [(&'a str, &'a str)],
    pub show_hints_panel: bool,
    pub path: &'a [ui::PathSegment],
    pub header_now: &'a ui::HeaderInfo,
    pub search: &'a str,
    pub sort_view: ui::SortState,
    pub marked: &'a HashSet<String>,
    /// The theme saved in the config, marked in the theme picker.
    pub config_preset: &'a str,
    pub config: &'a Config,
}

/// Draws one frame and returns the screen area it used (input handlers
/// need it to map mouse positions onto rows).
pub(super) fn draw_mode(
    terminal: &mut ratatui::DefaultTerminal,
    mode: &mut Mode,
    view: &View,
    table_state: &mut TableState,
    hovered: Option<ui::Hover>,
    icons: &mut icons::IconCache,
    hscroll: &mut usize,
) -> Result<Rect> {
    let View { rows, overview, nodes, usage, node_detail_rows, node_rows, crds, apis, favorites, hints, show_hints_panel, path, header_now, search, sort_view, marked, config_preset, config } = view;
    let (show_hints_panel, sort_view) = (*show_hints_panel, *sort_view);
    let rows_view = rows;
    let mut frame_area = Rect::default();
        match mode {
            Mode::List => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    ui::draw(frame, rows_view(), table_state, hovered, None, None, &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Command { input, selected, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let suggestions: Vec<ui::SuggestionView> = command_suggestions(input, crds, apis).into_iter().map(|s| ui::SuggestionView { icon: s.icon(), label: s.label }).collect();
                    let selected = (*selected).min(suggestions.len().saturating_sub(1));
                    let overlay = ui::Overlay::Command { input, suggestions: &suggestions, selected };
                    ui::draw(frame, rows_view(), table_state, hovered, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Context { contexts, filter, editing, state, error, sort: popup_sort, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let items: Vec<(String, String, bool)> =
                        filtered_contexts(contexts, filter, *popup_sort).into_iter().map(|c| (c.name.clone(), c.cluster.clone(), c.is_current)).collect();
                    let overlay = ui::Overlay::Context { items: &items, total: contexts.len(), filter, editing: *editing, state, error: error.as_deref(), sort: popup_sort.view() };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::NamespacePick { names, filter, editing, state, sort: popup_sort, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let items: Vec<(String, Option<usize>)> =
                        filtered_names(names, filter, *popup_sort, &favorites).into_iter().map(|n| (n.clone(), favorites.key_of(n))).collect();
                    let overlay = ui::Overlay::NamespacePicker { items: &items, total: names.len(), filter, editing: *editing, state, sort: popup_sort.view() };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Slots { namespace, selected, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Slots { namespace, slots: &favorites.slots, selected: *selected };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Notice { text, error, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Notice { text, error: *error };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Confirm { text, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Confirm { text };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
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
                            let shown = if is_editing { editing.clone().unwrap_or_default() } else { crate::config::settings::current(config, &current_theme, setting) };
                            let swatch = matches!(setting.kind, crate::config::settings::Kind::Color).then(|| crate::theme::parse_color(&shown).or_else(|| current_theme.get(setting.path.trim_start_matches("theme.colors."))).unwrap_or_default());
                            ui::SettingView {
                                section: setting.section,
                                label: setting.label.clone(),
                                value: shown,
                                swatch,
                                customised: crate::config::settings::is_customised(config, setting),
                                restart: setting.restart,
                                editing: is_editing,
                                help: crate::config::settings::describe(setting),
                            }
                        })
                        .collect();
                    let capture_view = capture.as_ref().and_then(|c| {
                        let setting = state.selected().and_then(|i| settings.get(i))?;
                        let keys = crate::config::settings::current(config, &current_theme, setting).split(", ").map(String::from).collect();
                        let stage = match (c.step, &c.pressed) {
                            (crate::app::mode::CaptureStep::Menu, _) => ui::CaptureStage::Menu,
                            (crate::app::mode::CaptureStep::Pick { .. }, None) => ui::CaptureStage::Waiting,
                            (crate::app::mode::CaptureStep::Pick { replace }, Some(key)) => ui::CaptureStage::Confirm { key: key.clone(), replace },
                        };
                        Some(ui::CaptureView { label: setting.label.clone(), keys, stage, problem: c.problem.clone() })
                    });
                    let layout_rows: Vec<ui::LayoutRow> = if *tab == ui::SettingsTab::Overview {
                        crate::k8s::layout::resolve(&config.overview).into_iter().enumerate().map(|(i, s)| ui::LayoutRow { name: s.name, number: i + 1, hidden: s.hidden }).collect()
                    } else {
                        Vec::new()
                    };
                    let overlay = ui::Overlay::Settings { tab: *tab, rows: &rows, layout: &layout_rows, state, error: error.as_deref(), capture: capture_view };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::ThemePicker { entries, state, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::ThemePicker { entries, state, saved: &config_preset };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
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
                        ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                    });
                })?;
            }
            Mode::Details { manifest, sections, scroll, hscroll, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let title = crate::app::mode::object_title(manifest);
                    let overlay = ui::Overlay::Details { title: &title, sections, scroll: *scroll, hscroll: *hscroll };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Relations { target, graph, selected, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let title = crate::app::mode::object_title(target);
                    let overlay = ui::Overlay::Relations { title: &title, graph, selected: *selected };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Yaml { title, text, scroll, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Yaml { title, text, scroll: *scroll };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::OpenUrl { text, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Confirm { text };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Scale { targets, input, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let title = match targets.as_slice() {
                        [one] => format!("Scale {} to", one.label()),
                        many => format!("Scale {} objects to", many.len()),
                    };
                    let overlay = ui::Overlay::Prompt { title: &title, value: input, hint: "" };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Ports { target, form, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let title = target.label();
                    let overlay = ui::Overlay::PortForward { title: &title, form };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Search => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    ui::draw(frame, rows_view(), table_state, None, None, None, &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: true }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Menu { selected } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let sections = menu_sections(crds);
                    let overlay = ui::Overlay::Menu { sections: &sections, selected: *selected };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Spec { title, items, state, viewing, back, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    // Hoisted out of the `if let` below so these live for
                    // the rest of the closure, not just that block, the
                    // background `Overlay` borrows from them.
                    let back_node_name: Option<String> = match &**back {
                        Mode::NodeDetail { name, .. } => Some(name.clone()),
                        _ => None,
                    };
                    let back_found_node = back_node_name.as_deref().and_then(|n| nodes.iter().find(|node| node.metadata.name.as_deref() == Some(n)));
                    let back_capacity = back_found_node.map(|n| k8s::node_capacity(n));
                    let back_detail_info = back_found_node.map(|n| k8s::node_detail_info(n));
                    let back_node_usage = back_node_name.as_deref().and_then(|n| usage.as_ref().and_then(|u| u.for_node(n)));
                    let node_background = if let Mode::NodeDetail { state: nd_state, .. } = &mut **back {
                        Some(ui::Overlay::NodeDetail {
                            name: back_node_name.as_deref().unwrap_or(""),
                            cpu_usage: back_node_usage.map(|u| u.cpu_millicores),
                            cpu_capacity: back_capacity.as_ref().map(|c| c.cpu_millicores).unwrap_or(0),
                            memory_usage: back_node_usage.map(|u| u.memory_bytes),
                            memory_capacity: back_capacity.as_ref().map(|c| c.memory_bytes).unwrap_or(0),
                            pod_capacity: back_capacity.as_ref().map(|c| c.pods).unwrap_or(0),
                            info: back_detail_info.as_ref(),
                            pods: &node_detail_rows,
                            state: nd_state,
                            sort: ui::SortState::default(),
                            search: ui::Search::default(),
                        })
                    } else {
                        None
                    };
                    // While a leaf's full value is shown, the Spec tree becomes the dimmed
                    // background instead of the overlay.
                    let (background, overlay) = match viewing {
                        Some((label, value)) => {
                            (Some(ui::Overlay::Spec { title, items, state }), ui::Overlay::ValueDetail { label, value })
                        }
                        None => (node_background, ui::Overlay::Spec { title, items, state }),
                    };
                    ui::draw(frame, rows_view(), table_state, None, background, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
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
                        Some(ui::Overlay::NodeDetail {
                            name: back_node_name.as_deref().unwrap_or(""),
                            cpu_usage: back_node_usage.map(|u| u.cpu_millicores),
                            cpu_capacity: back_capacity.as_ref().map(|c| c.cpu_millicores).unwrap_or(0),
                            memory_usage: back_node_usage.map(|u| u.memory_bytes),
                            memory_capacity: back_capacity.as_ref().map(|c| c.memory_bytes).unwrap_or(0),
                            pod_capacity: back_capacity.as_ref().map(|c| c.pods).unwrap_or(0),
                            info: back_detail_info.as_ref(),
                            pods: &node_detail_rows,
                            state: nd_state,
                            sort: ui::SortState::default(),
                            search: ui::Search::default(),
                        })
                    } else {
                        None
                    };
                    let shown = sorted_containers(containers, *popup_sort);
                    let overlay = ui::Overlay::Containers { title, containers: &shown, state, sort: popup_sort.view() };
                    ui::draw(frame, rows_view(), table_state, None, background, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::NodeDetail { name, state, sort: popup_sort, search: nd_search, editing: nd_editing, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let found_node = nodes.iter().find(|n| n.metadata.name.as_deref() == Some(name.as_str()));
                    let capacity = found_node.map(|n| k8s::node_capacity(n));
                    let detail_info = found_node.map(|n| k8s::node_detail_info(n));
                    let node_usage = usage.as_ref().and_then(|u| u.for_node(name));
                    let overlay = ui::Overlay::NodeDetail {
                        name: name.as_str(),
                        cpu_usage: node_usage.map(|u| u.cpu_millicores),
                        cpu_capacity: capacity.as_ref().map(|c| c.cpu_millicores).unwrap_or(0),
                        memory_usage: node_usage.map(|u| u.memory_bytes),
                        memory_capacity: capacity.as_ref().map(|c| c.memory_bytes).unwrap_or(0),
                        pod_capacity: capacity.as_ref().map(|c| c.pods).unwrap_or(0),
                        info: detail_info.as_ref(),
                        pods: &node_detail_rows,
                        state,
                        sort: popup_sort.view(),
                        search: ui::Search { text: nd_search, editing: *nd_editing },
                    };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::Events { filter, search, editing, state, sort: popup_sort } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::Events { events: &overview.events, filter: *filter, search, editing: *editing, state, sort: popup_sort.view() };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::EventDetail { entry, back } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let background = match &mut **back {
                        Mode::Events { filter, search, editing, state, sort: popup_sort } => {
                            Some(ui::Overlay::Events { events: &overview.events, filter: *filter, search, editing: *editing, state, sort: popup_sort.view() })
                        }
                        _ => None,
                    };
                    let overlay = ui::Overlay::EventDetail { entry };
                    ui::draw(frame, rows_view(), table_state, None, background, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::ResourcesDetail => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let overlay = ui::Overlay::ResourcesDetail { overview: &overview, nodes: node_rows };
                    ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
            Mode::ColumnDetail { col, selected, row_scroll } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    if let Some((title, items)) = overview.catalog.get(*col) {
                        let overlay = ui::Overlay::ColumnDetail { title, items, health: &overview.health, selected: *selected, row_scroll: *row_scroll };
                        ui::draw(frame, rows_view(), table_state, None, None, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                    } else {
                        ui::draw(frame, rows_view(), table_state, None, None, None, &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                    }
                })?;
            }
            Mode::Logs { title, lines, scroll, follow, timestamp_format, order, filter, filter_editing, back, .. } => {
                terminal.draw(|frame| {
                    frame_area = frame.area();
                    let shown_containers;
                    let background = match &mut **back {
                        Mode::Containers { title, containers, state, sort: popup_sort, .. } => {
                            shown_containers = sorted_containers(containers, *popup_sort);
                            Some(ui::Overlay::Containers { title, containers: &shown_containers, state, sort: popup_sort.view() })
                        }
                        _ => None,
                    };
                    let overlay = ui::Overlay::Logs {
                        title,
                        lines,
                        scroll: *scroll,
                        follow: *follow,
                        timestamp_format: *timestamp_format,
                        order: *order,
                        filter,
                        filter_editing: *filter_editing,
                    };
                    ui::draw(frame, rows_view(), table_state, None, background, Some(overlay), &hints, show_hints_panel, Some(path), icons, &header_now, ui::Search { text: &search, editing: false }, sort_view, hscroll, marked);
                })?;
            }
        }
    Ok(frame_area)
}
