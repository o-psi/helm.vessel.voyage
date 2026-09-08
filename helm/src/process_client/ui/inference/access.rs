//! Composer access entry points; live changes retain the owner's confirmation flow.
use super::*;
use crossterm::event::{KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

const MODES: [crate::config::AccessMode; 3] = [
    crate::config::AccessMode::ReadOnly,
    crate::config::AccessMode::Approval,
    crate::config::AccessMode::Unrestricted,
];

#[derive(Default)]
pub(super) struct AccessControls {
    pub(super) hit: std::cell::Cell<Option<(Rect, Destination)>>,
    pub(super) draft: Option<(Uuid, usize)>,
    pub(super) visible: std::cell::Cell<bool>,
}

impl App {
    pub(super) fn draw_access_control(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        destination: Destination,
    ) {
        let value = match destination {
            Destination::Live(target) => self
                .views
                .get(&target)
                .and_then(|v| v.snapshot.as_ref())
                .and_then(|s| s.access.clone()),
            Destination::Draft(id) => self.draft_access(id).ok(),
        };
        let style = Style::default().fg(if value.is_some() {
            Color::Cyan
        } else {
            Color::DarkGray
        });
        frame.render_widget(
            Paragraph::new(format!(
                "Access: {} ▾",
                safe(value.as_deref().unwrap_or("unavailable"))
            ))
            .style(style.patch(self.hover_style(area, true))),
            area,
        );
        self.inference.access.hit.set(Some((area, destination)));
    }

    pub(super) fn draw_draft_access(&self, frame: &mut Frame<'_>) {
        self.inference.access.visible.set(false);
        let Some((_, selected)) = self.inference.access.draft else {
            return;
        };
        let screen = frame.area();
        if screen.width < 40 || screen.height < 10 {
            return;
        }
        let area = Rect::new(
            screen.x + (screen.width - 40) / 2,
            screen.y + (screen.height - 9) / 2,
            40,
            9,
        );
        frame.render_widget(Clear, area);
        let text = format!(
            "Choose access for this draft\n\n{}\n\n↑↓ select · Enter save · Esc cancel",
            MODES
                .iter()
                .enumerate()
                .map(|(i, mode)| format!("{} {mode}", if i == selected { ">" } else { " " }))
                .collect::<Vec<_>>()
                .join("\n")
        );
        frame.render_widget(
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" Access ")),
            area,
        );
        self.inference.access.visible.set(true);
    }

    pub(super) fn composer_access_input(&mut self, event: &Event) -> Result<bool> {
        if let Event::Resize(..) = event {
            self.inference.access.hit.set(None);
            self.inference.access.visible.set(false);
        }
        if let Some((id, selected)) = self.inference.access.draft {
            if let Event::Key(key) = event
                && key.kind != KeyEventKind::Release
            {
                match key.code {
                    KeyCode::Esc => self.inference.access.draft = None,
                    KeyCode::Char('c' | 'q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.quit = true
                    }
                    KeyCode::Up => self.inference.access.draft = Some((id, (selected + 2) % 3)),
                    KeyCode::Down => self.inference.access.draft = Some((id, (selected + 1) % 3)),
                    KeyCode::Enter
                        if self.inference.access.visible.get()
                            && self.inference_destination() == Some(Destination::Draft(id)) =>
                    {
                        self.set_draft_access(id, MODES[selected])?;
                        self.inference.access.draft = None;
                    }
                    _ => {}
                }
            }
            return Ok(true);
        }
        if self.inference.picker.is_some()
            || self.help
            || self.explore.is_some()
            || self.sidebar.menu.is_some()
            || self.interactions.borrow().focused
        {
            return Ok(false);
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some((area, destination)) = self.inference.access.hit.get()
            && Some(destination) == self.inference_destination()
            && area.contains((mouse.column, mouse.row).into())
        {
            match destination {
                Destination::Live(target) => self.open_access(target, None)?,
                Destination::Draft(id) => {
                    let current = self.draft_access(id)?;
                    let selected = MODES
                        .iter()
                        .position(|mode| mode.to_string() == current)
                        .unwrap_or(0);
                    self.inference.access.draft = Some((id, selected));
                    self.inference.access.visible.set(false);
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}
