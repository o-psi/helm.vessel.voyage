//! Responsive recent-conversation navigation with exclusive modal input ownership.
use super::{App, commands::request_cli, text::display_safe};
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
        let arguments = vec!["chat".into(), "--resume".into(), session.id.to_string()];
        if let Err(error) = request_cli(app, store, arguments, true, false).await {
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
        || app.workflow_panel.is_open()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    fn app() -> App {
        App::new(Session::new("/tmp".into(), "test".into()), vec![])
    }
    #[test]
    fn orientation_uses_pixels_with_cell_fallback_and_minimum_workspace() {
        let mut app = app();
        assert!(sidebar_area(Rect::new(0, 0, 100, 40), &app).is_some());
        assert!(sidebar_area(Rect::new(0, 0, 80, 40), &app).is_none());
        assert!(sidebar_area(Rect::new(0, 0, 60, 20), &app).is_none());
        app.display_pixels = Some((900, 1200));
        assert!(sidebar_area(Rect::new(0, 0, 100, 40), &app).is_none());
        app.display_pixels = Some((1200, 900));
        assert!(sidebar_area(Rect::new(0, 0, 80, 40), &app).is_some());
        let area = Rect::new(0, 0, 100, 30);
        let sidebar = sidebar_area(area, &app).unwrap();
        let content = content_area(area, &app);
        assert_eq!(content.x, sidebar.right());
        assert_eq!(content.right(), area.right());
    }
    #[test]
    fn selected_recent_remains_visible_and_titles_are_safe() {
        let mut app = app();
        app.sessions = (0..50)
            .map(|i| {
                let mut s = Session::new("/tmp".into(), "test".into());
                s.set_name(format!("Conversation {i}"));
                s
            })
            .collect();
        app.sessions[49].set_name("LAST\u{1b}]2;hostile\u{7}\nline".into());
        app.selected_session = 49;
        app.show_sessions = true;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 12)).unwrap();
        terminal
            .draw(|frame| draw(frame, frame.area(), &app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("LAST"));
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains("Conversation 0"));
    }
    #[test]
    fn sidebar_mouse_does_not_scroll_transcript_or_intercept_modals() {
        let mut app = app();
        app.display_area = Rect::new(0, 0, 100, 30);
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 4,
            row: 2,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert!(mouse(click, &mut app));
        assert!(app.open_recent);
        app.open_recent = false;
        app.model_panel.model_picker = true;
        assert!(!mouse(click, &mut app));
        assert!(!app.open_recent);
        app.model_panel.model_picker = false;
        app.terminal_panel.attached_terminal =
            Some(crate::terminal::TerminalId(uuid::Uuid::new_v4()));
        assert!(!mouse(click, &mut app));
    }
    #[tokio::test]
    async fn switching_saves_draft_and_relaunches_with_existing_authority_path() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let mut app = app();
        let id = app.session.id;
        let destination = Session::new(directory.path().into(), "other".into());
        let target = destination.id;
        app.sessions.push(destination);
        app.selected_session = 1;
        app.composer.insert_str("unsent\n日本語");
        open_selected(&mut app, &store).await.unwrap();
        let saved = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap();
        assert_eq!(saved.draft, "unsent\n日本語");
        assert!(saved.messages.is_empty());
        assert_eq!(App::new(saved, vec![]).composer.text, "unsent\n日本語");
        let Some(super::super::TuiExit::Launch(request)) = app.exit else {
            panic!("missing safe handoff")
        };
        assert_eq!(request.arguments, ["chat", "--resume", &target.to_string()]);
        assert!(request.use_active_config);
    }
    #[tokio::test]
    async fn selecting_current_session_keeps_draft_without_exit() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let mut app = app();
        app.composer.insert_str("draft");
        app.show_sessions = true;
        open_selected(&mut app, &store).await.unwrap();
        assert!(!app.show_sessions);
        assert!(!app.quit);
        assert_eq!(app.composer.text, "draft");
    }
    #[tokio::test]
    async fn running_and_save_failure_leave_current_conversation_and_draft_intact() {
        let directory = tempfile::tempdir().unwrap();
        let blocked = directory.path().join("file");
        std::fs::write(&blocked, "not a directory").unwrap();
        let store = SessionStore::new(blocked.join("sessions"));
        let mut app = app();
        app.sessions
            .push(Session::new("/tmp".into(), "other".into()));
        app.selected_session = 1;
        app.composer.insert_str("keep me");
        let (steering, _receiver) = crate::agent::steering_channel(4);
        app.running = Some(super::super::Running {
            task: tokio::spawn(std::future::pending()),
            cancel: tokio_util::sync::CancellationToken::new(),
            steering,
        });
        open_selected(&mut app, &store).await.unwrap();
        assert!(app.status.contains("Finish or cancel"));
        assert!(!app.quit);
        app.running = None;
        open_selected(&mut app, &store).await.unwrap();
        assert!(app.status.contains("Cannot switch"));
        assert_eq!(app.composer.text, "keep me");
        assert!(!app.quit);
        assert!(app.exit.is_none());
    }
    #[tokio::test]
    async fn new_conversation_preserves_old_draft_without_copying_it() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = SessionStore::new(directory.path().join("sessions"));
        let mut app = app();
        let id = app.session.id;
        app.composer.insert_str("old draft");
        super::super::commands::start_new_session(&mut app, &mut store, Some("New"))
            .await
            .unwrap();
        assert_ne!(app.session.id, id);
        assert!(app.composer.text.is_empty());
        assert!(app.session.draft.is_empty());
        let saved = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap();
        assert_eq!(saved.draft, "old draft");
    }
    #[test]
    fn old_sessions_default_to_no_draft() {
        let mut value = serde_json::to_value(app().session).unwrap();
        value.as_object_mut().unwrap().remove("draft");
        assert!(
            serde_json::from_value::<Session>(value)
                .unwrap()
                .draft
                .is_empty()
        );
    }
}
