//! The extensions browser (`E`): third-party kinds (Flux, Argo CD, Helm, ...)
//! toggled on or off, whether each is actually present on this cluster, and
//! a live search — its own screen, not a Settings tab, reachable from
//! anywhere the same way `C` reaches the context switcher.

use super::*;

fn extensions_area(frame: Rect) -> Rect {
    centered_rect(94, 88, frame)
}

/// EXTENSION column width bounds: never narrower than the "EXTENSION" header
/// itself, never so wide (a long external name, `knav ext add`) that it
/// crowds the description out of a normal terminal.
const EXTENSION_NAME_MIN: usize = 10;
const EXTENSION_NAME_MAX: usize = 28;

/// One extension's row: status, name, whether it's present (separate from
/// on/off), and its static description — an error takes over the description
/// spot instead.
fn extension_row(row: &ExtensionRow) -> Row<'static> {
    let (status, status_style) = match (&row.error, row.enabled) {
        (Some(_), _) => ("error".to_string(), Style::default().fg(theme().bad)),
        (None, true) => ("on".to_string(), Style::default().fg(theme().ok).add_modifier(Modifier::BOLD)),
        (None, false) => ("off".to_string(), Style::default().fg(theme().muted)),
    };
    let name_style = if row.error.is_some() {
        Style::default().fg(theme().muted)
    } else if row.enabled {
        Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme().desc)
    };
    // Its own column, not folded into the description: on/off asks "did you turn
    // this on", present asks "is it actually here on THIS cluster" — two
    // different questions, so a glance at one doesn't answer the other.
    let (present, present_style) = match row.present {
        Some(true) => ("Present".to_string(), Style::default().fg(theme().ok)),
        Some(false) => ("Missing".to_string(), Style::default().fg(theme().warn)),
        None => ("—".to_string(), Style::default().fg(theme().muted)),
    };
    let detail = row.error.clone().unwrap_or_else(|| row.description.clone());
    Row::new(vec![
        Cell::from(Span::styled(status, status_style)),
        Cell::from(Span::styled(row.name.clone(), name_style)),
        Cell::from(Span::styled(present, present_style)),
        Cell::from(Span::styled(detail, Style::default().fg(theme().muted))),
    ])
}

fn section_heading(text: &str) -> Row<'static> {
    Row::new(vec![Cell::new(Span::styled(text.to_string(), Style::default().fg(theme().heading).add_modifier(Modifier::BOLD))).column_span(4)])
}

fn placeholder_row(text: String) -> Row<'static> {
    Row::new(vec![Cell::new(Span::styled(text, Style::default().fg(theme().muted).add_modifier(Modifier::ITALIC))).column_span(4)])
}

fn blank_row() -> Row<'static> {
    Row::new(vec![Cell::from("")])
}

/// What a section with nothing in it says: a search that turned up nothing
/// names what was typed, an empty External section (never filtered) points
/// at how to add one instead.
fn no_matches(filter: &str, fallback: &str) -> String {
    if filter.is_empty() { fallback.to_string() } else { format!("<none match \"{filter}\">") }
}

/// The render position (a row index into what `draw_extension_rows` actually
/// builds, headings, the spacer and any placeholder included) of logical
/// extension index `i`. The layout is always "Bundled" heading, then bundled
/// rows or (if there are none — only possible while filtering, bundled ships
/// at least one) a single placeholder row, a blank spacer, "External"
/// heading, then external rows or a placeholder the same way. Both section
/// heights are `.max(1)` for exactly that reason: a real section is as tall
/// as its rows, an empty one is one placeholder tall.
fn extension_render_index(i: usize, bundled_count: usize) -> usize {
    if i < bundled_count {
        return i + 1;
    }
    let bundled_h = bundled_count.max(1);
    bundled_h + 3 + (i - bundled_count)
}

/// The reverse of `extension_render_index`: which logical extension (if any)
/// sits at render position `r` — `None` on a heading, the spacer, a
/// placeholder, or past the end. The same `.max(1)` section-height rule as
/// `extension_render_index`, so the two can't disagree about where a row landed.
fn extension_at_render(r: usize, bundled_count: usize, total: usize) -> Option<usize> {
    let bundled_h = bundled_count.max(1);
    if r == 0 {
        return None; // "Bundled" heading
    }
    if r <= bundled_h {
        return (bundled_count > 0).then_some(r - 1);
    }
    let external_start = bundled_h + 3; // heading + bundled_h rows + spacer + heading
    if r < external_start {
        return None; // blank spacer, then "External" heading
    }
    let external_count = total - bundled_count;
    let offset = r - external_start;
    (offset < external_count.max(1) && external_count > 0).then(|| bundled_count + offset)
}

