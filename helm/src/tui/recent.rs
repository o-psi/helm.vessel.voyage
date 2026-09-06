//! Responsive recent-conversation navigation with exclusive modal input ownership.
use super::{App, commands::request_navigation, text::display_safe};
use crate::session::SessionStore;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};

pub(super) fn sidebar_area(area: Rect, app: &App) -> Option<Rect> {
    let landscape = app
        .display_pixels
        .map_or(area.width > area.height.saturating_mul(2), |(w, h)| w > h);
    (landscape && area.width >= 64 && area.height >= 10)
        .then(|| Rect::new(area.x, area.y, (area.width / 4).clamp(24, 36), area.height))
}
pub(super) fn content_area(area: Rect, app: &App) -> Rect {
    sidebar_area(area, app).map_or(area, |side| {
        Rect::new(
            area.x + side.width,
            area.y,
            area.width - side.width,
            area.height,
        )
    })
}
pub(super) fn drawer_area(area: Rect) -> Rect {
    Rect::new(area.x, area.y, area.width.min(48), area.height)
}
fn offset(area: Rect, app: &App) -> usize {
    let rows = area.height.saturating_sub(4).max(1) as usize;
    app.selected_session.saturating_sub(rows - 1)
}
pub(super) fn draw(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Recent · Ctrl+S ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.show_sessions {
            Color::Cyan
        } else {
            Color::DarkGray
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = vec![
        Line::from(if app.show_sessions {
            "↑↓ choose · Enter open · Esc"
        } else {
            "Click to open · Ctrl+N new"
        })
        .style(Style::default().fg(Color::DarkGray)),
    ];
    for (index, saved) in app
        .sessions
        .iter()
        .enumerate()
        .skip(offset(area, app))
        .take(area.height.saturating_sub(4) as usize)
    {
        let session = if saved.id == app.session.id {
            &app.session
        } else {
            saved
        };
        let active = session.id == app.session.id;
        let marker = if active { "●" } else { " " };
        let selected = index == app.selected_session && app.show_sessions;
        lines.push(
            Line::from(format!(
                "{marker} {}",
                display_safe(&session.display_name()).replace(['\n', '\r'], " ")
            ))
            .style(if selected {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else if active {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            }),
        );
    }
    if app.sessions.is_empty() {
        lines.push(Line::from("No saved conversations"));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
pub(super) async fn open_selected(app: &mut App, store: &SessionStore) -> anyhow::Result<()> {
    if app.is_running() {
        app.status = "Finish or cancel active work before switching conversations".into();
        return Ok(());
    }
    if let Some(session) = app.sessions.get(app.selected_session) {
        if session.id == app.session.id {
            app.show_sessions = false;
            return Ok(());
        }
        let reference = session.id.to_string();
        if let Err(error) = request_navigation(app, store, reference).await {
            app.status = format!(
                "Cannot switch conversations; draft retained: {}",
                display_safe(&error.to_string())
            );
        }
    }
    Ok(())
}
pub(super) fn mouse(mouse: MouseEvent, app: &mut App) -> bool {
    if app.question.is_some()
        || app.approval.is_some()
        || app.terminal_panel.attached_terminal.is_some()
        || app.terminal_panel.terminal_picker
        || app.model_panel.model_picker
        || app.shortcut_help
        || app.policy_panel.open
        || app.workflow_panel.is_open()
        || app.voyage_panel.is_open()
        || app.supervisor_panel.supervisor_mode.is_some()
        || app.todo_panel.todo_mode.is_some()
    {
        return false;
    }
    let area = sidebar_area(app.display_area, app)
        .or_else(|| app.show_sessions.then(|| drawer_area(app.display_area)));
    let Some(area) = area else {
        return false;
    };
    if !area.contains((mouse.column, mouse.row).into()) {
        return false;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            app.show_sessions = true;
            app.selected_session = app.selected_session.saturating_sub(3);
        }
        MouseEventKind::ScrollDown => {
            app.show_sessions = true;
            app.selected_session =
                (app.selected_session + 3).min(app.sessions.len().saturating_sub(1));
        }
        MouseEventKind::Down(MouseButton::Left)
            if mouse.row >= area.y + 2 && mouse.row < area.bottom().saturating_sub(2) =>
        {
            let index = offset(area, app) + usize::from(mouse.row - area.y - 2);
            if index < app.sessions.len() {
                app.selected_session = index;
                app.show_sessions = true;
                app.open_recent = true;
            }
        }
        _ => {}
    }
    true
}
