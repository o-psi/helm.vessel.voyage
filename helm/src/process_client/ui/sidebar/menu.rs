use super::*;
use ratatui::{
    Frame,
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};

impl App {
    pub(in crate::process_client::ui) fn draw_actions(&self, frame: &mut Frame<'_>) {
        self.sidebar.menu_hits.borrow_mut().clear();
        let Some(menu) = &self.sidebar.menu else {
            return;
        };
        self.sidebar
            .visible
            .set(Some((menu.target, menu.incarnation, menu.editor)));
        let screen = frame.area();
        let width = screen.width.saturating_sub(4).min(76);
        let height = screen.height.saturating_sub(2).min(22);
        let area = Rect::new(
            (screen.width - width) / 2,
            (screen.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Voyage actions ")
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let view = self.views.get(&menu.target);
        let title = view.map_or_else(
            || "Unavailable voyage".into(),
            |v| super::super::safe(&v.title()),
        );
        frame.render_widget(
            Paragraph::new(title).style(Style::default().add_modifier(Modifier::BOLD)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let body = Rect::new(
            inner.x,
            inner.y.saturating_add(2),
            inner.width,
            inner.height.saturating_sub(5),
        );
        if let Some(action) = menu.editor {
            let text=match action {
                Action::Details => view.map_or_else(|| "Voyage no longer available".into(), |v| {
                    let run = v.snapshot.as_ref().map_or("Unavailable", |s| s.run.as_ref().map_or("Idle", |r| super::super::presentation::run_state(&r.state)));
                    let cleanup = if v.process.state == voyage_protocol::process::ProcessState::Stopped { "Confirmed stopped" } else { v.snapshot.as_ref().map_or("Unknown", |s| if s.pending_cleanup_run.is_some() { "Unconfirmed run cleanup" } else { "No pending run cleanup" }) };
                    format!("Vessel: {}\nWorkspace: {}\nModel: {}\nProcess: {:?}\nRun: {run}\nArchived: {}\nCleanup: {cleanup}\nPending command: {}\nVoyage: {}\n{}", self.route_label(menu.target.route), v.process.workspace.display(), v.snapshot.as_ref().map_or("Unavailable", |s| s.model.as_str()), v.process.state, v.archived(), v.pending.is_some(), menu.target.session, v.error.as_deref().map(super::super::presentation::notice).unwrap_or_default())
                }),
                Action::Rename=>"Rename voyage\nEnter a new name:".into(),
                Action::Branch=>"Branch conversation\nOptional name for the new voyage:".into(),
                Action::Delete=>format!("Delete permanently\nThis removes the selected voyage's conversation history.\nVoyage: {}\nType DELETE to confirm:",menu.target.session),
                _=>String::new(),
            };
            let mut text =
                super::super::presentation::wrap(Text::raw(super::super::safe(&text)), body.width);
            if action != Action::Details {
                text.lines.push(Line::default());
                let cursor_row = text.lines.len() as u16;
                text.lines.extend(
                    super::super::presentation::wrap(Text::raw(&menu.text.text), body.width).lines,
                );
                let (row, col) = crate::composer::cursor_position(
                    &menu.text.text[..menu.text.cursor],
                    body.width,
                );
                if cursor_row + row < body.height {
                    frame.set_cursor_position((body.x + col, body.y + cursor_row + row));
                }
            }
            let max_scroll = text
                .lines
                .len()
                .saturating_sub(body.height as usize)
                .min(u16::MAX as usize) as u16;
            frame.render_widget(
                Paragraph::new(text).scroll((menu.scroll.min(max_scroll), 0)),
                body,
            );
        } else {
            let mut y = body.y;
            for (index, action) in menu.actions.iter().enumerate() {
                if *action == Action::Delete {
                    y = y.saturating_add(1);
                }
                if y >= body.bottom() {
                    break;
                }
                let reason = self.action_reason(menu, *action);
                let line = if let Some(reason) = reason {
                    format!(
                        "{}{} — {reason}",
                        if index == menu.selected { "> " } else { "  " },
                        action.label()
                    )
                } else {
                    format!(
                        "{}{}",
                        if index == menu.selected { "> " } else { "  " },
                        action.label()
                    )
                };
                let row = Rect::new(body.x, y, body.width, 1);
                let color = if reason.is_some() {
                    Color::DarkGray
                } else if *action == Action::Delete {
                    Color::Red
                } else if index == menu.selected {
                    Color::Cyan
                } else {
                    Color::Reset
                };
                frame.render_widget(Paragraph::new(line).style(Style::default().fg(color)), row);
                self.sidebar.menu_hits.borrow_mut().push((
                    row,
                    menu.target,
                    menu.incarnation,
                    index,
                ));
                y = y.saturating_add(1);
            }
        }
        let hint = if menu.error.is_empty() {
            if menu.editor == Some(Action::Details) {
                "↑↓ scroll · Enter close · Esc back"
            } else if menu.editor.is_some() {
                "Enter confirm · Esc back · Draft preserved"
            } else {
                "↑↓ select · Enter choose · ← / Esc close"
            }
        } else {
            &menu.error
        };
        frame.render_widget(
            Paragraph::new(super::super::presentation::wrap(
                Text::raw(super::super::safe(hint)),
                inner.width,
            ))
            .style(Style::default().fg(if menu.error.is_empty() {
                Color::DarkGray
            } else {
                Color::Yellow
            })),
            Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 2),
        );
    }
}
