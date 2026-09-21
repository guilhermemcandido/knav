//! The object summary: scroll it, or switch to the YAML.

use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<Outcome>> {
    let mut to_yaml = false;
    if let Mode::Details { sections, scroll, back, .. } = &mut st.mode {
        let total = ui::details_line_count(sections, cx.frame_area);
        let last = total.saturating_sub(1);
        let page = usize::from(cx.frame_area.height.saturating_sub(8)).max(1);
        match event {
            Event::Key(key) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => st.mode = std::mem::replace(&mut **back, Mode::List),
                    KeyCode::Char('j') | KeyCode::Down => *scroll = (*scroll + 1).min(last),
                    KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
                    KeyCode::Char('g') | KeyCode::Home => *scroll = 0,
                    KeyCode::Char('G') | KeyCode::End => *scroll = last,
                    KeyCode::Char('f') if ctrl => *scroll = (*scroll + page).min(last),
                    KeyCode::PageDown => *scroll = (*scroll + page).min(last),
                    KeyCode::Char('b') if ctrl => *scroll = scroll.saturating_sub(page),
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(page),
                    KeyCode::Char('y') => to_yaml = true,
                    _ => {}
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => *scroll = (*scroll + 3).min(last),
                MouseEventKind::ScrollUp => *scroll = scroll.saturating_sub(3),
                _ => {}
            },
            _ => {}
        }
    }
    if to_yaml && let Mode::Details { manifest, .. } = &st.mode {
        let title = crate::app::mode::object_title(manifest);
        let text = serde_yaml::to_string(manifest).unwrap_or_default();
        let back = std::mem::replace(&mut st.mode, Mode::List);
        st.mode = Mode::Yaml { title, text, scroll: 0, back: Box::new(back) };
    }
    Ok(None)
}
