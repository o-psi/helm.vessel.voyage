use super::*;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Paragraph},
};

impl App {
    pub(in crate::process_client::ui) fn draw_new_draft(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(draft) = self.active_draft.and_then(|id| self.new_drafts.get(&id)) else {
            return;
        };
        let area = area.inner(ratatui::layout::Margin::new(2, 1));
        let rows = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(2),
            Constraint::Length(7),
            Constraint::Length(4),
        ])
        .split(area);
        let state = if draft.busy {
            "Starting / checking…"
        } else if draft.saved.attempted {
            "First turn pending · checking receipts, never replaying"
        } else if draft.saved.process.is_some() {
            "Voyage created · Enter sends the saved first message"
        } else if draft.saved.start_attempted {
            "Creation outcome unknown · observing, never replaying"
        } else if draft.saved.start.is_some() {
            "Not started · Enter retries the saved launch preflight"
        } else {
            "Draft · starts when you send"
        };
        let model = safe(
            draft
                .saved
                .config
                .as_ref()
                .map_or("Executing-host model", |c| c.model.as_str()),
        );
        frame.render_widget(
            Paragraph::new(format!(
                "New voyage · {}\n{state}\n{} · {model}",
                safe(&self.route_label(draft.route)),
                safe(&draft.saved.workspace.display().to_string())
            ))
            .block(Block::default().borders(Borders::BOTTOM)),
            rows[0],
        );
        let guidance = if self.clients[draft.route].is_local() {
            "Describe what you want to do.\n\nThis draft is saved on this computer.\n/model · /thinking · /service · /access MODE · /workspace PATH · /help\n\nTab switches drafts and voyages. Ctrl+N opens a blank draft."
        } else {
            "Describe what you want the remote host to do.\n\nOnly the draft is saved here. Workspace authority, model settings, policy and provider credentials remain on the executing host.\n/workspace PATH must name an authorized workspace. Ctrl+N opens the workspace picker; Esc cancels without sending."
        };
        frame.render_widget(
            Paragraph::new(guidance).wrap(ratatui::widgets::Wrap { trim: false }),
            rows[1],
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" First message · {} ", self.attachment_summary()));
        let inner = block.inner(rows[2]);
        let body = Rect {
            height: inner.height.saturating_sub(1),
            ..inner
        };
        frame.render_widget(block, rows[2]);
        self.draw_inference_controls(
            frame,
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        );
        let (row, col) = composer::cursor_position(
            &safe(&draft.composer.text[..draft.composer.cursor]),
            body.width,
        );
        let scroll = row.saturating_sub(body.height.saturating_sub(1));
        frame.render_widget(
            Paragraph::new(presentation::wrap(
                Text::raw(safe(&draft.composer.text)),
                body.width,
            ))
            .scroll((scroll, 0)),
            body,
        );
        if body.height > 0 && body.width > 0 {
            frame.set_cursor_position((
                body.x + col.min(body.width - 1),
                body.y + row.saturating_sub(scroll).min(body.height - 1),
            ));
        }
        frame.render_widget(
            Paragraph::new(format!(
                "{}\nEnter Send · Alt+Enter New line · Ctrl+C Leave",
                safe(&self.status)
            ))
            .style(Style::default().fg(Color::Cyan))
            .wrap(ratatui::widgets::Wrap { trim: false }),
            rows[3],
        );
    }

    pub(in crate::process_client::ui) fn draw_draft_links(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
    ) {
        let offset = self
            .active_draft
            .and_then(|id| self.new_drafts.keys().position(|key| *key == id))
            .unwrap_or(0)
            .saturating_sub(area.height.saturating_sub(1) as usize);
        for (y, (id, draft)) in (area.y..).zip(self.new_drafts.iter().skip(offset)) {
            if y >= area.bottom() {
                break;
            }
            let row = Rect::new(area.x, y, area.width, 1);
            let title = draft
                .composer
                .text
                .lines()
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("New voyage");
            frame.render_widget(
                Paragraph::new(format!(
                    "{}Draft: {}",
                    if self.active_draft == Some(*id) {
                        "> "
                    } else {
                        "  "
                    },
                    safe(title)
                ))
                .style(Style::default().fg(Color::Cyan)),
                row,
            );
            self.draft_hits.borrow_mut().push((row, *id));
        }
    }
}
