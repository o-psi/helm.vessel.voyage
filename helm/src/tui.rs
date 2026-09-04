//! Full-screen terminal frontend. This module intentionally depends only on the
//! agent's event and approval contracts, so providers can add finer-grained
//! streaming without changing the UI.

use std::{io, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use crossterm::{
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
        KeyModifiers,
    },
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
use unicode_width::UnicodeWidthChar;

use crate::{
    Agent, AgentEvent, EventSink,
    model::Role,
    session::{Session, SessionStore, compact_messages},
    terminal::{
        InteractiveTerminals, TerminalColor, TerminalEvent, TerminalId, TerminalSnapshot,
        TerminalSummary,
    },
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
    id: uuid::Uuid,
    action: String,
    target: String,
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
                id: request.id,
                action: request.action.clone(),
                target: request.target.clone(),
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
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
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

    fn insert_str(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
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

    fn delete(&mut self) {
        if let Some(character) = self.text[self.cursor..].chars().next() {
            self.text
                .drain(self.cursor..self.cursor + character.len_utf8());
        }
    }

    fn line_start(&mut self) {
        self.cursor = self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
    }

    fn line_end(&mut self) {
        self.cursor += self.text[self.cursor..]
            .find('\n')
            .unwrap_or(self.text.len() - self.cursor);
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
    terminals: Vec<TerminalSummary>,
    terminal_picker: bool,
    selected_terminal: usize,
    attached_terminal: Option<TerminalId>,
    terminal_snapshot: Option<TerminalSnapshot>,
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
            terminals: Vec::new(),
            terminal_picker: false,
            selected_terminal: 0,
            attached_terminal: None,
            terminal_snapshot: None,
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
    terminals: Arc<dyn InteractiveTerminals>,
) -> Result<()> {
    let sessions = store.list().await?;
    let mut app = App::new(session, sessions);
    refresh_terminals(&mut app, terminals.as_ref()).await;
    let mut terminal_events = terminals.subscribe();
    let _guard = TerminalGuard::enter().context("failed to initialize terminal")?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let mut input = EventStream::new();
    let termination = termination_signal();
    tokio::pin!(termination);

    while !app.quit {
        terminal.draw(|frame| draw(frame, &app))?;
        tokio::select! {
            event = input.next() => {
                match event {
                    Some(Ok(Event::Key(key))) if key.is_press() => {
                        handle_key(key, &mut app, &agent, &store, &tx, terminals.as_ref()).await?;
                    }
                    Some(Ok(Event::Resize(columns, rows))) => {
                        if let Some(id) = app.attached_terminal {
                            let _ = terminals.resize(id, columns, rows.saturating_sub(1)).await;
                        }
                    }
                    Some(Ok(Event::Paste(text))) if app.approval.is_none() && !app.show_sessions => {
                        if let Some(id) = app.attached_terminal {
                            if let Err(error) = terminals.write(id, text.into_bytes()).await {
                                app.status = format!("Terminal input failed: {error}");
                            }
                        } else if !app.is_running() && !app.terminal_picker {
                            app.composer.insert_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
                        }
                    }
                    Some(Err(error)) => return Err(error.into()),
                    None => break,
                    _ => {}
                }
            }
            event = rx.recv() => {
                let Some(event) = event else { break };
                handle_ui_event(event, &mut app, &store, terminals.as_ref()).await?;
            }
            _ = &mut termination => {
                app.status = "Terminal closing; cancelling active work".into();
                app.quit = true;
            }
            event = terminal_events.recv() => {
                match event {
                    Ok(event) => handle_terminal_event(event, &mut app, terminals.as_ref()).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => refresh_terminals(&mut app, terminals.as_ref()).await,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        app.attached_terminal = None;
                        app.terminal_snapshot = None;
                        app.status = "Terminal manager disconnected".into();
                    }
                }
            }
        }
    }
    app.cancel();
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(unix)]
async fn termination_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let Ok(mut terminate) = signal(SignalKind::terminate()) else {
        std::future::pending::<()>().await;
        return;
    };
    let Ok(mut hangup) = signal(SignalKind::hangup()) else {
        terminate.recv().await;
        return;
    };
    tokio::select! {
        _ = terminate.recv() => {}
        _ = hangup.recv() => {}
    }
}

#[cfg(not(unix))]
async fn termination_signal() {
    std::future::pending::<()>().await;
}

async fn handle_ui_event(
    event: UiEvent,
    app: &mut App,
    store: &SessionStore,
    terminals: &dyn InteractiveTerminals,
) -> Result<()> {
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
            if app.attached_terminal.is_some() {
                let _ = request.response.send(ApprovalOutcome::Unavailable);
                app.status = "Agent approval denied while direct terminal input is attached".into();
            } else {
                app.status = "Approval required".into();
                app.approval = Some(request);
            }
        }
        UiEvent::Finished(result) => {
            app.running = None;
            match result {
                Ok(outcome) => {
                    app.session.messages = outcome.messages;
                    app.session.usage.input_tokens += outcome.usage.input_tokens;
                    app.session.usage.output_tokens += outcome.usage.output_tokens;
                    app.session.terminals = terminals.list().await.unwrap_or_default();
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
    terminals: &dyn InteractiveTerminals,
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
    if let Some(id) = app.attached_terminal {
        handle_attached_key(id, key, app, terminals).await;
        return Ok(());
    }
    if app.terminal_picker {
        match key.code {
            KeyCode::Esc | KeyCode::Char('t') => app.terminal_picker = false,
            KeyCode::Up => app.selected_terminal = app.selected_terminal.saturating_sub(1),
            KeyCode::Down => {
                app.selected_terminal =
                    (app.selected_terminal + 1).min(app.terminals.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(summary) = app.terminals.get(app.selected_terminal) {
                    let id = summary.id;
                    match terminals.snapshot(id).await {
                        Ok(snapshot) => {
                            app.attached_terminal = Some(id);
                            app.terminal_snapshot = Some(snapshot);
                            app.terminal_picker = false;
                        }
                        Err(error) => app.status = format!("Cannot attach: {error}"),
                    }
                }
            }
            KeyCode::Char('r') => refresh_terminals(app, terminals).await,
            _ => {}
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
            KeyCode::Char('t') => {
                refresh_terminals(app, terminals).await;
                app.terminal_picker = true;
            }
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
        KeyCode::Delete if !app.is_running() => app.composer.delete(),
        KeyCode::Home if !app.is_running() => app.composer.line_start(),
        KeyCode::End if !app.is_running() => app.composer.line_end(),
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

async fn handle_attached_key(
    id: TerminalId,
    key: KeyEvent,
    app: &mut App,
    terminals: &dyn InteractiveTerminals,
) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(']') {
        app.attached_terminal = None;
        app.terminal_snapshot = None;
        app.status = "Detached; terminal is still running".into();
    } else if let Some(bytes) = encode_terminal_key(key)
        && let Err(error) = terminals.write(id, bytes).await
    {
        app.status = format!("Terminal input failed: {error}");
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    if app.attached_terminal.is_some() {
        draw_attached_terminal(frame, area, app);
        return;
    }
    if area.width < 32 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("Helm needs a terminal of at least 32×10. Resize the window or use `helm chat --plain`.")
                .wrap(Wrap { trim: true })
                .block(Block::default().title(" Helm ").borders(Borders::ALL)),
            area,
        );
        return;
    }
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
    let viewport_width = chunks[1].width.saturating_sub(2) as usize;
    let rendered_lines = transcript
        .lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(viewport_width.max(1)))
        .sum::<usize>();
    let bottom = rendered_lines.saturating_sub(viewport_height) as u16;
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
    let composer_width = chunks[2].width.saturating_sub(2).max(1);
    let composer_height = chunks[2].height.saturating_sub(2).max(1);
    let (composer_row, composer_column) =
        cursor_position(&app.composer.text[..app.composer.cursor], composer_width);
    let composer_scroll = composer_row.saturating_sub(composer_height - 1);
    frame.render_widget(
        Paragraph::new(app.composer.text.as_str())
            .wrap(Wrap { trim: false })
            .scroll((composer_scroll, 0))
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
            "{}  │  ^T terminals  ^S sessions  ^N new  ^B branch  ^K compact  ^E export  ^Q quit",
            app.status
        ))
        .style(Style::default().fg(Color::Gray)),
        chunks[3],
    );
    if !app.is_running() {
        frame.set_cursor_position((
            (chunks[2].x + 1 + composer_column).min(chunks[2].right().saturating_sub(2)),
            (chunks[2].y + 1 + composer_row.saturating_sub(composer_scroll))
                .min(chunks[2].bottom().saturating_sub(2)),
        ));
    }
    if app.show_sessions {
        draw_sessions(frame, area, app);
    }
    if app.terminal_picker {
        draw_terminal_picker(frame, area, app);
    }
    if let Some(approval) = &app.approval {
        draw_approval(frame, area, approval);
    }
}

fn draw_attached_terminal(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let (title, state) = app
        .terminal_snapshot
        .as_ref()
        .map(|snapshot| (snapshot.title.as_str(), format!("{:?}", snapshot.state)))
        .unwrap_or(("loading", "unknown".into()));
    frame.render_widget(
        Paragraph::new(format!(
            " HELM TERMINAL · {title} · {state}{} · Ctrl+] detach (process keeps running)",
            app.terminal_snapshot
                .as_ref()
                .filter(|snapshot| snapshot.dropped_unread_bytes > 0)
                .map(|snapshot| format!(
                    " · {} agent-unread bytes evicted",
                    snapshot.dropped_unread_bytes
                ))
                .unwrap_or_default()
        ))
        .style(Style::default().fg(Color::Black).bg(Color::Cyan)),
        chunks[0],
    );
    if let Some(snapshot) = &app.terminal_snapshot {
        let screen = Text::from(
            snapshot
                .cells
                .iter()
                .map(|row| {
                    Line::from(
                        row.iter()
                            .map(|cell| {
                                let mut style = Style::default()
                                    .fg(terminal_color(cell.foreground))
                                    .bg(terminal_color(cell.background));
                                if cell.bold {
                                    style = style.add_modifier(Modifier::BOLD);
                                }
                                if cell.dim {
                                    style = style.add_modifier(Modifier::DIM);
                                }
                                if cell.italic {
                                    style = style.add_modifier(Modifier::ITALIC);
                                }
                                if cell.underlined {
                                    style = style.add_modifier(Modifier::UNDERLINED);
                                }
                                if cell.reversed {
                                    style = style.add_modifier(Modifier::REVERSED);
                                }
                                Span::styled(cell.text.clone(), style)
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
        );
        frame.render_widget(Paragraph::new(screen), chunks[1]);
        if let Some((column, row)) = snapshot.cursor {
            frame.set_cursor_position((
                chunks[1]
                    .x
                    .saturating_add(column)
                    .min(chunks[1].right().saturating_sub(1)),
                chunks[1]
                    .y
                    .saturating_add(row)
                    .min(chunks[1].bottom().saturating_sub(1)),
            ));
        }
    } else {
        frame.render_widget(Paragraph::new("Waiting for terminal screen…"), chunks[1]);
    }
}

fn terminal_color(color: TerminalColor) -> Color {
    match color {
        TerminalColor::Default => Color::Reset,
        TerminalColor::Indexed(index) => Color::Indexed(index),
        TerminalColor::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

fn draw_terminal_picker(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered(area, 80, 70);
    let items: Vec<_> = app
        .terminals
        .iter()
        .enumerate()
        .map(|(index, terminal)| {
            ListItem::new(format!(
                "{} {}  {}  {:?}",
                if index == app.selected_terminal {
                    "▶"
                } else {
                    " "
                },
                terminal.id,
                terminal.title,
                terminal.state
            ))
        })
        .collect();
    let items = if items.is_empty() {
        vec![ListItem::new(
            "No interactive terminals · r refresh · Esc close",
        )]
    } else {
        items
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Terminals · ↑↓ select · Enter attach · r refresh · Esc close ")
                .borders(Borders::ALL),
        ),
        popup,
    );
}

async fn refresh_terminals(app: &mut App, terminals: &dyn InteractiveTerminals) {
    match terminals.list().await {
        Ok(list) => {
            app.terminals = list;
            app.selected_terminal = app
                .selected_terminal
                .min(app.terminals.len().saturating_sub(1));
        }
        Err(error) => app.status = format!("Cannot list terminals: {error}"),
    }
}

async fn handle_terminal_event(
    event: TerminalEvent,
    app: &mut App,
    terminals: &dyn InteractiveTerminals,
) {
    let id = match event {
        TerminalEvent::Changed(id) | TerminalEvent::Added(id) | TerminalEvent::Removed(id) => id,
    };
    refresh_terminals(app, terminals).await;
    if app.attached_terminal == Some(id) {
        match terminals.snapshot(id).await {
            Ok(snapshot) => app.terminal_snapshot = Some(snapshot),
            Err(_) => {
                app.attached_terminal = None;
                app.terminal_snapshot = None;
                app.status = "Attached terminal disappeared".into();
            }
        }
    }
}

fn encode_terminal_key(key: KeyEvent) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
    }
    match key.code {
        KeyCode::Char(character) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let upper = character.to_ascii_uppercase() as u32;
            if (64..=95).contains(&upper) {
                bytes.push((upper - 64) as u8);
            } else {
                return None;
            }
        }
        KeyCode::Char(character) => {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
        KeyCode::Enter => bytes.push(b'\r'),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::BackTab => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => bytes.push(0x7f),
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::Up => bytes.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => bytes.extend_from_slice(b"\x1b[B"),
        KeyCode::Right => bytes.extend_from_slice(b"\x1b[C"),
        KeyCode::Left => bytes.extend_from_slice(b"\x1b[D"),
        KeyCode::Home => bytes.extend_from_slice(b"\x1b[H"),
        KeyCode::End => bytes.extend_from_slice(b"\x1b[F"),
        KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
        KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        KeyCode::F(number) if (1..=4).contains(&number) => {
            bytes.extend_from_slice(&[0x1b, b'O', b'P' + number - 1]);
        }
        KeyCode::F(number) if (5..=12).contains(&number) => {
            const CODES: [&[u8]; 8] = [b"15", b"17", b"18", b"19", b"20", b"21", b"23", b"24"];
            bytes.extend_from_slice(b"\x1b[");
            bytes.extend_from_slice(CODES[(number - 5) as usize]);
            bytes.push(b'~');
        }
        _ => return None,
    }
    Some(bytes)
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
    let items = if items.is_empty() {
        vec![ListItem::new("No saved sessions yet")]
    } else {
        items
    };
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

fn draw_approval(frame: &mut ratatui::Frame<'_>, area: Rect, approval: &ApprovalRequest) {
    let popup = centered(area, 70, 35);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(format!(
            "Action: {}\nTarget: {}\nRequest: {}\n\n{}\n\n[y] approve    [n/Esc] deny",
            approval.action, approval.target, approval.id, approval.reason
        ))
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

fn cursor_position(text: &str, width: u16) -> (u16, u16) {
    let width = width.max(1);
    let mut row = 0_u16;
    let mut column = 0_u16;
    for character in text.chars() {
        if character == '\n' {
            row = row.saturating_add(1);
            column = 0;
            continue;
        }
        let character_width = character.width().unwrap_or(0) as u16;
        if column.saturating_add(character_width) > width {
            row = row.saturating_add(1);
            column = 0;
        }
        column = column.saturating_add(character_width);
        if column == width {
            row = row.saturating_add(1);
            column = 0;
        }
    }
    (row, column)
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
                "/name TITLE · /branch [TITLE] · /compact [KEEP] · /export [PATH] · /clear confirm"
                    .into()
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
        "clear" if argument.trim() == "confirm" => {
            app.session.messages.clear();
            store.save(&mut app.session).await?;
            app.activity.clear();
            app.status = "Conversation cleared".into();
        }
        "clear" => app.status = "Clearing is permanent; use /clear confirm".into(),
        _ => app.status = format!("Unknown or incomplete command: /{name}"),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{TerminalCell, TerminalError, TerminalState};
    use std::sync::Mutex;

    struct FakeTerminals {
        id: TerminalId,
        writes: Mutex<Vec<Vec<u8>>>,
        resizes: Mutex<Vec<(u16, u16)>>,
        events: tokio::sync::broadcast::Sender<TerminalEvent>,
    }

    impl FakeTerminals {
        fn new() -> Self {
            let (events, _) = tokio::sync::broadcast::channel(8);
            Self {
                id: TerminalId(uuid::Uuid::new_v4()),
                writes: Mutex::new(Vec::new()),
                resizes: Mutex::new(Vec::new()),
                events,
            }
        }
    }

    #[async_trait]
    impl InteractiveTerminals for FakeTerminals {
        async fn list(&self) -> Result<Vec<TerminalSummary>, TerminalError> {
            Ok(vec![TerminalSummary {
                id: self.id,
                title: "shell".into(),
                state: TerminalState::Running,
            }])
        }
        async fn snapshot(&self, id: TerminalId) -> Result<TerminalSnapshot, TerminalError> {
            if id != self.id {
                return Err(TerminalError::NotFound(id));
            }
            Ok(TerminalSnapshot {
                id,
                title: "shell".into(),
                state: TerminalState::Running,
                revision: 1,
                cells: vec![vec![TerminalCell {
                    text: "$ ready".into(),
                    ..TerminalCell::default()
                }]],
                cursor: Some((2, 0)),
                dropped_unread_bytes: 0,
            })
        }
        async fn write(&self, id: TerminalId, bytes: Vec<u8>) -> Result<(), TerminalError> {
            if id != self.id {
                return Err(TerminalError::NotFound(id));
            }
            self.writes.lock().unwrap().push(bytes);
            Ok(())
        }
        async fn resize(
            &self,
            id: TerminalId,
            columns: u16,
            rows: u16,
        ) -> Result<(), TerminalError> {
            if id != self.id {
                return Err(TerminalError::NotFound(id));
            }
            self.resizes.lock().unwrap().push((columns, rows));
            Ok(())
        }
        fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TerminalEvent> {
            self.events.subscribe()
        }
    }

    #[test]
    fn composer_edits_utf8_safely() {
        let mut composer = Composer::default();
        composer.insert('λ');
        composer.insert('x');
        composer.backspace();
        assert_eq!(composer.text, "λ");
        composer.backspace();
        assert!(composer.text.is_empty());
        composer.insert_str("one\nλtwo");
        composer.line_start();
        assert_eq!(&composer.text[composer.cursor..], "λtwo");
        composer.line_end();
        composer.delete();
        assert_eq!(composer.cursor, composer.text.len());
    }

    #[test]
    fn truncation_uses_character_boundaries() {
        assert_eq!(one_line("αβγδε", 3), "αβγ…");
    }

    #[test]
    fn cursor_tracks_wrapping_and_wide_characters() {
        assert_eq!(cursor_position("abcd", 4), (1, 0));
        assert_eq!(cursor_position("ab\n界", 4), (1, 2));
        assert_eq!(cursor_position("abcde", 4), (1, 1));
    }

    #[test]
    fn encodes_terminal_keys_without_text_transformation() {
        assert_eq!(
            encode_terminal_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![3])
        );
        assert_eq!(
            encode_terminal_key(KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE)),
            Some("界".as_bytes().to_vec())
        );
        assert_eq!(
            encode_terminal_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_terminal_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
            Some(b"\x1bx".to_vec())
        );
    }

    #[tokio::test]
    async fn attached_input_is_isolated_and_detach_does_not_close_terminal() {
        let directory = tempfile::tempdir().unwrap();
        let session = Session::new(directory.path().into(), "test-model".into());
        let mut app = App::new(session, Vec::new());
        let terminals = FakeTerminals::new();
        app.attached_terminal = Some(terminals.id);
        app.terminal_snapshot = Some(terminals.snapshot(terminals.id).await.unwrap());
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE);
        handle_attached_key(terminals.id, key, &mut app, &terminals).await;
        assert_eq!(
            terminals.writes.lock().unwrap().as_slice(),
            &[b"p".to_vec()]
        );
        assert!(app.composer.text.is_empty());
        assert!(app.session.messages.is_empty());
        handle_attached_key(
            terminals.id,
            KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL),
            &mut app,
            &terminals,
        )
        .await;
        assert!(app.attached_terminal.is_none());
        assert_eq!(
            terminals.writes.lock().unwrap().len(),
            1,
            "detach chord must not reach the PTY"
        );
        assert_eq!(
            terminals.list().await.unwrap()[0].state,
            TerminalState::Running
        );
    }

    #[tokio::test]
    async fn attached_keystrokes_can_never_answer_agent_approvals() {
        let directory = tempfile::tempdir().unwrap();
        let session = Session::new(directory.path().into(), "test-model".into());
        let mut app = App::new(session, Vec::new());
        app.attached_terminal = Some(TerminalId(uuid::Uuid::new_v4()));
        let (response, receive) = oneshot::channel();
        let store = SessionStore::new(directory.path().join("sessions"));
        handle_ui_event(
            UiEvent::Approval(ApprovalRequest {
                id: uuid::Uuid::new_v4(),
                action: "shell".into(),
                target: "dangerous".into(),
                reason: "test".into(),
                response,
            }),
            &mut app,
            &store,
            &crate::terminal::NoInteractiveTerminals::default(),
        )
        .await
        .unwrap();
        assert_eq!(receive.await.unwrap(), ApprovalOutcome::Unavailable);
        assert!(app.approval.is_none());
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

    #[test]
    fn renders_tiny_terminal_with_resize_guidance() {
        let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        let app = App::new(session, Vec::new());
        let backend = ratatui::backend::TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("Helm"));
    }

    #[tokio::test]
    async fn clear_requires_explicit_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let mut session = Session::new(directory.path().into(), "test-model".into());
        session
            .messages
            .push(crate::Message::new(Role::User, "keep me"));
        let mut app = App::new(session, Vec::new());
        handle_command("/clear", &mut app, &store).await.unwrap();
        assert_eq!(app.session.messages.len(), 1);
        handle_command("/clear confirm", &mut app, &store)
            .await
            .unwrap();
        assert!(app.session.messages.is_empty());
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
