//! Full-screen terminal frontend. This module intentionally depends only on the
//! agent's event and approval contracts, so providers can add finer-grained
//! streaming without changing the UI.

use std::{io, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use tokio::sync::{mpsc, oneshot};

use crate::{
    Agent, AgentEvent, EventSink,
    model::Role,
    session::{Session, SessionStore, compact_messages},
    tools::{ApprovalOutcome, ApprovalRequest as ToolApprovalRequest, Approver},
};

#[derive(Debug)]
#[doc(hidden)]
pub enum UiEvent {
    Agent(AgentEvent),
    Approval(ApprovalRequest),
    Finished(Result<crate::AgentOutcome, String>),
}

#[derive(Debug)]
#[doc(hidden)]
pub struct ApprovalRequest {
    reason: String,
    response: oneshot::Sender<ApprovalOutcome>,
}

#[derive(Clone)]
pub struct UiBridge {
    tx: mpsc::UnboundedSender<UiEvent>,
}

#[async_trait]
impl EventSink for UiBridge {
    async fn emit(&self, event: AgentEvent) {
        let _ = self.tx.send(UiEvent::Agent(event));
    }
}

#[async_trait]
impl Approver for UiBridge {
    async fn approve(&self, request: &ToolApprovalRequest) -> ApprovalOutcome {
        let (response, receive) = oneshot::channel();
        if self
            .tx
            .send(UiEvent::Approval(ApprovalRequest {
                reason: request.reason.clone(),
                response,
            }))
            .is_err()
        {
            return ApprovalOutcome::Unavailable;
        }
        let outcome = receive.await.unwrap_or(ApprovalOutcome::Unavailable);
        tracing::info!(approval_id = %request.id, execution_id = %request.execution_id,
            action = %request.action, target = %request.target, outcome = ?outcome,
            "approval decided");
        outcome
    }
}

impl UiBridge {
    #[doc(hidden)]
    pub fn sender(&self) -> mpsc::UnboundedSender<UiEvent> {
        self.tx.clone()
    }
}

/// The pair passed into an Agent and [`run`] respectively.
pub fn bridge() -> (Arc<UiBridge>, mpsc::UnboundedReceiver<UiEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (Arc::new(UiBridge { tx }), rx)
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

#[derive(Default)]
struct Composer {
    text: String,
    cursor: usize,
}

impl Composer {
    fn insert(&mut self, character: char) {
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.text.drain(previous..self.cursor);
        self.cursor = previous;
    }

    fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
}

struct App {
    session: Session,
    sessions: Vec<Session>,
    composer: Composer,
    activity: Vec<String>,
    streaming_activity: Option<usize>,
    status: String,
    scroll: u16,
    running: Option<Running>,
    approval: Option<ApprovalRequest>,
    show_sessions: bool,
    selected_session: usize,
    quit: bool,
}

struct Running {
    task: tokio::task::JoinHandle<()>,
    cancel: tokio_util::sync::CancellationToken,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

impl App {
    fn new(session: Session, sessions: Vec<Session>) -> Self {
        Self {
            session,
            sessions,
            composer: Composer::default(),
            activity: Vec::new(),
            streaming_activity: None,
            status: "Ready".into(),
            scroll: 0,
            running: None,
            approval: None,
            show_sessions: false,
            selected_session: 0,
            quit: false,
        }
    }

    fn is_running(&self) -> bool {
        self.running.is_some()
    }

    fn cancel(&mut self) {
        if let Some(running) = &self.running {
            running.cancel.cancel();
            self.status = "Cancelling; partial output will not be committed".into();
        }
        if let Some(approval) = self.approval.take() {
            let _ = approval.response.send(ApprovalOutcome::Denied);
        }
    }
}

pub async fn run(
    agent: Arc<Agent>,
    store: SessionStore,
    session: Session,
    mut rx: mpsc::UnboundedReceiver<UiEvent>,
    tx: mpsc::UnboundedSender<UiEvent>,
) -> Result<()> {
    let sessions = store.list().await?;
    let mut app = App::new(session, sessions);
    let _guard = TerminalGuard::enter().context("failed to initialize terminal")?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let mut input = EventStream::new();

    while !app.quit {
        terminal.draw(|frame| draw(frame, &app))?;
        tokio::select! {
            event = input.next() => {
                match event {
                    Some(Ok(Event::Key(key))) if key.is_press() => {
                        handle_key(key, &mut app, &agent, &store, &tx).await?;
                    }
                    Some(Ok(Event::Resize(_, _))) => {}
                    Some(Err(error)) => return Err(error.into()),
                    None => break,
                    _ => {}
                }
            }
            event = rx.recv() => {
                let Some(event) = event else { break };
                handle_ui_event(event, &mut app, &store).await?;
            }
        }
    }
    app.cancel();
    store.save(&mut app.session).await?;
    terminal.show_cursor()?;
    Ok(())
}

async fn handle_ui_event(event: UiEvent, app: &mut App, store: &SessionStore) -> Result<()> {
    match event {
        UiEvent::Agent(AgentEvent::Thinking { turn }) => {
            app.status = format!("Model turn {turn}…  Esc cancels");
        }
        UiEvent::Agent(AgentEvent::AssistantTextDelta(text)) => {
            let index = *app.streaming_activity.get_or_insert_with(|| {
                app.activity.push("assistant: ".into());
                app.activity.len() - 1
            });
            app.activity[index].push_str(&text);
            app.status = "Receiving response…  Esc cancels".into();
        }
        UiEvent::Agent(AgentEvent::AssistantText(text)) => {
            if app.streaming_activity.take().is_none() {
                app.activity
                    .push(format!("assistant: {}", one_line(&text, 240)));
            }
            app.status = "Receiving response…  Esc cancels".into();
        }
        UiEvent::Agent(AgentEvent::ToolStarted { name, arguments }) => {
            app.streaming_activity = None;
            app.activity.push(format!("▶ {name} {arguments}"));
            app.status = format!("Running {name}…  Esc cancels");
        }
        UiEvent::Agent(AgentEvent::ToolFinished {
            name,
            result,
            success,
        }) => {
            app.activity.push(format!(
                "{} {name}: {}",
                if success { "✓" } else { "✗" },
                one_line(&result, 160)
            ));
        }
        UiEvent::Agent(AgentEvent::ProviderRetry {
            attempt,
            delay,
            error,
        }) => {
            app.activity.push(format!(
                "↻ provider retry {attempt} in {:.1}s: {}",
                delay.as_secs_f32(),
                one_line(&error, 160)
            ));
            app.status = format!("Provider retry {attempt}…  Esc cancels");
        }
        UiEvent::Agent(AgentEvent::Cancelled) => {
            app.activity.push("■ operation cancelled".into());
            app.status = "Cancelled".into();
        }
        UiEvent::Approval(request) => {
            app.status = "Approval required".into();
            app.approval = Some(request);
        }
        UiEvent::Finished(result) => {
            app.running = None;
            match result {
                Ok(outcome) => {
                    app.session.messages = outcome.messages;
                    app.session.usage.input_tokens += outcome.usage.input_tokens;
                    app.session.usage.output_tokens += outcome.usage.output_tokens;
                    store.save(&mut app.session).await?;
                    app.status = format!("Ready · {} model turn(s)", outcome.turns);
                }
                Err(error) => app.status = format!("Error: {error}"),
            }
        }
    }
    Ok(())
}

async fn handle_key(
    key: KeyEvent,
    app: &mut App,
    agent: &Arc<Agent>,
    store: &SessionStore,
    tx: &mpsc::UnboundedSender<UiEvent>,
) -> Result<()> {
    if let Some(approval) = app.approval.take() {
        let approved = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
        if matches!(
            key.code,
            KeyCode::Char('y' | 'Y' | 'n' | 'N') | KeyCode::Esc
        ) {
            let _ = approval.response.send(if approved {
                ApprovalOutcome::Approved
            } else {
                ApprovalOutcome::Denied
            });
            app.status = if approved { "Approved" } else { "Denied" }.into();
        } else {
            app.approval = Some(approval);
        }
        return Ok(());
    }
    if app.show_sessions {
        match key.code {
            KeyCode::Esc | KeyCode::Char('s') => app.show_sessions = false,
            KeyCode::Up => app.selected_session = app.selected_session.saturating_sub(1),
            KeyCode::Down => {
                app.selected_session =
                    (app.selected_session + 1).min(app.sessions.len().saturating_sub(1));
            }
            KeyCode::Enter if !app.is_running() => {
                if let Some(session) = app.sessions.get(app.selected_session).cloned() {
                    app.session = session;
                    app.activity.clear();
                    app.status = "Session opened".into();
                    app.show_sessions = false;
                }
            }
            _ => {}
        }
        return Ok(());
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                if app.is_running() {
                    app.cancel();
                } else {
                    app.quit = true;
                }
            }
            KeyCode::Char('q') => app.quit = true,
            KeyCode::Char('s') => app.show_sessions = true,
            KeyCode::Char('n') if !app.is_running() => {
                app.session =
                    Session::new(app.session.workspace.clone(), app.session.model.clone());
                app.activity.clear();
                app.status = "New session".into();
            }
            KeyCode::Char('b') if !app.is_running() => {
                app.session = store.branch(&app.session, None).await?;
                app.sessions = store.list().await?;
                app.status = "Branched session".into();
            }
            KeyCode::Char('e') => {
                let path = export_path(&app.session);
                store.export_markdown(&app.session, &path).await?;
                app.status = format!("Exported to {}", path.display());
            }
            KeyCode::Char('k') if !app.is_running() => {
                let removed = compact_messages(&mut app.session.messages, 24);
                store.save(&mut app.session).await?;
                app.status = format!("Compacted {removed} messages");
            }
            _ => {}
        }
        return Ok(());
    }
    match key.code {
        KeyCode::Esc if app.is_running() => app.cancel(),
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => app.composer.insert('\n'),
        KeyCode::Enter if !app.is_running() => {
            let prompt = app.composer.take();
            if !prompt.trim().is_empty() {
                if handle_command(&prompt, app, store).await? {
                    return Ok(());
                }
                if app.session.messages.len() > 96 {
                    let removed = compact_messages(&mut app.session.messages, 64);
                    app.activity
                        .push(format!("context: compacted {removed} older messages"));
                }
                let history = app.session.messages.clone();
                let agent = agent.clone();
                let events = tx.clone();
                app.status = "Starting…  Esc cancels".into();
                let cancel = tokio_util::sync::CancellationToken::new();
                let run_cancel = cancel.clone();
                let task = tokio::spawn(async move {
                    let result = agent
                        .run_with_cancel(history, prompt, run_cancel)
                        .await
                        .map_err(|error| error.to_string());
                    let _ = events.send(UiEvent::Finished(result));
                });
                app.running = Some(Running { task, cancel });
            }
        }
        KeyCode::Char(character) if !app.is_running() => app.composer.insert(character),
        KeyCode::Backspace if !app.is_running() => app.composer.backspace(),
        KeyCode::Left if !app.is_running() => {
            if app.composer.cursor > 0 {
                app.composer.cursor = app.composer.text[..app.composer.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index);
            }
        }
        KeyCode::Right if !app.is_running() => {
            if let Some(character) = app.composer.text[app.composer.cursor..].chars().next() {
                app.composer.cursor += character.len_utf8();
            }
        }
        KeyCode::PageUp => app.scroll = app.scroll.saturating_add(8),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_sub(8),
        _ => {}
    }
    Ok(())
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);
    let title = app.session.name.as_deref().unwrap_or("untitled");
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " HELM ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {title} · {} · {}",
                app.session.model,
                app.session.workspace.display()
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    let transcript = transcript(app);
    let viewport_height = chunks[1].height.saturating_sub(2) as usize;
    let bottom = transcript.lines.len().saturating_sub(viewport_height) as u16;
    let offset = bottom.saturating_sub(app.scroll);
    frame.render_widget(
        Paragraph::new(transcript)
            .wrap(Wrap { trim: false })
            .scroll((offset, 0))
            .block(
                Block::default()
                    .title(" Conversation ")
                    .borders(Borders::ALL),
            ),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new(app.composer.text.as_str())
            .wrap(Wrap { trim: false })
            .style(if app.is_running() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            })
            .block(
                Block::default()
                    .title(" Prompt · Enter send · Alt+Enter newline ")
                    .borders(Borders::ALL),
            ),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{}  │  ^S sessions  ^N new  ^B branch  ^K compact  ^E export  ^Q quit  /help",
            app.status
        ))
        .style(Style::default().fg(Color::Gray)),
        chunks[3],
    );
    if !app.is_running() {
        let prefix = app.composer.text[..app.composer.cursor]
            .lines()
            .last()
            .unwrap_or_default();
        let row = app.composer.text[..app.composer.cursor]
            .chars()
            .filter(|character| *character == '\n')
            .count() as u16;
        frame.set_cursor_position((
            chunks[2].x + 1 + prefix.chars().count() as u16,
            chunks[2].y + 1 + row.min(2),
        ));
    }
    if app.show_sessions {
        draw_sessions(frame, area, app);
    }
    if let Some(approval) = &app.approval {
        draw_approval(frame, area, &approval.reason);
    }
}

