//! Access is edited on the owner, never reconstructed from public configuration.
use super::*;
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    style::{Color, Style},
    widgets::Paragraph,
};
use voyage_protocol::process::RuntimeCommand;

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
                    self.sidebar.visible.set(None);
                    return Ok(false);
                }
                KeyCode::Up if choosing => menu.access_selected = (menu.access_selected + 2) % 3,
                KeyCode::Down if choosing => menu.access_selected = (menu.access_selected + 1) % 3,
                KeyCode::Enter
                    if self.sidebar.visible.get()
                        == Some((menu.target, menu.incarnation, menu.editor)) =>
                {
                    if choosing {
                        menu.editor = Some(MODES[menu.access_selected].0);
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
                self.sidebar.visible.set(None);
            }
        }
        Ok(false)
    }
    fn apply_access(&mut self, menu: &Menu) -> Result<()> {
        if let Some(reason) = self.action_reason(menu, Action::Access) {
            anyhow::bail!(reason);
        }
        let view = self
            .views
            .get_mut(&menu.target)
            .context("voyage unavailable")?;
        let snapshot = view.snapshot.as_ref().context("waiting for voyage state")?;
        ensure!(
            Some(snapshot.revision) == menu.revision,
            "Voyage changed; reopen Access to review its current state"
        );
        let command_id = Uuid::new_v4();
        let access = MODES[menu.access_selected].1;
        let command = RuntimeCommand::SetAccess {
            command_id,
            expected_revision: snapshot.revision,
            expires_at_ms: u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis(),
            )?
            .saturating_add(60_000),
            access: access.into(),
        };
        view.pending = Some(super::super::state::Pending {
            command_id,
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
        frame.render_widget(
            Paragraph::new(format!("Current access: {label}")),
            Rect::new(body.x, body.y, body.width, 1),
        );
        if menu.editor == Some(Action::Access) {
            if body.y.saturating_add(2 + menu.access_selected as u16) >= body.bottom() {
                self.sidebar.visible.set(None);
            }
            for (index, (action, _, _)) in MODES.iter().enumerate() {
                let y = body.y.saturating_add(2 + index as u16);
                if y >= body.bottom() {
                    break;
                }
                let row = Rect::new(body.x, y, body.width, 1);
                let selected = index == menu.access_selected;
                frame.render_widget(
                    Paragraph::new(format!(
                        "{}{}",
                        if selected { "> " } else { "  " },
                        action.label()
                    ))
                    .style(Style::default().fg(if selected {
                        Color::Cyan
                    } else {
                        Color::Reset
                    })),
                    row,
                );
                self.sidebar.menu_hits.borrow_mut().push((
                    row,
                    menu.target,
                    menu.incarnation,
                    index,
                ));
            }
        } else {
            let (action, _, description) = MODES[menu.access_selected];
            let text = format!(
                "Change access to {}?\n\n{description}\n\nExisting folder limits, blocked commands and machine policy still apply. Retained terminals close before the change.",
                action.label()
            );
            let area = Rect::new(
                body.x,
                body.y.saturating_add(2),
                body.width,
                body.height.saturating_sub(2),
            );
            let wrapped =
                super::super::presentation::wrap(ratatui::text::Text::raw(text), area.width);
            if wrapped.lines.len() > usize::from(area.height) {
                self.sidebar.visible.set(None);
                frame.render_widget(
                    Paragraph::new("Enlarge the terminal to review this access change."),
                    area,
                );
            } else {
                frame.render_widget(Paragraph::new(wrapped), area);
            }
        }
    }
}