fn draw_extension_rows(frame: &mut Frame, area: Rect, extensions: &[ExtensionRow], filter: &str, state: &mut TableState) {
    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(extensions.len().saturating_sub(1))));
    }
    // The caller lists bundled ones first, so finding where they stop is the
    // whole split, not a real sort.
    let bundled_count = extensions.iter().take_while(|e| e.bundled).count();

    let mut rows: Vec<Row> = vec![section_heading("Bundled")];
    if bundled_count == 0 {
        rows.push(placeholder_row(no_matches(filter, "<none>")));
    } else {
        rows.extend(extensions[..bundled_count].iter().map(extension_row));
    }
    rows.push(blank_row());
    rows.push(section_heading("External"));
    if bundled_count < extensions.len() {
        rows.extend(extensions[bundled_count..].iter().map(extension_row));
    } else {
        rows.push(placeholder_row(no_matches(filter, "<none> — add one with `knav ext add <repo>`")));
    }

    // The heading/placeholder rows aren't selectable, so the selection needs
    // remapping from logical (an index into `extensions`, what the handler
    // indexes directly) to render position (a row in `rows` above, heading
    // rows included) — the same remap `extension_row_at` uses for mouse
    // clicks, or clicking a row and landing on the one above or below it
    // would disagree. The offset lives in render terms the whole time
    // instead: nothing outside this screen reads it, so there's no logical
    // meaning it needs to hold, and the widget already knows how to shift it
    // to keep the selection in view.
    let logical = state.selected().unwrap_or(0);
    let mut render_state = TableState::default().with_selected(Some(extension_render_index(logical, bundled_count))).with_offset(state.offset());

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled("EXTENSION", Style::default().fg(theme().muted))),
        Cell::from(Span::styled("PRESENT", Style::default().fg(theme().muted))),
        Cell::from(""),
    ]);
    // Wide enough for every bundled name today ("OPA Gatekeeper", 14 chars)
    // with headroom, and grows with whatever an external manifest names
    // itself rather than silently cutting it off (see `EXTENSION_NAME_MIN`).
    let name_width = extensions.iter().map(|e| e.name.chars().count()).max().unwrap_or(0).clamp(EXTENSION_NAME_MIN, EXTENSION_NAME_MAX) as u16;
    let table = Table::new(rows, [Constraint::Length(6), Constraint::Length(name_width), Constraint::Length(7), Constraint::Min(10)])
        .column_spacing(2)
        .style(theme_row(false))
        .header(header)
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, false));
    frame.render_stateful_widget(table, area, &mut render_state);
    state.select(Some(logical));
    *state.offset_mut() = render_state.offset();
}

