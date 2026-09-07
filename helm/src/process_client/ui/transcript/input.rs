use super::super::App;
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseEventKind};
impl App {
    pub(in crate::process_client::ui) fn transcript_input(&mut self, event: &Event) -> bool {
        if matches!(event, Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Release) {
            return false;
        }
        if matches!(event, Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'q')))
        {
            return false;
        }
        if self.help
            || self.explore.is_some()
            || self.sidebar.menu.is_some()
            || self.interactions.borrow().focused
        {
            return false;
        }
        let Some(view) = self.selected.and_then(|t| self.views.get(&t)) else {
            return false;
        };
        if view.panel.is_some() || view.terminals.open {
            return false;
        }
        let mut state = view.transcript.borrow_mut();
        if state.search.is_some() {
            match event {
                Event::Key(key) => match key.code {
                    KeyCode::Esc => {
                        state.search = None;
                        state.query.clear();
                        state.search_error = false;
                    }
                    KeyCode::Enter => {
                        state.query = state.search.clone().unwrap_or_default();
                        state.search_next = !state.query.is_empty();
                    }
                    KeyCode::Backspace => {
                        state.search.as_mut().unwrap().pop();
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            && state.search.as_ref().unwrap().chars().count() < 256 =>
                    {
                        state.search.as_mut().unwrap().push(c);
                    }
                    _ => {}
                },
                Event::Paste(text) => {
                    let text = crate::process_client::safe(text);
                    let available =
                        256usize.saturating_sub(state.search.as_ref().unwrap().chars().count());
                    state
                        .search
                        .as_mut()
                        .unwrap()
                        .extend(text.chars().take(available));
                }
                _ => {}
            }
            return true;
        }
        let mut older = false;
        match event {
            Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL) => match key.code {
                KeyCode::Char('t') => {
                    state.details = !state.details;
                    state.dirty = true;
                }
                KeyCode::Char('f') => {
                    state.search = Some(String::new());
                    state.search_error = false;
                }
                KeyCode::End => {
                    state.anchor = None;
                    state.new_output = false;
                    state.search_error = false;
                }
                KeyCode::Home => older = true,
                _ => return false,
            },
            Event::Key(key) => match key.code {
                KeyCode::PageUp => {
                    older = state.top == 0;
                    let amount = state.height.saturating_sub(2).max(1);
                    state.scroll(true, amount);
                }
                KeyCode::PageDown => {
                    let amount = state.height.saturating_sub(2).max(1);
                    state.scroll(false, amount);
                }
                _ => return false,
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    older = state.top == 0;
                    state.scroll(true, 3);
                }
                MouseEventKind::ScrollDown => state.scroll(false, 3),
                _ => return false,
            },
            _ => return false,
        }
        if older && let Some(snapshot) = &view.snapshot {
            let first = state
                .messages
                .first()
                .map_or(snapshot.message_offset, |m| m.message_index);
            state.requested_from = Some(first.saturating_sub(128));
            if !state.loading {
                state.attempted = None;
                state.live_attempt = None;
            }
            state.remember(0);
        }
        true
    }
}
