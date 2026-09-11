//! Name-based navigation with pinned identities, independent of sidebar width.
use super::{App, state::Target};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination {
    Voyage(Target),
    Draft(Uuid),
}
struct Entry {
    destination: Destination,
    name: String,
    detail: String,
}
#[derive(Default)]
pub(super) struct Picker {
    entries: Vec<Entry>,
    query: String,
    selected: usize,
}
impl Picker {
    fn filtered(&self) -> Vec<&Entry> {
        let query = self.query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                format!("{} {}", e.name, e.detail)
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }
    fn input(&mut self, event: &Event) -> Option<Destination> {
        match event {
            Event::Paste(text) => {
                self.append(text);
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.filtered().len().saturating_sub(1))
                }
                KeyCode::Home => self.selected = 0,
                KeyCode::End => self.selected = self.filtered().len().saturating_sub(1),
                KeyCode::Backspace => {
                    self.query.pop();
                    self.selected = 0;
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.append(&ch.to_string())
                }
                KeyCode::Enter if key.kind == KeyEventKind::Press => {
                    return self.filtered().get(self.selected).map(|e| e.destination);
                }
                _ => {}
            },
            _ => {}
        }
        None
    }
    fn append(&mut self, text: &str) {
        for ch in super::safe(text).chars().filter(|ch| !ch.is_control()) {
            if self.query.len() + ch.len_utf8() > 256 {
                break;
            }
            self.query.push(ch);
        }
        self.selected = 0;
    }
}
impl App {
    pub(super) fn voyage_picker_input(&mut self, event: &Event) -> bool {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return self.voyage_picker.is_some();
            }
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'q'))
            {
                return false;
            }
            if key.code == KeyCode::F(2) && key.kind == KeyEventKind::Press {
                if self.voyage_picker.take().is_some() {
                    return true;
                }
                // Do not steal input from a private form or a consent review.
                if self.sidebar.menu.is_some() || self.inference_picker_open() {
                    return false;
                }
                self.cancel_paste_for_private_panel();
                self.help = false;
                self.explore = None;
                let mut entries = Vec::new();
                for (id, draft) in &self.new_drafts {
                    entries.push(Entry {
                        destination: Destination::Draft(*id),
                        name: format!("Draft: {}", super::safe(draft.navigation_title())),
                        detail: format!(
                            "{} · {}",
                            self.route_label(draft.route),
                            draft.navigation_workspace().display()
                        ),
                    });
                }
                for target in self.ordered_targets() {
                    let view = &self.views[&target];
                    let status = if view.archived() {
                        "Archived".to_owned()
                    } else if view.connection_unavailable {
                        "Vessel unavailable · cached conversation".to_owned()
                    } else {
                        view.snapshot
                            .as_ref()
                            .map_or("Observation unavailable", super::presentation::voyage_state)
                            .to_owned()
                    };
                    entries.push(Entry {
                        destination: Destination::Voyage(target),
                        name: view.title(),
                        detail: format!("{} · {}", self.route_label(target.route), status),
                    });
                }
                let selected = entries
                    .iter()
                    .position(|e| match e.destination {
                        Destination::Draft(id) => self.active_draft == Some(id),
                        Destination::Voyage(t) => {
                            self.active_draft.is_none() && self.selected == Some(t)
                        }
                    })
                    .unwrap_or(0);
                self.voyage_picker = Some(Picker {
                    entries,
                    selected,
                    ..Default::default()
                });
                self.sidebar.resize.clear();
                return true;
            }
            if self.voyage_picker.is_some() && key.code == KeyCode::Esc {
                self.voyage_picker = None;
                return true;
            }
        }
        let Some(picker) = &mut self.voyage_picker else {
            return false;
        };
        if let Some(destination) = picker.input(event) {
            match destination {
                Destination::Draft(id) if self.new_drafts.contains_key(&id) => {
                    self.select_draft(id);
                    self.voyage_picker = None;
                }
                Destination::Voyage(target)
                    if self
                        .views
                        .get(&target)
                        .is_some_and(|v| !v.deleted() && v.archived() == self.archives) =>
                {
                    self.active_draft = None;
                    self.selected = Some(target);
                    self.sidebar.focus = super::sidebar::Focus::Voyages;
                    self.views
                        .get_mut(&target)
                        .expect("checked target")
                        .terminals
                        .clear_displayed();
                    self.voyage_picker = None;
                }
                _ => {
                    self.status =
                        "This item changed. Close and reopen F2; your current draft is retained."
                            .into()
                }
            }
        }
        true
    }
}
pub(super) fn draw(frame: &mut Frame<'_>, picker: &Picker, area: Rect) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Find voyages · F2 ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(format!("Search: {}", super::safe(&picker.query))),
        rows[0],
    );
    let entries = picker.filtered();
    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No matching voyages or drafts.\nEsc returns without changing your draft.",
            ),
            rows[1],
        );
    } else {
        let items: Vec<_> = entries
            .iter()
            .map(|e| {
                ListItem::new(vec![
                    Line::from(super::safe(&e.name)),
                    Line::styled(super::safe(&e.detail), crate::theme::Role::Muted.style()),
                ])
            })
            .collect();
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol("> ")
                .highlight_style(crate::theme::Role::Selection.style()),
            rows[1],
            &mut ListState::default().with_selected(Some(picker.selected)),
        );
    }
    frame.render_widget(
        Paragraph::new(
            "Type to search · ↑↓ choose · Enter open\nEsc back · F5 Archives after closing",
        ),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    fn picker() -> Picker {
        Picker {
            entries: vec![
                Entry {
                    destination: Destination::Draft(Uuid::from_u128(1)),
                    name: "First voyage".into(),
                    detail: "remote · Needs you".into(),
                },
                Entry {
                    destination: Destination::Draft(Uuid::from_u128(2)),
                    name: "日本語".into(),
                    detail: "local · Ready".into(),
                },
            ],
            ..Default::default()
        }
    }
    #[test]
    fn search_paste_is_bounded_and_never_opens_or_redirects() {
        let mut p = picker();
        assert_eq!(p.input(&Event::Paste("日本語\n".into())), None);
        assert_eq!(p.filtered().len(), 1);
        assert_eq!(
            p.input(&Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            ))),
            Some(Destination::Draft(Uuid::from_u128(2)))
        );
        p.append(&"界".repeat(1000));
        assert!(p.query.len() <= 256);
    }
    #[test]
    fn empty_results_and_repeated_enter_do_not_select() {
        let mut p = picker();
        p.append("missing");
        assert_eq!(
            p.input(&Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            ))),
            None
        );
        p.query.clear();
        let mut key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        key.kind = KeyEventKind::Repeat;
        assert_eq!(p.input(&Event::Key(key)), None);
    }
    #[test]
    fn constrained_and_wide_picker_frames_show_essential_controls() {
        for (width, height) in [(40, 18), (80, 24), (120, 32), (180, 48)] {
            let backend = ratatui::backend::TestBackend::new(width, height);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| draw(frame, &picker(), frame.area()))
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            for expected in [
                "Find voyages",
                "Search:",
                "First voyage",
                "Enter open",
                "Esc back",
            ] {
                assert!(
                    text.contains(expected),
                    "{width}x{height}: missing {expected}"
                );
            }
        }
    }
}
