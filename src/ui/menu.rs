//! The `m` resource-switcher menu.

use super::*;

/// Same movement rules as `move_selection`, for the resource-switcher
/// menu's own section/tile grid.
pub fn move_menu_selection(sections: &[MenuSection], cols: usize, current: (usize, usize), dir: Direction) -> (usize, usize) {
    let lens: Vec<usize> = sections.iter().map(|s| s.tiles.len()).collect();
    move_selection(&lens, cols, current, dir)
}

/// The menu popup's tile-grid column count, computed from the popup's
/// actual inner area so keyboard navigation and mouse hit-testing can't
/// drift from what's rendered.
pub fn menu_cols(frame_area: Rect) -> usize {
    let area = centered_rect(70, 85, frame_area);
    let inner = Block::default().borders(Borders::ALL).inner(area);
    (inner.width / TILE_WIDTH).max(1) as usize
}

/// The "switch resource" menu: rounded tiles grouped by section, Freelens-style.
pub(super) fn draw_menu_popup(frame: &mut Frame, sections: &[MenuSection], selected: (usize, usize)) {
    let area = centered_rect(70, 85, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .title(pill_title("Resources", false, Style::default()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Sections wrap tiles into rows of `cols`, so the menu can exceed the popup. Lay
    // it out on a virtual canvas and scroll just enough to keep the selected tile in
    // view; tiles not fully inside are skipped, never squeezed.
    let cols = menu_cols(frame.area());
    let view_h = inner.height;
    let mut y: u16 = 0;
    let mut selected_bottom: u16 = 0;
    // (virtual y, section index, row index) for each tile row, plus each
    // section's title y, collected first so the scroll is known before
    // anything is drawn.
    let mut layout: Vec<(u16, usize, usize)> = Vec::new();
    let mut titles: Vec<u16> = Vec::new();
    for (section_idx, section) in sections.iter().enumerate() {
        titles.push(y);
        y += 1;
        for row in 0..section.tiles.len().div_ceil(cols).max(1) {
            layout.push((y, section_idx, row));
            if selected.0 == section_idx && selected.1 / cols == row {
                selected_bottom = y + 3;
            }
            y += 3;
        }
    }
    let scroll = selected_bottom.saturating_sub(view_h);
    let fits = |top: u16, h: u16| top >= scroll && top + h <= scroll + view_h;

    for (section_idx, section) in sections.iter().enumerate() {
        if fits(titles[section_idx], 1) {
            let title_area = Rect { x: inner.x, y: inner.y + titles[section_idx] - scroll, width: inner.width, height: 1 };
            frame.render_widget(Paragraph::new(Line::styled(section.title, Style::default().add_modifier(Modifier::BOLD))), title_area);
        }
    }

    for &(row_y, section_idx, row) in &layout {
        if !fits(row_y, 3) {
            continue;
        }
        let section = &sections[section_idx];
        let start = row * cols;
        let row_tiles = &section.tiles[start..(start + cols).min(section.tiles.len())];
        let row_area = Rect { x: inner.x, y: inner.y + row_y - scroll, width: inner.width, height: 3 };
        let tile_constraints: Vec<Constraint> = row_tiles.iter().map(|_| Constraint::Ratio(1, row_tiles.len() as u32)).collect();
        let tile_areas = Layout::horizontal(tile_constraints).split(row_area);

        for (col, (tile_area, kind)) in tile_areas.iter().zip(row_tiles.iter()).enumerate() {
            let is_selected = selected == (section_idx, start + col);
            // The selected tile gets a solid background, since a coloured border alone was
            // too subtle.
            let (border_style, text_style) = if is_selected {
                (Style::default().fg(theme().namespace), Style::default().bg(theme().namespace).fg(crate::theme::on(theme().namespace)).add_modifier(Modifier::BOLD))
            } else {
                (Style::default(), Style::default())
            };
            let tile = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).style(text_style);
            let label = Paragraph::new(kind.label()).alignment(Alignment::Center).style(text_style).block(tile);
            frame.render_widget(label, *tile_area);
        }
    }
}
