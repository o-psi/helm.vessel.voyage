//! Displayed sidebar identities and a modal action editor, independent of composer drafts.
mod access;
mod input;
mod menu;
use super::{App, state::Target};
use crate::composer::Composer;
use ratatui::layout::Rect;
use uuid::Uuid;

#[derive(Clone, Copy, Default, PartialEq)]
pub(super) enum Focus {
    #[default]
    Composer,
    Voyages,
    Button,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Access,
    ReadOnly,
    Approval,
    Unrestricted,
    Rename,
    Archive,
    Restore,
    Branch,
    Cancel,
    Details,
    Clear,
    Compact,
    Delete,
}
impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Access => "Access →",
            Self::ReadOnly => "Read only",
            Self::Approval => "Ask first",
            Self::Unrestricted => "Unrestricted",
            Self::Rename => "Rename",
            Self::Archive => "Archive",
            Self::Restore => "Restore",
            Self::Branch => "Branch",
            Self::Cancel => "Cancel current run",
            Self::Details => "Details",
            Self::Clear => "Clear conversation",
            Self::Compact => "Compact older messages",
            Self::Delete => "Delete permanently",
        }
    }
}
pub(super) struct Menu {
    target: Target,
    incarnation: Uuid,
    run: Option<Uuid>,
    selected: usize,
    access_selected: usize,
    actions: Vec<Action>,
    scroll: std::cell::Cell<u16>,
    follow_selection: std::cell::Cell<bool>,
    editor: Option<Action>,
    revision: Option<u64>,
    text: Composer,
    error: String,
}
#[derive(Clone, Copy)]
pub(super) struct Hit {
    pub area: Rect,
    pub button: Rect,
    pub target: Target,
    pub incarnation: Uuid,
}
#[derive(Default)]
pub(super) struct Sidebar {
    pub resize: Resize,
    pub focus: Focus,
    pub pointer: Option<ratatui::layout::Position>,
    pub hits: std::cell::RefCell<Vec<Hit>>,
    pub action_trigger: std::cell::Cell<Option<Hit>>,
    pub menu: Option<Menu>,
    // Cleared every frame and bound to the exact menu target and incarnation.
    pub visible: std::cell::Cell<Option<(Target, Uuid, Option<Action>)>>,
    menu_hits: std::cell::RefCell<Vec<(Rect, Target, Uuid, usize)>>,
    controls: std::cell::RefCell<Vec<(Rect, crossterm::event::KeyCode)>>,
    area: std::cell::Cell<Rect>,
    scroll_bounds: std::cell::Cell<(u16, u16)>,
}
impl App {
    // Resolve against current geometry; hover never changes keyboard selection.
    pub(super) fn hover_style(&self, area: Rect, modal: bool) -> ratatui::style::Style {
        use ratatui::style::Style;
        let searching = self
            .selected
            .and_then(|t| self.views.get(&t))
            .is_some_and(|v| {
                v.transcript.borrow().search.is_some() && v.panel.is_none() && !v.terminals.open
            });
        if !self.help
            && self.explore.is_none()
            && !self.interactions.borrow().focused
            && self.sidebar.menu.is_some() == modal
            && (modal || !searching)
            && self
                .sidebar
                .pointer
                .is_some_and(|point| area.contains(point))
        {
            crate::theme::Role::Hover.style()
        } else {
            Style::default()
        }
    }

    fn sidebar_select(&mut self, target: Target) {
        self.selected = Some(target);
        self.sidebar.focus = Focus::Voyages;
        self.interactions.borrow_mut().focused = false;
        if let Some(view) = self.views.get_mut(&target) {
            view.terminals.clear_displayed();
        }
    }
    fn sidebar_composer(&mut self) {
        if self.sidebar.focus != Focus::Composer {
            self.interactions.borrow_mut().focused = false;
            if let Some(view) = self.selected.and_then(|target| self.views.get_mut(&target)) {
                view.panel = None;
                view.terminals.open = false;
            }
        }
        self.sidebar.focus = Focus::Composer;
    }

