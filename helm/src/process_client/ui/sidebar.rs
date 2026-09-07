//! Displayed sidebar identities and a modal action editor, independent of composer drafts.
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
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Action {
    Rename,
    Archive,
    Restore,
    Branch,
    Cancel,
    Details,
    Delete,
}
impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Rename => "Rename",
            Self::Archive => "Archive",
            Self::Restore => "Restore",
            Self::Branch => "Branch",
            Self::Cancel => "Cancel current run",
            Self::Details => "Details",
            Self::Delete => "Delete permanently",
        }
    }
}
pub(super) struct Menu {
    target: Target,
    incarnation: Uuid,
    run: Option<Uuid>,
    selected: usize,
    actions: Vec<Action>,
    scroll: u16,
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
    pub focus: Focus,
    pub hits: std::cell::RefCell<Vec<Hit>>,
    pub menu: Option<Menu>,
    // Cleared every frame and bound to the exact menu target and incarnation.
    pub visible: std::cell::Cell<Option<(Target, Uuid, Option<Action>)>>,
    menu_hits: std::cell::RefCell<Vec<(Rect, Target, Uuid, usize)>>,
}
impl App {
    fn sidebar_select(&mut self, target: Target) {
        self.selected = Some(target);
        self.sidebar.focus = Focus::Voyages;
        self.interactions.borrow_mut().focused = false;
        if let Some(view) = self.views.get_mut(&target) {
            view.unread = false;
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
            actions: self.sidebar_actions(target),
            scroll: 0,
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
        actions.extend([Action::Details, Action::Delete]);
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
        if view.pending.is_some() {
            return Some("Check the pending action with F4 first");
        }
        if action == Action::Restore && view.process.archive.is_some() {
            return None;
        }
        if view.archived() {
            return Some("Restore this voyage first");
        }
        if view.process.state != voyage_protocol::process::ProcessState::Live
            || view.error.is_some()
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
        if snapshot.run.as_ref().is_some_and(|r| r.active()) {
            return Some("Wait for the current run to finish");
        }
        if snapshot.pending_cleanup_run.is_some() {
            return Some("Waiting for confirmed cleanup");
        }
        None
    }
}