fn transcript(app: &App) -> Text<'static> {
    let mut lines = Vec::new();
    for message in &app.session.messages {
        if message.role == Role::System || message.role == Role::Tool {
            continue;
        }
        let (label, color) = match message.role {
            Role::User => ("you", Color::Cyan),
            Role::Assistant => ("helm", Color::Green),
            _ => continue,
        };
        lines.push(Line::from(Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));
        lines.extend(
            message
                .content
                .lines()
                .map(|line| Line::raw(line.to_owned())),
        );
        lines.push(Line::raw(""));
    }
    if !app.activity.is_empty() {
        lines.push(Line::styled(
            "activity",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        lines.extend(
            app.activity
                .iter()
                .rev()
                .take(6)
                .rev()
                .map(|line| Line::styled(line.clone(), Style::default().fg(Color::DarkGray))),
        );
    }
    Text::from(lines)
}

fn draw_sessions(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered(area, 80, 70);
    let items: Vec<_> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(index, session)| {
            let marker = if index == app.selected_session {
                "▶"
            } else {
                " "
            };
            ListItem::new(format!(
                "{marker} {}  {}  {} messages",
                session.name.as_deref().unwrap_or("untitled"),
                session.updated_at.format("%Y-%m-%d %H:%M"),
                session.messages.len()
            ))
        })
        .collect();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Sessions · ↑↓ select · Enter open · Esc close ")
                .borders(Borders::ALL),
        ),
        popup,
    );
}