    fn open_actions(&mut self, target: Target) {
        self.sidebar.visible.set(None);
        self.sidebar.menu_hits.borrow_mut().clear();
        let Some(view) = self.views.get(&target) else {
            return;
        };
        self.sidebar.menu = Some(Menu {
            target,
            incarnation: view.process.incarnation,
            run: view
                .snapshot
                .as_ref()
                .and_then(|s| s.run.as_ref())
                .map(|r| r.run_id),
            selected: 0,
            access_selected: 0,
            actions: self.sidebar_actions(target),
            scroll: std::cell::Cell::new(0),
            follow_selection: std::cell::Cell::new(true),
            editor: None,
            revision: None,
            text: Composer::default(),
            error: String::new(),
        });
        self.sidebar.focus = Focus::Button;
    }
    fn sidebar_actions(&self, target: Target) -> Vec<Action> {
        let mut actions = vec![
            Action::Rename,
            if self.views.get(&target).is_some_and(|v| v.archived()) {
                Action::Restore
            } else {
                Action::Archive
            },
            Action::Branch,
            Action::Access,
        ];
        if self
            .views
            .get(&target)
            .and_then(|v| v.snapshot.as_ref())
            .and_then(|s| s.run.as_ref())
            .is_some_and(|r| r.active())
        {
            actions.push(Action::Cancel);
        }
        actions.extend([
            Action::Details,
            Action::Compact,
            Action::Clear,
            Action::Delete,
        ]);
        actions
    }
    fn action_reason(&self, menu: &Menu, action: Action) -> Option<&'static str> {
        let Some(view) = self.views.get(&menu.target) else {
            return Some("Voyage is no longer available");
        };
        if view.process.incarnation != menu.incarnation {
            return Some("Voyage restarted; reopen Actions");
        }
        if action == Action::Details {
            return None;
        }
        if !self.clients.available(menu.target.route) {
            return Some("Vessel unavailable · Ctrl+G to manage / retry");
        }
        if action == Action::Access && self.clients[menu.target.route].access_file.is_some() {
            return Some("Access changes require executing-account owner authority");
        }
        if view.pending.is_some() {
            return Some("Waiting for automatic confirmation of the pending action");
        }
        if action == Action::Restore && view.process.archive.is_some() {
            return None;
        }
        if view.archived() {
            return Some("Restore this voyage first");
        }
        if !matches!(
            view.process.state,
            voyage_protocol::process::ProcessState::Live
                | voyage_protocol::process::ProcessState::Suspended
        ) || view.error.is_some()
        {
            return Some("Voyage is unavailable");
        }
        let Some(snapshot) = &view.snapshot else {
            return Some("Waiting for voyage state");
        };
        if snapshot.lifecycle["deleted"] == true {
            return Some("History has been deleted");
        }
        if action == Action::Cancel {
            return if snapshot
                .run
                .as_ref()
                .is_some_and(|r| r.active() && Some(r.run_id) == menu.run)
            {
                None
            } else {
                Some("The selected run has ended or changed")
            };
        }
        if action != Action::Access && snapshot.run.as_ref().is_some_and(|r| r.active()) {
            return Some("Wait for the current run to finish");
        }
        if snapshot.pending_cleanup_run.is_some()
            && !(action == Action::Access && snapshot.run.as_ref().is_some_and(|r| r.active()))
        {
            return Some("Waiting for confirmed cleanup");
        }
        None
    }
}

// Rendering publishes geometry. Temporary constraints must not erase preference.
pub(super) struct Resize {
    preferred: u16,
    pane: std::cell::Cell<Option<Rect>>,
    divider: std::cell::Cell<Option<Rect>>,
    dragging: std::cell::Cell<bool>,
}
impl Default for Resize {
    fn default() -> Self {
        Self {
            preferred: 30,
            pane: Default::default(),
            divider: Default::default(),
            dragging: Default::default(),
        }
    }
}
impl Resize {
    pub fn width(&self, pane: Rect) -> u16 {
        if pane.width < 110 {
            0
        } else {
            self.preferred.clamp(24, Self::maximum(pane))
        }
    }
    fn maximum(pane: Rect) -> u16 {
        pane.width.saturating_sub(80).clamp(24, 64)
    }
    pub fn layout(&self, pane: Option<Rect>) {
        self.divider.set(pane.map(|pane| self.divider(pane)));
        if self.pane.replace(pane) != pane || pane.is_none() {
            self.dragging.set(false);
        }
    }
    pub fn clear(&self) {
        self.layout(None);
    }
    pub fn highlighted(&self, pointer: Option<ratatui::layout::Position>) -> bool {
        self.dragging.get()
            || self
                .divider
                .get()
                .is_some_and(|divider| pointer.is_some_and(|point| divider.contains(point)))
    }
    fn divider(&self, pane: Rect) -> Rect {
        Rect::new(pane.x + self.width(pane) - 1, pane.y, 1, pane.height)
    }
    pub fn input(&mut self, event: &crossterm::event::Event) -> bool {
        use crossterm::event::{Event, MouseButton, MouseEventKind};
        if matches!(event, Event::FocusLost | Event::Resize(..)) {
            self.clear();
            return false;
        }
        let Event::Mouse(mouse) = event else {
            // Keyboard input may change views or open overlays before repaint.
            self.dragging.set(false);
            return false;
        };
        let Some(pane) = self.pane.get() else {
            return false;
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self
                    .divider
                    .get()
                    .is_some_and(|divider| divider.contains((mouse.column, mouse.row).into()));
                self.dragging.set(hit);
                hit
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging.get() => {
                self.preferred = mouse
                    .column
                    .saturating_sub(pane.x)
                    .saturating_add(1)
                    .clamp(24, Self::maximum(pane));
                true
            }
            MouseEventKind::Up(_) => self.dragging.replace(false),
            _ => {
                self.dragging.set(false);
                false
            }
        }
    }
}

impl App {
    pub(super) fn action_editor(&self) -> Option<(super::right_panel::Editor, String)> {
        let menu = self.sidebar.menu.as_ref()?;
        let action = menu.editor?;
        if !matches!(
            action,
            Action::Rename | Action::Branch | Action::Delete | Action::Clear | Action::Compact
        ) || self.sidebar.visible.get() != Some((menu.target, menu.incarnation, menu.editor))
            || self.action_reason(menu, action).is_some()
        {
            return None;
        }
        Some((
            super::right_panel::Editor::Action(menu.target, menu.incarnation, action),
            menu.text.text.clone(),
        ))
    }
}

impl App {
    pub(super) fn draw_action_trigger(&self, frame: &mut ratatui::Frame<'_>, area: Rect) {
        let Some(target) = self.selected else {
            return;
        };
        let Some(view) = self.views.get(&target) else {
            return;
        };
        frame.render_widget(ratatui::widgets::Clear, area);
        frame.render_widget(
            ratatui::widgets::Paragraph::new("[Actions]").style(super::right_panel::control_style(
                self.sidebar.pointer,
                area,
                false,
                true,
            )),
            area,
        );
        self.sidebar.action_trigger.set(Some(Hit {
            area,
            button: area,
            target,
            incarnation: view.process.incarnation,
        }));
    }
}
