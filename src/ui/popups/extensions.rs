//! The Extensions screen (`E`): turn extensions on or off, see whether each one's kinds
//! exist on this cluster, and search.

use super::*;

fn extensions_area(frame: Rect) -> Rect {
    centered_rect(94, 88, frame)
}

/// EXTENSION column width bounds: at least the header, at most enough to leave the
/// description room.
const EXTENSION_NAME_MIN: usize = 10;
const EXTENSION_NAME_MAX: usize = 28;

/// One extension's row: status, name, presence and description. An error replaces
/// the description.
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
    // On/off says whether you turned it on; present says whether this cluster has
    // its kinds. Separate columns, since one doesn't answer the other.
    let (present, present_style) = match row.present {
        Some(true) => ("Present".to_string(), Style::default().fg(theme().ok)),
        Some(false) => ("Missing".to_string(), Style::default().fg(theme().warn)),
        None => ("-".to_string(), Style::default().fg(theme().muted)),
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

/// What an empty section says: the search that found nothing, or `fallback`.
fn no_matches(filter: &str, fallback: &str) -> String {
    if filter.is_empty() { fallback.to_string() } else { format!("<none match \"{filter}\">") }
}

/// Where logical extension `i` is drawn, counting the headings, spacer and any
/// placeholder rows. An empty section still takes one placeholder row.
fn extension_render_index(i: usize, bundled_count: usize) -> usize {
    if i < bundled_count {
        return i + 1;
    }
    let bundled_h = bundled_count.max(1);
    bundled_h + 3 + (i - bundled_count)
}

/// The reverse of `extension_render_index`: the extension at render row `r`, `None`
/// on a heading, spacer, placeholder or past the end.
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
    // The caller lists bundled ones first, so this finds the split.
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
        rows.push(placeholder_row(no_matches(filter, "<none>: add one under ~/.config/knav/extensions/<id>/manifest.toml")));
    }

    // The selection is kept as an extension index and mapped to its render row, the
    // same mapping mouse clicks use. The scroll offset stays in render rows.
    let logical = state.selected().unwrap_or(0);
    let mut render_state = TableState::default().with_selected(Some(extension_render_index(logical, bundled_count))).with_offset(state.offset());

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled("EXTENSION", Style::default().fg(theme().muted))),
        Cell::from(Span::styled("PRESENT", Style::default().fg(theme().muted))),
        Cell::from(""),
    ]);
    // Grows with the longest name, up to the cap.
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

/// The Extensions screen: bundled extensions, then external ones, each under a heading
/// that shows even when empty. `rows` is already filtered and sorted.
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

/// The extension a click lands on, mapped through the same heading-aware rows the
/// drawing uses. `offset` is in render rows, like `state.offset()` here.
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
        // 3 bundled and 2 external: headings at rows 0 and 5, a spacer at row 4.
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
        // Nothing bundled matches but something external does: the Bundled placeholder
        // takes a row, shifting the external rows down by one.
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