fn draw_approval(frame: &mut ratatui::Frame<'_>, area: Rect, reason: &str) {
    let popup = centered(area, 70, 35);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(format!("{reason}\n\n[y] approve    [n/Esc] deny"))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(" Approval required ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow)),
            ),
        popup,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height) / 2),
        Constraint::Percentage(height),
        Constraint::Percentage((100 - height) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - width) / 2),
        Constraint::Percentage(width),
        Constraint::Percentage((100 - width) / 2),
    ])
    .split(vertical[1])[1]
}

fn one_line(text: &str, max: usize) -> String {
    let text = text.lines().next().unwrap_or_default();
    let mut output: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        output.push('…');
    }
    output
}

fn export_path(session: &Session) -> PathBuf {
    session
        .workspace
        .join(format!("helm-session-{}.md", session.id))
}

async fn handle_command(command: &str, app: &mut App, store: &SessionStore) -> Result<bool> {
    let Some(command) = command.strip_prefix('/') else {
        return Ok(false);
    };
    let (name, argument) = command.split_once(' ').unwrap_or((command, ""));
    match name {
        "help" => {
            app.status =
                "/name TITLE · /branch [TITLE] · /compact [KEEP] · /export [PATH] · /clear".into()
        }
        "name" if !argument.trim().is_empty() => {
            app.session.name = Some(argument.trim().into());
            store.save(&mut app.session).await?;
            app.sessions = store.list().await?;
            app.status = "Session renamed".into();
        }
        "branch" => {
            let branch_name = (!argument.trim().is_empty()).then(|| argument.trim().to_owned());
            app.session = store.branch(&app.session, branch_name).await?;
            app.sessions = store.list().await?;
            app.status = "Branched session".into();
        }
        "compact" => {
            let retain = if argument.trim().is_empty() {
                24
            } else {
                argument
                    .trim()
                    .parse()
                    .context("/compact expects a message count")?
            };
            let removed = compact_messages(&mut app.session.messages, retain);
            store.save(&mut app.session).await?;
            app.status = format!("Compacted {removed} messages");
        }
        "export" => {
            let path = if argument.trim().is_empty() {
                export_path(&app.session)
            } else {
                PathBuf::from(argument.trim())
            };
            store.export_markdown(&app.session, &path).await?;
            app.status = format!("Exported to {}", path.display());
        }
        "clear" => {
            app.session.messages.clear();
            store.save(&mut app.session).await?;
            app.activity.clear();
            app.status = "Conversation cleared".into();
        }
        _ => app.status = format!("Unknown or incomplete command: /{name}"),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_edits_utf8_safely() {
        let mut composer = Composer::default();
        composer.insert('λ');
        composer.insert('x');
        composer.backspace();
        assert_eq!(composer.text, "λ");
        composer.backspace();
        assert!(composer.text.is_empty());
    }

    #[test]
    fn truncation_uses_character_boundaries() {
        assert_eq!(one_line("αβγδε", 3), "αβγ…");
    }

    #[test]
    fn renders_small_terminal_without_panicking() {
        let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        let app = App::new(session, Vec::new());
        let backend = ratatui::backend::TestBackend::new(60, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered = terminal.backend().buffer().content();
        assert!(rendered.iter().any(|cell| cell.symbol() == "H"));
    }

    #[tokio::test]
    async fn local_commands_name_and_export_session() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let session = Session::new(directory.path().into(), "test-model".into());
        let mut app = App::new(session, Vec::new());
        assert!(
            handle_command("/name field work", &mut app, &store)
                .await
                .unwrap()
        );
        assert_eq!(app.session.name.as_deref(), Some("field work"));
        let export = directory.path().join("export.md");
        handle_command(&format!("/export {}", export.display()), &mut app, &store)
            .await
            .unwrap();
        assert!(export.exists());
    }
}
