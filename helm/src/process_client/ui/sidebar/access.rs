//! Access is edited on the owner, never reconstructed from public configuration.
use super::*;
use anyhow::{Context, Result};
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    text::{Line, Text},
    widgets::Paragraph,
};
use voyage_protocol::vessel::VoyageCommand;

const MODES: [(Action, &str, &str); 3] = [
    (
        Action::ReadOnly,
        "read-only",
        "Read files; restrict changes and modifying commands.",
    ),
    (
        Action::Approval,
        "approval",
        "Ask for permission when an action requires approval.",
    ),
    (
        Action::Unrestricted,
        "unrestricted",
        "Run actions without asking, within configured limits.",
    ),
];
impl App {
    pub(in crate::process_client::ui) fn open_access(
        &mut self,
        target: Target,
        mode: Option<&str>,
    ) -> Result<()> {
        let choice = mode
            .map(|mode| {
                MODES
                    .iter()
                    .position(|(_, value, _)| *value == mode.trim())
                    .context("use /access read-only, approval or unrestricted")
            })
            .transpose()?;
        self.open_actions(target);
        let mut menu = self.sidebar.menu.take().context("voyage unavailable")?;
        self.begin_access(&mut menu);
        if let Some(choice) = choice {
            menu.access_selected = choice;
            menu.editor = Some(MODES[choice].0);
        }
        self.sidebar.menu = Some(menu);
        Ok(())
    }
    pub(super) fn begin_access(&self, menu: &mut Menu) {
        menu.editor = Some(Action::Access);
        menu.scroll.set(0);
        menu.error.clear();
        let snapshot = self
            .views
            .get(&menu.target)
            .and_then(|v| v.snapshot.as_ref());
        menu.revision = snapshot.map(|s| s.revision);
        menu.access_selected = snapshot
            .and_then(|s| s.access.as_deref())
            .and_then(|mode| MODES.iter().position(|(_, value, _)| *value == mode))
            .unwrap_or(0);
        self.sidebar.visible.set(None);
    }
    pub(super) fn access_input(&mut self, menu: &mut Menu, event: &Event) -> Result<bool> {
        let choosing = menu.editor == Some(Action::Access);
        if let Event::Key(key) = event {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'q'))
            {
                self.quit = true;
                return Ok(true);
            }
            match key.code {
                KeyCode::Esc | KeyCode::Left => {
                    menu.editor = if choosing { None } else { Some(Action::Access) };
                    menu.error.clear();
                    menu.scroll.set(0);
                    self.sidebar.visible.set(None);
                    return Ok(false);
                }
                KeyCode::Up | KeyCode::BackTab if choosing => {
                    menu.follow_selection.set(true);
                    menu.access_selected = (menu.access_selected + 2) % 3
                }
                KeyCode::Down | KeyCode::Tab if choosing => {
                    menu.follow_selection.set(true);
                    menu.access_selected = (menu.access_selected + 1) % 3
                }
                KeyCode::Enter
                    if self.sidebar.visible.get()
                        == Some((menu.target, menu.incarnation, menu.editor)) =>
                {
                    if choosing {
                        menu.editor = Some(MODES[menu.access_selected].0);
                        menu.scroll.set(0);
                        self.sidebar.visible.set(None);
                    } else {
                        self.apply_access(menu)?;
                        return Ok(true);
                    }
                }
                _ => {}
            }
        } else if let Event::Mouse(mouse) = event
            && choosing
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self.sidebar.visible.get() == Some((menu.target, menu.incarnation, menu.editor))
        {
            let index = self
                .sidebar
                .menu_hits
                .borrow()
                .iter()
                .find(|(rect, target, inc, _)| {
                    *target == menu.target
                        && *inc == menu.incarnation
                        && rect.contains((mouse.column, mouse.row).into())
                })
                .map(|(_, _, _, index)| *index);
            if let Some(index) = index {
                menu.access_selected = index;
                menu.editor = Some(MODES[index].0);
                menu.scroll.set(0);
                self.sidebar.visible.set(None);
            }
        }
        Ok(false)
    }
    fn apply_access(&mut self, menu: &Menu) -> Result<()> {
        anyhow::ensure!(
            self.sidebar.scroll_bounds.get().0 == self.sidebar.scroll_bounds.get().1,
            "Read to the end of the access review before confirming"
        );
        if let Some(reason) = self.action_reason(menu, Action::Access) {
            anyhow::bail!(reason);
        }
        let view = self
            .views
            .get_mut(&menu.target)
            .context("voyage unavailable")?;
        // Preserve the reviewed revision. The owner atomically checks settings
        // changes, rather than rejecting ordinary conversation progress.
        let expected_revision = menu.revision.context("waiting for voyage state")?;
        let command_id = Uuid::new_v4();
        let access = MODES[menu.access_selected].1;
        let command = VoyageCommand::SetAccess {
            command_id,
            expected_revision,
            expires_at_ms: u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis(),
            )?
            .saturating_add(60_000),
            access: access.into(),
        };
        view.pending = Some(super::super::state::Pending {
            account_host: None,
            command_id,
            original: Some(Box::new(command.clone())),
            receipt_only: false,
            incarnation: menu.incarnation,
            draft: format!("/access {access}"),
            preserve_draft: true,
        });
        if let Err(error) = super::super::drafts::save(&self.clients[menu.target.route], view) {
            view.pending = None;
            return Err(error.context("cannot persist command identity; nothing sent"));
        }
        self.dispatch(menu.target, command_id, command);
        Ok(())
    }
    pub(super) fn draw_access(&self, frame: &mut Frame<'_>, menu: &Menu, body: Rect) {
        let current = self
            .views
            .get(&menu.target)
            .and_then(|v| v.snapshot.as_ref())
            .and_then(|s| s.access.as_deref());
        let label = current
            .and_then(|mode| MODES.iter().find(|(_, value, _)| *value == mode))
            .map_or("Unavailable", |(action, _, _)| action.label());
        let title = self.views.get(&menu.target).map_or_else(
            || "Unavailable voyage".into(),
            |view| super::super::safe(&view.title()),
        );
        let mut lines = super::super::presentation::wrap(
            Text::raw(format!("{title}\nCurrent access: {label}\n")),
            body.width,
        )
        .lines;
        if !menu.error.is_empty() {
            lines.extend(
                super::super::presentation::wrap(
                    Text::raw(format!("{}\n", super::super::safe(&menu.error))),
                    body.width,
                )
                .lines,
            );
        }
        let mut ranges = Vec::new();
        if menu.editor == Some(Action::Access) {
            for (index, (action, _, _)) in MODES.iter().enumerate() {
                let start = lines.len();
                lines.extend(
                    super::super::presentation::wrap(
                        Text::raw(format!(
                            "{}{}",
                            if index == menu.access_selected {
                                "> "
                            } else {
                                "  "
                            },
                            action.label()
                        )),
                        body.width,
                    )
                    .lines,
                );
                ranges.push((start, lines.len(), index));
            }
        } else {
            let (action, _, description) = MODES[menu.access_selected];
            let text = format!(
                "Change access to {}?\n\n{description}\n\nApplies to subsequent tool calls, including active local agents. Pending approvals are denied; retry those actions under the new mode. Already-started work and private terminals are not undone or stopped. Existing folder limits, blocked commands and machine policy still apply. Idle changes close retained terminals.",
                action.label()
            );
            lines.push(Line::default());
            lines.extend(super::super::presentation::wrap(Text::raw(text), body.width).lines);
        }
        let max = lines
            .len()
            .saturating_sub(body.height as usize)
            .min(u16::MAX as usize) as u16;
        let mut scroll = menu.scroll.get().min(max);
        if menu.editor == Some(Action::Access)
            && menu.follow_selection.replace(false)
            && let Some((start, _, _)) = ranges
                .iter()
                .find(|(_, _, index)| *index == menu.access_selected)
        {
            if *start < scroll as usize {
                scroll = *start as u16;
            } else if *start >= scroll as usize + body.height as usize {
                scroll = start
                    .saturating_sub(body.height.saturating_sub(1) as usize)
                    .min(max as usize) as u16;
            }
        }
        menu.scroll.set(scroll);
        self.sidebar.scroll_bounds.set((scroll, max));
        let enabled = self.action_reason(menu, Action::Access).is_none();
        for (start, end, index) in ranges {
            let top = start.max(scroll as usize);
            let bottom = end.min(scroll as usize + body.height as usize);
            if top < bottom {
                let rect = Rect::new(
                    body.x,
                    body.y + (top - scroll as usize) as u16,
                    body.width,
                    (bottom - top) as u16,
                );
                let style = super::super::right_panel::control_style(
                    self.sidebar.pointer,
                    rect,
                    index == menu.access_selected,
                    enabled,
                );
                for line in &mut lines[top..bottom] {
                    *line = line.clone().style(style);
                }
                if enabled {
                    self.sidebar.menu_hits.borrow_mut().push((
                        rect,
                        menu.target,
                        menu.incarnation,
                        index,
                    ));
                }
            }
        }
        frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), body);
    }
}
