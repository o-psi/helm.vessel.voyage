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
        if self.answer_actions_input(event) {
            return true;
        }
        let view = self.selected.and_then(|t| self.views.get(&t)).unwrap();
        let mut state = view.transcript.borrow_mut();
        if matches!(event, Event::Resize(..)) {
            state.hits.clear();
            state.last_click = None;
        }
        if state.search.is_some() {
            match event {
                Event::Key(key) => match key.code {
                    KeyCode::Esc => {
                        state.search = None;
                        state.query.clear();
                        state.search_error = false;
                    }
                    KeyCode::Enter => {
                        state.search_previous = key.modifiers.contains(KeyModifiers::SHIFT);
                        state.query = state.search.clone().unwrap_or_default();
                        state.search_next = !state.query.is_empty();
                    }
                    KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Some(snapshot) = &view.snapshot {
                            let first = state
                                .messages
                                .first()
                                .map_or(snapshot.message_offset, |m| m.message_index);
                            state.older(first);
                        }
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
        if !matches!(event, Event::Mouse(mouse) if matches!(mouse.kind,
            MouseEventKind::Down(crossterm::event::MouseButton::Left)
            | MouseEventKind::Up(crossterm::event::MouseButton::Left)))
        {
            state.last_click = None;
        }
        let mut older = false;
        match event {
            Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL) => match key.code {
                KeyCode::Up | KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    state.disclosure_jump(key.code == KeyCode::Up);
                }
                KeyCode::Char(' ') => {
                    state.toggle_disclosure();
                }
                KeyCode::Char('t') => {
                    state.details = !state.details;
                    state.expanded.clear();
                    if !state.details
                        && let Some(anchor) = &mut state.anchor
                        && let super::Key::Activity(id) = anchor.key
                    {
                        anchor.key = super::Key::ActivityHeader(id);
                        anchor.offset = 0;
                    }
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
                KeyCode::Up | KeyCode::Down => {
                    let users = state
                        .messages
                        .iter()
                        .chain(view.snapshot.iter().flat_map(|s| &s.messages))
                        .filter(|m| m.role == "user")
                        .map(|m| m.message_index)
                        .collect::<Vec<_>>();
                    let previous = key.code == KeyCode::Up;
                    if !state.user_jump(&users, previous) && previous {
                        older = true;
                    }
                }
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
                MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                    let hit = state
                        .hits
                        .iter()
                        .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                        .map(|(_, key)| key.clone());
                    let Some(key) = hit else {
                        state.last_click = None;
                        return false;
                    };
                    if let super::Key::Sender(index) = key {
                        let source = state
                            .messages
                            .iter()
                            .chain(view.snapshot.iter().flat_map(|s| &s.messages))
                            .find(|m| m.message_index == index)
                            .and_then(|m| m.coordination.clone());
                        drop(state);
                        if let Some(source) = source {
                            self.open_coordination(source);
                        }
                        return true;
                    }
                    state.click(key, mouse.column, mouse.row, std::time::Instant::now());
                }
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
            state.older(first);
        }
        true
    }
}

impl super::State {
    fn click(&mut self, key: super::Key, column: u16, row: u16, now: std::time::Instant) {
        match &key {
            super::Key::ActivityHeader(id) => {
                let expanded = self.expanded.get(id).copied().unwrap_or(self.details);
                self.expanded.insert(*id, !expanded);
                self.last_click = None;
            }
            super::Key::Tool(id) => {
                let double = self
                    .last_click
                    .as_ref()
                    .is_some_and(|(previous, x, y, at)| {
                        previous == &key
                            && *x == column
                            && *y == row
                            && now.saturating_duration_since(*at)
                                <= std::time::Duration::from_millis(500)
                    });
                if !double {
                    self.last_click = Some((key, column, row, now));
                    return;
                }
                self.last_click = None;
                if !self.tool_expanded.remove(id) {
                    self.tool_expanded.insert(id.clone());
                }
            }
            _ => return,
        }
        self.remember(self.top);
        self.dirty = true;
    }
}

#[cfg(test)]
mod detail_checks {
    use super::super::{Key, State};
    use std::time::{Duration, Instant};