/// The extensions browser's own screen: bundled ones under their own
/// heading, then anything added from `~/.config/knav/extensions/` under an
/// "External" heading of its own — the two are a different kind of thing
/// (shipped with knav vs. someone's own manifest), worth keeping visually
/// apart rather than one flat list. Both headings always show, even over
/// nothing, either because a search matched nothing in that section or
/// because External has nothing added yet — a placeholder row says which,
/// rather than the section just vanishing. `rows` is already filtered and
/// sorted by the caller (`extensions::visible_order`); this only knows how
/// to lay it out. A bad manifest shows its error instead of the on/off
/// toggle doing anything.
pub(in crate::ui) fn draw_extensions_popup(frame: &mut Frame, rows: &[ExtensionRow], filter: &str, filter_editing: bool, error: Option<&str>, state: &mut TableState) {
    let area = extensions_area(frame.area());
    frame.render_widget(Clear, area);
    let enabled = rows.iter().filter(|r| r.enabled).count();
    let title = pill_title(&format!("Extensions ({enabled}/{} on)", rows.len()), false, Style::default());
    let bottom = match error {
        Some(e) => Line::styled(format!(" {e} "), Style::default().fg(theme().bad)),
        None => hint_strip(&[("space/enter", "on/off"), ("/", "filter"), ("q/esc", "close")]).right_aligned(),
    };
    let block = with_search(Block::default().borders(Borders::ALL).border_set(border_set()).title(title).title_bottom(bottom), filter, filter_editing, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let parts = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(inner);
    let help = Paragraph::new("Off by default. Space or Enter toggles the selected one.").style(Style::default().fg(theme().muted)).wrap(Wrap { trim: true });
    frame.render_widget(help, parts[0]);
    draw_extension_rows(frame, parts[1], rows, filter, state);
}

/// Which extension a click on the Extensions screen lands on: converts the
/// click into a render-space row the same way `settings_row_at` does, then
/// translates it through the same heading-aware remap `draw_extension_rows`
/// uses, so a click can't land on the wrong row just because a
/// "Bundled"/"External" heading sits somewhere above it on screen.
/// `bundled_count`/`total` describe the loaded extensions the same way
/// `draw_extension_rows` derives them, just handed in rather than a typed
/// slice. `offset` is already in render terms (see `draw_extension_rows`),
/// the same value `state.offset()` holds for this screen.
pub fn extension_row_at(frame_area: Rect, bundled_count: usize, total: usize, offset: usize, row: u16) -> Option<usize> {
    let area = extensions_area(frame_area);
    let top = area.y + 1 /* border */ + 2 /* help */ + 1 /* column header */;
    let bottom = area.y + area.height.saturating_sub(1);
    if row < top || row >= bottom {
        return None;
    }
    let render_row = offset + usize::from(row - top);
    extension_at_render(render_row, bundled_count, total)
}

#[cfg(test)]
mod extension_row_tests {
    use super::*;

    #[test]
    fn every_logical_index_round_trips_through_its_render_position() {
        // 3 bundled, 2 external: headings at render 0 ("Bundled") and render 5 ("External"),
        // with a blank spacer at render 4.
        let (bundled_count, total) = (3, 5);
        for logical in 0..total {
            let r = extension_render_index(logical, bundled_count);
            assert_eq!(extension_at_render(r, bundled_count, total), Some(logical), "logical {logical} -> render {r} didn't round-trip");
        }
    }

    #[test]
    fn heading_and_spacer_rows_belong_to_nobody() {
        let (bundled_count, total) = (3, 5);
        assert_eq!(extension_at_render(0, bundled_count, total), None, "the \"Bundled\" heading");
        assert_eq!(extension_at_render(4, bundled_count, total), None, "the blank spacer");
        assert_eq!(extension_at_render(5, bundled_count, total), None, "the \"External\" heading");
        assert_eq!(extension_at_render(99, bundled_count, total), None, "past the end");
    }

    #[test]
    fn with_nothing_external_the_placeholder_row_belongs_to_nobody() {
        let (bundled_count, total) = (3, 3);
        assert_eq!(extension_at_render(0, bundled_count, total), None, "the \"Bundled\" heading");
        assert_eq!(extension_at_render(1, bundled_count, total), Some(0));
        assert_eq!(extension_at_render(3, bundled_count, total), Some(2));
        assert_eq!(extension_at_render(4, bundled_count, total), None, "the blank spacer");
        assert_eq!(extension_at_render(5, bundled_count, total), None, "the \"External\" heading");
        assert_eq!(extension_at_render(6, bundled_count, total), None, "the <none> placeholder row");
    }

    #[test]
    fn a_search_that_clears_the_bundled_section_still_round_trips_the_external_rows() {
        // A filter can match nothing bundled while still matching something external:
        // the Bundled section falls back to its own one-row placeholder instead of
        // disappearing, which shifts every external row down by one render position.
        let (bundled_count, total) = (0, 2);
        assert_eq!(extension_at_render(0, bundled_count, total), None, "the \"Bundled\" heading");
        assert_eq!(extension_at_render(1, bundled_count, total), None, "the bundled placeholder row");
        assert_eq!(extension_at_render(2, bundled_count, total), None, "the blank spacer");
        assert_eq!(extension_at_render(3, bundled_count, total), None, "the \"External\" heading");
        for logical in 0..total {
            let r = extension_render_index(logical, bundled_count);
            assert_eq!(extension_at_render(r, bundled_count, total), Some(logical), "logical {logical} -> render {r} didn't round-trip");
        }
    }

    #[test]
    fn with_nothing_loaded_at_all_both_sections_are_just_their_placeholders() {
        let (bundled_count, total) = (0, 0);
        for r in 0..6 {
            assert_eq!(extension_at_render(r, bundled_count, total), None, "render {r} with nothing loaded");
        }
    }
}
