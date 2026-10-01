//! The object summary: scroll it, or switch to the YAML.

use super::super::*;
use super::Cx;

pub(super) fn handle(event: Event, st: &mut State, cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let mut to_yaml = false;
    let mut toggle = false;
    let mut open: Option<(ResourceKind, Option<String>, String)> = None;
    if let Mode::Details { sections, scroll, hscroll, back, .. } = &mut st.mode {
        let (last, widest) = ui::details_max_scroll(sections, cx.frame_area);
        let page = usize::from(cx.frame_area.height.saturating_sub(8)).max(1);
        match event {
            Event::Key(key) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        st.reveal = true;
                        st.mode = std::mem::replace(&mut **back, Mode::List);
                    }
                    KeyCode::Char('j') | KeyCode::Down => *scroll = (*scroll + 1).min(last),
                    KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
                    KeyCode::Char('g') | KeyCode::Home => *scroll = 0,
                    KeyCode::Char('G') | KeyCode::End => *scroll = last,
                    KeyCode::Char('f') if ctrl => *scroll = (*scroll + page).min(last),
                    KeyCode::PageDown => *scroll = (*scroll + page).min(last),
                    KeyCode::Char('b') if ctrl => *scroll = scroll.saturating_sub(page),
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(page),
                    KeyCode::Left | KeyCode::Char('h') => *hscroll = hscroll.saturating_sub(6),
                    KeyCode::Right | KeyCode::Char('l') => *hscroll = (*hscroll + 6).min(widest),
                    KeyCode::Char('y') => to_yaml = true,
                    KeyCode::Char('x') => toggle = true,
                    // Enter goes to the object's own list.
                    KeyCode::Enter => {
                        if let Mode::Details { manifest, .. } = &st.mode {
                            let text = |path: &[&str]| path.iter().try_fold(manifest, |v, key| v.get(*key)).and_then(|v| v.as_str()).map(String::from);
                            if let (Some(kind), Some(name)) = (text(&["kind"]).and_then(|k| ResourceKind::from_owner_kind(&k)), text(&["metadata", "name"])) {
                                open = Some((kind, text(&["metadata", "namespace"]), name));
                            }
                        }
                    }
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
    if toggle {
        st.reveal = !st.reveal;
        if let Mode::Details { manifest, sections, .. } = &mut st.mode {
            let view = cx.catalog.view_for(&cx.config.extensions.enabled, manifest);
            *sections = k8s::details::details(manifest, &cx.d.overview.events, crate::app::live_usage(cx.d.pod_usage.as_deref(), cx.d.usage.as_ref()), st.reveal, view);
        }
    }
    if let Some((kind, namespace, name)) = open {
        // The details view is one of the steps q or Esc undoes, like a drill-down.
        let snap = st.list_snapshot();
        st.back_stack.push(Step::Mode(Box::new(std::mem::replace(&mut st.mode, Mode::List)), snap));
        st.jump_to_object(kind, namespace.as_deref(), &name);
        // `jump_to_object` pushed the list too, which the step above already covers.
        st.back_stack.pop();
    }
    if to_yaml && let Mode::Details { manifest, .. } = &st.mode {
        let title = crate::app::mode::object_title(manifest);
        let text = serde_yaml::to_string(manifest).unwrap_or_default();
        let back = std::mem::replace(&mut st.mode, Mode::List);
        st.mode = Mode::Yaml { title, text, scroll: 0, back: Box::new(back) };
    }
    Ok(None)
}
