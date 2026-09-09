//! Shared right-panel chrome and passive, geometry-based control presentation.
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::Style,
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
};

pub(super) fn block(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .padding(Padding::horizontal(1))
        .border_style(crate::theme::Role::Focus.style())
        .title(format!(" {} ", title.into()))
}

pub(super) fn control_style(
    pointer: Option<Position>,
    rect: Rect,
    selected: bool,
    enabled: bool,
) -> Style {
    if !enabled {
        return crate::theme::Role::Muted.style();
    }
    let style = if selected {
        crate::theme::Role::Selection.style()
    } else {
        crate::theme::Role::Focus.style()
    };
    if pointer.is_some_and(|point| rect.contains(point)) {
        style.patch(crate::theme::Role::Hover.style())
    } else {
        style
    }
}

/// Wrap complete buttons, never publish a partially clipped click target.
pub(super) fn buttons<T: Copy>(
    frame: &mut Frame<'_>,
    area: Rect,
    pointer: Option<Position>,
    controls: &[(&str, T, bool)],
) -> Vec<(Rect, T)> {
    let mut hits = Vec::new();
    let (mut x, mut y) = (area.x, area.y);
    for (label, control, enabled) in controls {
        let width = unicode_width::UnicodeWidthStr::width(*label) as u16;
        if x.saturating_add(width) > area.right() {
            x = area.x;
            y = y.saturating_add(1);
        }
        if width > area.width || y >= area.bottom() {
            continue;
        }
        let rect = Rect::new(x, y, width, 1);
        frame.render_widget(
            Paragraph::new(*label).style(control_style(pointer, rect, false, *enabled)),
            rect,
        );
        if *enabled {
            hits.push((rect, *control));
        }
        x = x.saturating_add(width + 1);
    }
    hits
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Editor {
    Action(super::state::Target, uuid::Uuid, super::sidebar::Action),
    Question(super::state::Target, uuid::Uuid),
}
impl Editor {
    pub(super) fn target(self) -> super::state::Target {
        match self {
            Self::Action(target, ..) | Self::Question(target, ..) => target,
        }
    }
}
impl super::App {
    pub(super) fn panel_editor(&self) -> Option<(Editor, String)> {
        if self.vessels_open()
            || self.help
            || self.explore.is_some()
            || self.workspace_picker.is_some()
        {
            return None;
        }
        self.action_editor().or_else(|| self.question_editor())
    }
    pub(super) fn apply_panel_text(
        &mut self,
        editor: Editor,
        before: &str,
        text: String,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.panel_editor()
                .as_ref()
                .is_some_and(|(current, value)| *current == editor && value == before),
            "Panel changed while reading clipboard; paste again"
        );
        match editor {
            Editor::Action(..) => {
                let text = super::safe(&text).replace(['\n', '\r'], " ");
                anyhow::ensure!(
                    before.len() + text.len() <= 256,
                    "Input limit is 256 bytes; field preserved"
                );
                self.sidebar_input(&crossterm::event::Event::Paste(text))?;
            }
            Editor::Question(..) => {
                let text: String = super::safe(&text)
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                anyhow::ensure!(
                    before.len() + text.len() <= 4096,
                    "Answer is too long; field preserved"
                );
                self.interaction_input(&crossterm::event::Event::Paste(text))?;
            }
        }
        Ok(())
    }
}