    #[test]
    fn double_click_is_independent_and_group_click_stays_single() {
        let mut state = State::default();
        let now = Instant::now();
        let a = Key::Tool("a".into());
        let b = Key::Tool("b".into());
        state.click(a.clone(), 5, 4, now);
        assert!(state.tool_expanded.is_empty());
        state.click(a.clone(), 5, 4, now + Duration::from_millis(100));
        assert!(state.tool_expanded.contains("a"));
        state.click(b.clone(), 5, 5, now);
        state.click(b.clone(), 5, 5, now + Duration::from_millis(600));
        assert!(!state.tool_expanded.contains("b"));
        state.click(b, 5, 5, now + Duration::from_millis(700));
        assert!(state.tool_expanded.contains("b"));
        state.click(a.clone(), 5, 4, now);
        state.click(a, 5, 4, now + Duration::from_millis(100));
        assert!(!state.tool_expanded.contains("a"));
        assert!(state.tool_expanded.contains("b"));
        state.click(Key::ActivityHeader(0), 5, 3, now);
        assert_eq!(state.expanded.get(&0), Some(&true));
    }
}

impl super::State {
    fn disclosure_jump(&mut self, previous: bool) -> bool {
        let current = self
            .anchor
            .as_ref()
            .and_then(|anchor| {
                self.rows
                    .iter()
                    .position(|row| row.key == anchor.key && row.offset >= anchor.offset)
            })
            .unwrap_or(self.top);
        let mut headers = self
            .rows
            .iter()
            .enumerate()
            .filter(|(i, row)| {
                matches!(row.key, super::Key::Tool(_) | super::Key::ActivityHeader(_))
                    && row.offset == 0
                    && (*i == 0 || self.rows[*i - 1].key != row.key)
            })
            .map(|(i, _)| i);
        let next = if previous {
            headers.filter(|i| *i < current).next_back()
        } else {
            headers.find(|i| *i > current)
        };
        if let Some(next) = next {
            self.remember(next);
            true
        } else {
            false
        }
    }
    fn toggle_disclosure(&mut self) -> bool {
        let key = self
            .anchor
            .as_ref()
            .map(|a| a.key.clone())
            .or_else(|| self.rows.get(self.top).map(|r| r.key.clone()));
        match key {
            Some(super::Key::Tool(id)) => {
                if !self.tool_expanded.remove(&id) {
                    self.tool_expanded.insert(id);
                }
            }
            Some(super::Key::ActivityHeader(id)) => {
                let expanded = self.expanded.get(&id).copied().unwrap_or(self.details);
                self.expanded.insert(id, !expanded);
            }
            _ => return false,
        }
        self.last_click = None;
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod keyboard_disclosure_tests {
    use super::super::{Key, Row, State};
    #[test]
    fn navigate_and_toggle_tools_reasoning_and_groups_without_composer_enter() {
        let mut state = State::default();
        for key in [
            Key::Message(0),
            Key::ActivityHeader(0),
            Key::Tool("call".into()),
            Key::Tool("reasoning:run:0:Summary:false".into()),
        ] {
            state.rows.push(Row {
                key,
                offset: 0,
                line: ratatui::text::Line::raw("header"),
            });
        }
        state.remember(0);
        assert!(!state.toggle_disclosure());
        assert!(!state.disclosure_jump(true));
        assert!(state.disclosure_jump(false));
        assert!(state.toggle_disclosure());
        assert_eq!(state.expanded.get(&0), Some(&true));
        assert!(state.disclosure_jump(false));
        assert!(state.toggle_disclosure());
        assert!(state.tool_expanded.contains("call"));
        assert!(state.disclosure_jump(false));
        assert!(state.toggle_disclosure());
        assert!(
            state
                .tool_expanded
                .contains("reasoning:run:0:Summary:false")
        );
        assert!(!state.disclosure_jump(false));
        assert!(state.toggle_disclosure());
        assert!(
            !state
                .tool_expanded
                .contains("reasoning:run:0:Summary:false")
        );
        assert!(state.tool_expanded.contains("call"));
        assert!(state.disclosure_jump(true));
        assert_eq!(state.top, 2);
    }
}

#[cfg(test)]
#[path = "input_coverage_tests.rs"]
mod coverage_tests;
