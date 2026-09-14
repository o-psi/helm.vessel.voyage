//! Named workspace navigation. Shortcuts remain optional; private/modal scopes win.
use super::{App, state::Target};
use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use std::cell::RefCell;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Menu,
    New,
    Find,
    Changes,
    Work,
    Copy,
    Tasks,
    Agents,
    Preferences,
    Paste,
    More,
    Stop,
    Commands,
    Settings,
    Connections,
    Programs,
    Browser,
    Workflows,
    Archives,
    Help,
    Leave,
}
impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Menu => "Helm ▾",
            Self::New => "+ New",
            Self::Find => "Find",
            Self::Changes => "Changes",
            Self::Work => "This conversation",
            Self::Copy => "Copy last answer",
            Self::Tasks => "Tasks",
            Self::Agents => "Delegated work",
            Self::Preferences => "Model and account",
            Self::Paste => "Paste text or image",
            Self::More => "More",
            Self::Stop => "Stop",
            Self::Commands => "Search all actions",
            Self::Settings => "Settings",
            Self::Connections => "Connections",
            Self::Programs => "Open terminal",
            Self::Browser => "Browser",
            Self::Workflows => "Saved workflows",
            Self::Archives => "Current / archived conversations",
            Self::Help => "Help and keyboard shortcuts",
            Self::Leave => "Leave Helm — work continues",
        }
    }
    fn contextual(self) -> bool {
        matches!(
            self,
            Self::Changes
                | Self::Work
                | Self::Copy
                | Self::Tasks
                | Self::Agents
                | Self::More
                | Self::Stop
                | Self::Programs
                | Self::Browser
                | Self::Workflows
        )
    }
}
const MENU: &[Action] = &[
    Action::New,
    Action::Find,
    Action::Work,
    Action::Preferences,
    Action::Paste,
    Action::Commands,
    Action::Archives,
    Action::Settings,
    Action::Connections,
    Action::Help,
    Action::Leave,
];
const WORK: &[Action] = &[
    Action::Changes,
    Action::Copy,
    Action::Tasks,
    Action::Agents,
    Action::Programs,
    Action::Browser,
    Action::Workflows,
    Action::More,
    Action::Stop,
];
#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    target: Option<(Target, Uuid)>,
    draft: Option<Uuid>,
}
#[derive(Clone, Copy)]
struct Hit {
    area: Rect,
    action: Action,
    identity: Identity,
}
#[derive(Default)]
pub(super) struct Navigation {
    menu: Option<(Identity, usize)>,
    work: bool,
    hits: RefCell<Vec<Hit>>,
}
impl App {
    pub(super) fn workspace_menu_open(&self) -> bool {
        self.workspace.menu.is_some()
    }
    fn workspace_entries(&self) -> &'static [Action] {
        if self.workspace.work { WORK } else { MENU }
    }
    pub(super) fn clear_workspace_hits(&self) {
        self.workspace.hits.borrow_mut().clear();
    }
    fn workspace_identity(&self) -> Identity {
        Identity {
            target: self
                .selected
                .and_then(|t| self.views.get(&t).map(|v| (t, v.process.incarnation))),
            draft: self.active_draft,
        }
    }
    pub(super) fn draw_workspace_chrome(&self) -> bool {
        self.viewport.get().is_some_and(|(w, h)| w >= 40 && h >= 18) && self.workspace_available()
    }
    fn workspace_available(&self) -> bool {
        !self.accounts.open()
            && !self.vessels_open()
            && self.workspace_picker.is_none()
            && self.stop_review.is_none()
            && !self.help
            && self.explore.is_none()
            && !self.workflows_open()
            && self.operator.is_none()
            && self.operator_loading.is_none()
            && self.voyage_picker.is_none()
            && !self.interactions.borrow().focused
            && self.sidebar.menu.is_none()
            && !self.inference_picker_open()
            && self.inspection.panel.is_none()
            && self.inspection.confirmation.is_none()
    }
    fn workspace_enabled(&self, action: Action) -> bool {
        if !action.contextual() {
            return true;
        }
        let Some(view) = self.selected.and_then(|t| self.views.get(&t)) else {
            return false;
        };
        if self.active_draft.is_some() {
            return false;
        }
        if action == Action::Stop {
            return view.pending.is_none()
                && view.snapshot.as_ref().is_some_and(|s| {
                    !s.recovery_pending
                        && s.run
                            .as_ref()
                            .is_some_and(|r| r.active() && r.state != "cancel_requested")
                });
        }
        true
    }
    pub(super) fn workspace_input(&mut self, event: &Event) -> Result<bool> {
        if matches!(event, Event::Resize(..)) {
            self.workspace.hits.borrow_mut().clear();
            return Ok(false);
        }
        if !self.workspace_available() {
            self.workspace.menu = None;
            self.workspace.hits.borrow_mut().clear();
            return Ok(false);
        }
        if matches!(event, Event::Key(k) if k.kind == KeyEventKind::Release) {
            return Ok(self.workspace.menu.is_some());
        }
        if matches!(event, Event::Key(k) if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('p'))
        {
            self.workspace.work = false;
            self.workspace.menu = if self.workspace.menu.is_some() {
                None
            } else {
                Some((self.workspace_identity(), 0))
            };
            self.workspace.hits.borrow_mut().clear();
            return Ok(true);
        }
        if let Some((identity, selected)) = self.workspace.menu {
            let entries = self.workspace_entries();
            if identity != self.workspace_identity() {
                self.workspace.menu = None;
                self.workspace.hits.borrow_mut().clear();
                self.status =
                    "Selection changed. Open the Helm menu again; your draft is retained.".into();
                return Ok(true);
            }
            if let Event::Key(k) = event {
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(k.code, KeyCode::Char('c' | 'q'))
                {
                    self.quit = true;
                } else {
                    match k.code {
                        KeyCode::Esc => self.workspace.menu = None,
                        KeyCode::Up | KeyCode::BackTab => {
                            self.workspace.menu =
                                Some((identity, (selected + entries.len() - 1) % entries.len()))
                        }
                        KeyCode::Down | KeyCode::Tab => {
                            self.workspace.menu = Some((identity, (selected + 1) % entries.len()))
                        }
                        KeyCode::Enter | KeyCode::Char(' ') if k.modifiers.is_empty() => {
                            self.workspace_activate(entries[selected])?
                        }
                        _ => {}
                    }
                }
                self.workspace.hits.borrow_mut().clear();
                return Ok(true);
            }
            if matches!(event, Event::Paste(_)) {
                return Ok(true);
            }
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let hit = self
                .workspace
                .hits
                .borrow()
                .iter()
                .find(|h| h.area.contains((mouse.column, mouse.row).into()))
                .copied();
            if let Some(hit) = hit {
                if hit.identity == self.workspace_identity() {
                    self.workspace_activate(hit.action)?;
                }
                self.workspace.hits.borrow_mut().clear();
                return Ok(true);
            }
            if self.workspace.menu.is_some() {
                self.workspace.menu = None;
                self.workspace.hits.borrow_mut().clear();
                return Ok(true);
            }
        }
        Ok(self.workspace.menu.is_some())
    }
    fn workspace_activate(&mut self, action: Action) -> Result<()> {
        if !self.workspace_enabled(action) {
            self.status = if action == Action::Stop {
                "No active run ready to stop."
            } else {
                "Select a conversation first using Find."
            }
            .into();
            return Ok(());
        }
        self.workspace.menu = None;
        match action {
            Action::Menu => {
                self.workspace.work = false;
                self.workspace.menu = Some((self.workspace_identity(), 0));
            }
            Action::Work => {
                self.workspace.work = true;
                self.workspace.menu = Some((self.workspace_identity(), 0));
            }
            Action::Copy => self.discovery_open("copy")?,
            Action::Tasks => self.discovery_open("todos")?,
            Action::Agents => self.discovery_open("subagents")?,
            Action::Preferences => self.open_model_options()?,
            Action::Paste => self.paste_clipboard_action()?,
            Action::New => self.create(None)?,
            Action::Find => self.open_voyage_picker(),
            Action::Changes => {
                self.discovery_open("diff")?;
            }
            Action::More => {
                if let Some(target) = self.selected {
                    self.open_actions(target);
                }
            }
            Action::Stop => {
                if let Some(target) = self.selected {
                    self.review_stop(target)?;
                }
            }
            Action::Commands => self.discovery_open("actions")?,
            Action::Settings => self.discovery_open("settings")?,
            Action::Connections => self.open_vessels(),
            Action::Programs => self.discovery_open("terminals")?,
            Action::Browser => self.discovery_open("browser")?,
            Action::Workflows => self.discovery_open("workflows")?,
            Action::Archives => self.show_archives(!self.archives),
            Action::Help => self.discovery_open("help")?,
            Action::Leave => self.quit = true,
        }
        Ok(())
    }
    pub(super) fn draw_workspace_bar(&self, frame: &mut Frame<'_>, area: Rect) {
        self.workspace.hits.borrow_mut().clear();
        if !self.workspace_available() {
            return;
        }
        let identity = self.workspace_identity();
        let mut x = area.x;
        let mut actions = vec![Action::Menu, Action::New, Action::Find];
        if self.selected.is_some() && self.active_draft.is_none() {
            if self.workspace_enabled(Action::Stop) {
                actions.push(Action::Stop);
            }
            actions.extend([Action::Changes, Action::Work]);
        }
        for action in actions {
            let text = format!(" {} ", action.label());
            let width = unicode_width::UnicodeWidthStr::width(text.as_str()) as u16;
            if x + width > area.right() {
                break;
            }
            let hit = Rect::new(x, area.y, width, 1);
            frame.render_widget(
                Paragraph::new(text).style(crate::theme::Role::Focus.style()),
                hit,
            );
            self.workspace.hits.borrow_mut().push(Hit {
                area: hit,
                action,
                identity,
            });
            x += width + 1;
        }
        let hint = "Ctrl+P menu";
        if area.right().saturating_sub(x) >= hint.len() as u16 + 2 {
            frame.render_widget(
                Paragraph::new(hint).style(crate::theme::Role::Muted.style()),
                Rect::new(
                    area.right() - hint.len() as u16,
                    area.y,
                    hint.len() as u16,
                    1,
                ),
            );
        }
    }
    pub(super) fn draw_workspace_menu(&self, frame: &mut Frame<'_>) {
        let Some((identity, selected)) = self.workspace.menu else {
            return;
        };
        if !self.workspace_available() || identity != self.workspace_identity() {
            return;
        }
        let area = frame.area();
        if area.width < 40 || area.height < 18 {
            return;
        }
        let entries = self.workspace_entries();
        let rect = Rect::new(
            area.x,
            area.y + 1,
            area.width.min(56),
            (entries.len() as u16 + 3).min(area.height - 1),
        );
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(if self.workspace.work {
                    " This conversation "
                } else {
                    " Helm "
                })
                .title_bottom(" ↑↓ Choose · Enter Open · Esc Back "),
            rect,
        );
        let items: Vec<_> = entries
            .iter()
            .map(|a| {
                ListItem::new(if self.workspace_enabled(*a) {
                    a.label().to_owned()
                } else {
                    format!("{} — unavailable", a.label())
                })
            })
            .collect();
        let list = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, entries.len() as u16);
        frame.render_stateful_widget(
            List::new(items).highlight_style(crate::theme::Role::Selection.style()),
            list,
            &mut ListState::default().with_selected(Some(selected)),
        );
        self.workspace
            .hits
            .borrow_mut()
            .retain(|h| h.area.y == area.y);
        for (n, action) in entries.iter().enumerate() {
            self.workspace.hits.borrow_mut().push(Hit {
                area: Rect::new(list.x, list.y + n as u16, list.width, 1),
                action: *action,
                identity,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn select_fixture(app: &mut App) -> Target {
        let route = app.clients.routes().next().unwrap();
        let session = Uuid::new_v4();
        let target = Target { route, session };
        let view = super::super::state::View::new(voyage_protocol::vessel::ProcessInfo {
            catalogue: None,
            archive: None,
            deletion: None,
            session_id: session,
            incarnation: Uuid::new_v4(),
            workspace: "/synthetic".into(),
            state: voyage_protocol::process::ProcessState::Live,
            name: Some("My conversation".into()),
        });
        app.views.insert(target, view);
        app.selected = Some(target);
        target
    }
    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(code, modifiers))
    }
    fn text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    }
    #[tokio::test]
    async fn conversation_menu_groups_work_without_application_settings() {
        let f = super::super::account_test_support::Fixture::new();
        let mut app = super::super::accounts::app_tests::app(f.0.path());
        select_fixture(&mut app);
        app.workspace_activate(Action::Work).unwrap();
        assert!(app.workspace.work);
        assert!(app.workspace_entries().contains(&Action::Copy));
        assert!(app.workspace_entries().contains(&Action::Tasks));
        assert!(!app.workspace_entries().contains(&Action::Connections));
        let mut terminal = Terminal::new(TestBackend::new(40, 18)).unwrap();
        terminal
            .draw(|f| super::super::render::draw(f, &app))
            .unwrap();
        for label in [
            "This conversation",
            "Copy last answer",
            "Delegated work",
            "Open terminal",
        ] {
            assert!(text(&terminal).contains(label));
        }
        app.workspace_input(&key(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        app.workspace_activate(Action::Menu).unwrap();
        assert!(!app.workspace.work);
        assert!(app.workspace_entries().contains(&Action::Connections));
    }

    #[tokio::test]
    async fn named_controls_render_and_menu_preserves_composer_at_both_widths() {
        let fixture = super::super::account_test_support::Fixture::new();
        let mut app = super::super::accounts::app_tests::app(fixture.0.path());
        let target = select_fixture(&mut app);
        app.views
            .get_mut(&target)
            .unwrap()
            .draft
            .insert_str("keep this draft");
        for (w, h) in [(40, 18), (100, 32)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| super::super::render::draw(f, &app))
                .unwrap();
            let rendered = text(&terminal);
            assert!(rendered.contains("Helm ▾"), "{rendered}");
            assert!(rendered.contains("+ New"));
            assert!(rendered.contains("Find"));
            for obsolete in [
                "F1 Help",
                "F2 Voyages",
                "F3 Console",
                "F8 Explore",
                "F9 Actions",
                "Vessels [Ctrl+G]",
            ] {
                assert!(!rendered.contains(obsolete), "{obsolete}: {rendered}");
            }
            app.input(key(KeyCode::Char('p'), KeyModifiers::CONTROL))
                .unwrap();
            terminal
                .draw(|f| super::super::render::draw(f, &app))
                .unwrap();
            assert!(text(&terminal).contains("Connections"));
            app.input(Event::Paste("must not reach draft".into()))
                .unwrap();
            app.input(key(KeyCode::Esc, KeyModifiers::NONE)).unwrap();
            assert_eq!(app.views[&target].draft.text, "keep this draft");
        }
    }
    #[tokio::test]
    async fn find_uses_named_picker_and_pointer_targets_expire_on_resize() {
        let fixture = super::super::account_test_support::Fixture::new();
        let mut app = super::super::accounts::app_tests::app(fixture.0.path());
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| super::super::render::draw(f, &app))
            .unwrap();
        let hit = *app
            .workspace
            .hits
            .borrow()
            .iter()
            .find(|h| h.action == Action::Find)
            .unwrap();
        let click = Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.area.x,
            row: hit.area.y,
            modifiers: KeyModifiers::NONE,
        });
        app.input(Event::Resize(81, 24)).unwrap();
        assert!(app.workspace.hits.borrow().is_empty());
        app.workspace_input(&click).unwrap();
        assert!(app.voyage_picker.is_none());
        terminal
            .draw(|f| super::super::render::draw(f, &app))
            .unwrap();
        app.input(click).unwrap();
        assert!(app.voyage_picker.is_some());
        app.input(key(KeyCode::Esc, KeyModifiers::NONE)).unwrap();
        app.input(key(KeyCode::Char('p'), KeyModifiers::CONTROL))
            .unwrap();
        app.input(key(KeyCode::Down, KeyModifiers::NONE)).unwrap();
        app.input(key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
        assert!(app.voyage_picker.is_some());
    }
    #[tokio::test]
    async fn context_change_or_private_scope_cannot_dispatch_stale_menu_action() {
        let fixture = super::super::account_test_support::Fixture::new();
        let mut app = super::super::accounts::app_tests::app(fixture.0.path());
        select_fixture(&mut app);
        app.workspace_input(&key(KeyCode::Char('p'), KeyModifiers::CONTROL))
            .unwrap();
        app.selected = None;
        app.workspace_input(&key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(app.workspace.menu.is_none());
        assert!(app.new_drafts.is_empty());
        app.help = true;
        assert!(
            !app.workspace_input(&key(KeyCode::Char('p'), KeyModifiers::CONTROL))
                .unwrap()
        );
        assert!(app.workspace.menu.is_none());
        app.help = false;
        app.workspace_activate(Action::Stop).unwrap();
        assert!(app.stop_review.is_none());
        assert!(app.status.contains("No active run"));
    }
}
