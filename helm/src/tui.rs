//! Full-screen terminal frontend. This module intentionally depends only on the
//! agent's event and approval contracts, so providers can add finer-grained
//! streaming without changing the UI.

use std::{collections::BTreeSet, io, path::PathBuf, sync::Arc};

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
    markdown::{MarkdownTheme, RenderOptions, render_markdown},
    model::Role,
    provider::ModelInfo,
    session::{Session, SessionStore, compact_messages},
    supervision::{
        AgentEvent as SupervisionEvent, AgentEventKind as SupervisionEventKind, AgentId,
        AgentStatus, AgentSupervisor, AgentView,
    },
    terminal::{
        InteractiveTerminals, TerminalColor, TerminalEvent, TerminalId, TerminalSnapshot,
        TerminalSummary,
    },
    todo::{EntryKind, NewTodo, Priority, TodoId, TodoItem, TodoList, TodoStatus, TodoStore},
    tools::{ApprovalOutcome, ApprovalRequest as ToolApprovalRequest, Approver},
};

#[derive(Debug)]
#[doc(hidden)]
pub enum UiEvent {
    Agent(AgentEvent),
    Approval(ApprovalRequest),
    Finished(Result<crate::AgentOutcome, String>),
    SupervisorTree(Result<Vec<AgentView>, String>),
    SupervisorInspect(AgentId, Result<Vec<SupervisionEvent>, String>),
    SupervisorAction(Result<String, String>),
    TodoSnapshot(Result<TodoList, String>),
    TodoAction(Result<String, String>),
    Models(Result<Vec<ModelInfo>, String>),
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
    provider_label: String,
    composer: Composer,
    activity: Vec<String>,
    streaming_response: String,
    status: String,
    scroll: u16,
    conversation_width: usize,
    conversation_height: usize,
    markdown_theme: MarkdownTheme,
    markdown_syntax_highlighting: bool,
    running: Option<Running>,
    approval: Option<ApprovalRequest>,
    show_sessions: bool,
    selected_session: usize,
    terminals: Vec<TerminalSummary>,
    terminal_picker: bool,
    selected_terminal: usize,
    attached_terminal: Option<TerminalId>,
    terminal_snapshot: Option<TerminalSnapshot>,
    supervisor_mode: Option<SupervisorMode>,
    agents: Vec<AgentView>,
    selected_agent: usize,
    inspected_events: Vec<SupervisionEvent>,
    supervisor_input: Composer,
    cancel_armed: Option<AgentId>,
    supervisor_scroll: u16,
    todo_mode: Option<TodoMode>,
    todos: Vec<TodoItem>,
    todo_revision: u64,
    selected_todo: usize,
    todo_scroll: u16,
    todo_input: Composer,
    model_picker: bool,
    models: Vec<ModelInfo>,
    selected_model: usize,
    model_filter: Composer,
    model_manual: bool,
    quit: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SupervisorMode {
    Tree,
    Inspect(AgentId),
    Message { target: AgentId, follow_up: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TodoMode {
    List,
    Inspect(TodoId),
    Input {
        target: Option<TodoId>,
        action: TodoInput,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TodoInput {
    Add,
    Edit,
    Block,
    Assign,
    Dependencies,
    Progress,
    Note,
    Evidence,
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
            provider_label: "provider unknown".into(),
            composer: Composer::default(),
            activity: Vec::new(),
            streaming_response: String::new(),
            status: "Ready".into(),
            scroll: 0,
            conversation_width: 78,
            conversation_height: 13,
            markdown_theme: markdown_theme(),
            markdown_syntax_highlighting: std::env::var_os("NO_COLOR").is_none(),
            running: None,
            approval: None,
            show_sessions: false,
            selected_session: 0,
            terminals: Vec::new(),
            terminal_picker: false,
            selected_terminal: 0,
            attached_terminal: None,
            terminal_snapshot: None,
            supervisor_mode: None,
            agents: Vec::new(),
            selected_agent: 0,
            inspected_events: Vec::new(),
            supervisor_input: Composer::default(),
            cancel_armed: None,
            supervisor_scroll: 0,
            todo_mode: None,
            todos: Vec::new(),
            todo_revision: 0,
            selected_todo: 0,
            todo_scroll: 0,
            todo_input: Composer::default(),
            model_picker: false,
            models: Vec::new(),
            selected_model: 0,
            model_filter: Composer::default(),
            model_manual: false,
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

// Each argument is a distinct lifecycle-owned channel/backend assembled by the CLI.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    agent: Arc<Agent>,
    store: SessionStore,
    session: Session,
    mut rx: mpsc::UnboundedReceiver<UiEvent>,
    tx: mpsc::UnboundedSender<UiEvent>,
    terminals: Arc<dyn InteractiveTerminals>,
    supervisor: Arc<dyn AgentSupervisor>,
    todos: Arc<TodoStore>,
    provider_label: String,
) -> Result<()> {
    let sessions = store.list().await?;
    let mut app = App::new(session, sessions);
    app.provider_label = provider_label;
    refresh_terminals(&mut app, terminals.as_ref()).await;
    let mut terminal_events = Some(terminals.subscribe());
    let mut supervisor_events = Some(supervisor.subscribe());
    let mut todo_refresh = tokio::time::interval(std::time::Duration::from_secs(1));
    let _guard = TerminalGuard::enter().context("failed to initialize terminal")?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    if let Ok(size) = terminal.size() {
        app.conversation_width = size.width.saturating_sub(2).max(1) as usize;
        app.conversation_height = size.height.saturating_sub(11).max(1) as usize;
    }
    let mut input = EventStream::new();
    let termination = termination_signal();
    tokio::pin!(termination);

    while !app.quit {
        terminal.draw(|frame| draw(frame, &app))?;
        tokio::select! {
            event = input.next() => {
                match event {
                    Some(Ok(Event::Key(key))) if key.is_press() => {
                        handle_key(key, &mut app, &agent, &store, &tx, terminals.as_ref(), supervisor.clone(), todos.clone()).await?;
                    }
                    Some(Ok(Event::Resize(columns, rows))) => {
                        resize_conversation(
                            &mut app,
                            columns.saturating_sub(2) as usize,
                            rows.saturating_sub(11).max(1) as usize,
                        );
                        if let Some(id) = app.attached_terminal {
                            let _ = terminals.resize(id, columns, rows.saturating_sub(1)).await;
                        }
                        // Discard any stale cells after the terminal changes its backing grid.
                        terminal.clear()?;
                    }
                    Some(Ok(Event::Paste(text))) if app.approval.is_none() && !app.show_sessions => {
                        if let Some(id) = app.attached_terminal {
                            if let Err(error) = terminals.write(id, text.into_bytes()).await {
                                app.status = format!("Terminal input failed: {error}");
                            }
                        } else if !app.is_running() && !app.terminal_picker {
                            let text = text.replace("\r\n", "\n").replace('\r', "\n");
                            if app.model_picker {
                                app.model_filter.insert_str(&text);
                                app.selected_model = 0;
                            } else if matches!(app.supervisor_mode, Some(SupervisorMode::Message { .. })) {
                                app.supervisor_input.insert_str(&text);
                            } else if matches!(app.todo_mode, Some(TodoMode::Input { .. })) {
                                app.todo_input.insert_str(&text);
                            } else if app.supervisor_mode.is_none() {
                                app.composer.insert_str(&text);
                            }
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
            event = async { terminal_events.as_mut().expect("guarded terminal receiver").recv().await }, if terminal_events.is_some() => {
                match event {
                    Ok(event) => handle_terminal_event(event, &mut app, terminals.as_ref()).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => refresh_terminals(&mut app, terminals.as_ref()).await,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        terminal_events = None;
                        app.attached_terminal = None;
                        app.terminal_snapshot = None;
                        app.status = "Terminal manager disconnected".into();
                    }
                }
            }
            event = async { supervisor_events.as_mut().expect("guarded supervisor receiver").recv().await }, if supervisor_events.is_some() => {
                match event {
                    Ok(event) => {
                        if matches!(app.supervisor_mode, Some(SupervisorMode::Inspect(id)) if id == event.agent_id) {
                            app.inspected_events.push(event);
                            if app.inspected_events.len() > 500 { app.inspected_events.drain(..100); }
                        }
                        request_supervisor_tree(&tx, supervisor.clone());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => request_supervisor_tree(&tx, supervisor.clone()),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        supervisor_events = None;
                        app.status = "Agent supervisor disconnected".into();
                    }
                }
            }
            _ = todo_refresh.tick(), if app.todo_mode.is_some() => request_todo_snapshot(&tx, todos.clone()),
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
            let before = transcript_height(app, app.conversation_width);
            app.streaming_response.push_str(&text);
            preserve_manual_anchor(app, before);
            app.status = "Receiving response…  Esc cancels".into();
        }
        UiEvent::Agent(AgentEvent::AssistantText(text)) => {
            let before = transcript_height(app, app.conversation_width);
            app.streaming_response = text.clone();
            preserve_manual_anchor(app, before);
            app.status = "Receiving response…  Esc cancels".into();
        }
        UiEvent::Agent(AgentEvent::ToolStarted { name, arguments }) => {
            let before = transcript_height(app, app.conversation_width);
            app.activity.push(format!("▶ {name} {arguments}"));
            preserve_manual_anchor(app, before);
            app.status = format!("Running {name}…  Esc cancels");
        }
        UiEvent::Agent(AgentEvent::ToolFinished {
            name,
            result,
            success,
        }) => {
            let before = transcript_height(app, app.conversation_width);
            app.activity.push(format!(
                "{} {name}: {}",
                if success { "✓" } else { "✗" },
                one_line(&result, 160)
            ));
            preserve_manual_anchor(app, before);
        }
        UiEvent::Agent(AgentEvent::ProviderRetry {
            attempt,
            delay,
            error,
        }) => {
            let before = transcript_height(app, app.conversation_width);
            app.activity.push(format!(
                "↻ provider retry {attempt} in {:.1}s: {}",
                delay.as_secs_f32(),
                one_line(&error, 160)
            ));
            preserve_manual_anchor(app, before);
            app.status = format!("Provider retry {attempt}…  Esc cancels");
        }
        UiEvent::Agent(AgentEvent::Cancelled) => {
            let before = transcript_height(app, app.conversation_width);
            app.activity.push("■ operation cancelled".into());
            preserve_manual_anchor(app, before);
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
            let before = transcript_height(app, app.conversation_width);
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
            app.streaming_response.clear();
            preserve_manual_anchor(app, before);
        }
        UiEvent::SupervisorTree(result) => match result {
            Ok(agents) => {
                app.agents = flatten_agent_tree(agents);
                app.selected_agent = app.selected_agent.min(app.agents.len().saturating_sub(1));
                app.status = format!("Supervising {} agent(s)", app.agents.len());
            }
            Err(error) => app.status = format!("Supervisor refresh failed: {error}"),
        },
        UiEvent::SupervisorInspect(id, result) => match result {
            Ok(events) => {
                app.inspected_events = events;
                app.supervisor_mode = Some(SupervisorMode::Inspect(id));
            }
            Err(error) => app.status = format!("Agent inspection failed: {error}"),
        },
        UiEvent::SupervisorAction(result) => {
            app.status =
                result.unwrap_or_else(|error| format!("Supervisor action failed: {error}"));
            app.cancel_armed = None;
        }
        UiEvent::TodoSnapshot(result) => match result {
            Ok(list) if list.revision != app.todo_revision || app.todos.is_empty() => {
                let selected = app.todos.get(app.selected_todo).map(|item| item.id);
                app.todo_revision = list.revision;
                app.todos = list.ordered().into_iter().cloned().collect();
                app.selected_todo = selected
                    .and_then(|id| app.todos.iter().position(|item| item.id == id))
                    .unwrap_or_else(|| app.selected_todo.min(app.todos.len().saturating_sub(1)));
            }
            Ok(_) => {}
            Err(error) => app.status = format!("Todo refresh failed: {error}"),
        },
        UiEvent::TodoAction(result) => {
            app.status = result.unwrap_or_else(|error| format!("Todo action failed: {error}"));
        }
        UiEvent::Models(result) => match result {
            Ok(models) => {
                app.models = models;
                app.selected_model = 0;
                app.status = format!("{} model(s) available", app.models.len());
            }
            Err(error) => {
                app.status = format!("Model discovery failed: {error}; Tab enters an ID manually")
            }
        },
    }
    Ok(())
}

// Keeping these explicit makes modal input routing auditable (especially PTY isolation).
#[allow(clippy::too_many_arguments)]
async fn handle_key(
    key: KeyEvent,
    app: &mut App,
    agent: &Arc<Agent>,
    store: &SessionStore,
    tx: &mpsc::UnboundedSender<UiEvent>,
    terminals: &dyn InteractiveTerminals,
    supervisor: Arc<dyn AgentSupervisor>,
    todos: Arc<TodoStore>,
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
    if app.model_picker {
        handle_model_key(key, app, agent, store, tx).await?;
        return Ok(());
    }
    if app.supervisor_mode.is_some() {
        handle_supervisor_key(key, app, tx, supervisor).await;
        return Ok(());
    }
    if app.todo_mode.is_some() {
        handle_todo_key(key, app, tx, todos).await;
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
                    agent.set_model(app.session.model.clone())?;
                    app.activity.clear();
                    app.streaming_response.clear();
                    app.scroll = 0;
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
            KeyCode::Char('a') => {
                app.supervisor_mode = Some(SupervisorMode::Tree);
                request_supervisor_tree(tx, supervisor);
            }
            KeyCode::Char('d') => {
                app.todo_mode = Some(TodoMode::List);
                request_todo_snapshot(tx, todos);
            }
            KeyCode::Char('m') if !app.is_running() => {
                app.model_picker = true;
                app.model_manual = false;
                app.model_filter = Composer::default();
                app.selected_model = 0;
                request_models(tx, agent.clone(), false);
            }
            KeyCode::Char('n') if !app.is_running() => {
                app.session =
                    Session::new(app.session.workspace.clone(), app.session.model.clone());
                app.activity.clear();
                app.streaming_response.clear();
                app.scroll = 0;
                app.status = "New session".into();
            }
            KeyCode::Char('b') if !app.is_running() => {
                app.session = store.branch(&app.session, None).await?;
                app.sessions = store.list().await?;
                app.streaming_response.clear();
                app.scroll = 0;
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
                if handle_command(&prompt, app, store, Some(agent.as_ref())).await? {
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
    if is_terminal_detach_key(key) {
        app.attached_terminal = None;
        app.terminal_snapshot = None;
        app.status = "Detached; terminal is still running".into();
    } else if let Some(bytes) = encode_terminal_key(key)
        && let Err(error) = terminals.write(id, bytes).await
    {
        app.status = format!("Terminal input failed: {error}");
    }
}

async fn handle_supervisor_key(
    key: KeyEvent,
    app: &mut App,
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
) {
    let Some(mode) = app.supervisor_mode else {
        return;
    };
    if let SupervisorMode::Message { target, follow_up } = mode {
        match key.code {
            KeyCode::Esc => {
                app.supervisor_input = Composer::default();
                app.supervisor_mode = Some(SupervisorMode::Inspect(target));
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                app.supervisor_input.insert('\n')
            }
            KeyCode::Enter => {
                let message = app.supervisor_input.take();
                if !message.trim().is_empty() {
                    request_supervisor_message(tx, supervisor, target, message, follow_up);
                    app.supervisor_mode = Some(SupervisorMode::Inspect(target));
                }
            }
            KeyCode::Char(character) => app.supervisor_input.insert(character),
            KeyCode::Backspace => app.supervisor_input.backspace(),
            KeyCode::Delete => app.supervisor_input.delete(),
            KeyCode::Home => app.supervisor_input.line_start(),
            KeyCode::End => app.supervisor_input.line_end(),
            KeyCode::Left if app.supervisor_input.cursor > 0 => {
                app.supervisor_input.cursor = app.supervisor_input.text
                    [..app.supervisor_input.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index);
            }
            KeyCode::Right => {
                if let Some(character) = app.supervisor_input.text[app.supervisor_input.cursor..]
                    .chars()
                    .next()
                {
                    app.supervisor_input.cursor += character.len_utf8();
                }
            }
            _ => {}
        }
        return;
    }
    let selected = match mode {
        SupervisorMode::Inspect(id) => Some(id),
        SupervisorMode::Tree => app.agents.get(app.selected_agent).map(|agent| agent.id),
        SupervisorMode::Message { .. } => unreachable!(),
    };
    match key.code {
        KeyCode::Esc => match mode {
            SupervisorMode::Inspect(_) => {
                app.supervisor_mode = Some(SupervisorMode::Tree);
                app.supervisor_scroll = 0;
            }
            SupervisorMode::Tree => app.supervisor_mode = None,
            SupervisorMode::Message { .. } => unreachable!(),
        },
        KeyCode::Up if mode == SupervisorMode::Tree => {
            app.selected_agent = app.selected_agent.saturating_sub(1);
        }
        KeyCode::Down if mode == SupervisorMode::Tree => {
            app.selected_agent = (app.selected_agent + 1).min(app.agents.len().saturating_sub(1));
        }
        KeyCode::Enter if mode == SupervisorMode::Tree => {
            if let Some(id) = selected {
                app.supervisor_mode = Some(SupervisorMode::Inspect(id));
                app.supervisor_scroll = 0;
                request_supervisor_inspect(tx, supervisor, id, None);
            }
        }
        KeyCode::PageUp if matches!(mode, SupervisorMode::Inspect(_)) => {
            app.supervisor_scroll = app.supervisor_scroll.saturating_add(8);
        }
        KeyCode::PageDown if matches!(mode, SupervisorMode::Inspect(_)) => {
            app.supervisor_scroll = app.supervisor_scroll.saturating_sub(8);
        }
        KeyCode::Char('r') => {
            request_supervisor_tree(tx, supervisor.clone());
            if let SupervisorMode::Inspect(id) = mode {
                request_supervisor_inspect(tx, supervisor, id, None);
            }
        }
        KeyCode::Char('m') if selected.is_some() => {
            app.supervisor_input = Composer::default();
            app.supervisor_mode = Some(SupervisorMode::Message {
                target: selected.unwrap(),
                follow_up: false,
            });
        }
        KeyCode::Char('f') if selected.is_some() => {
            app.supervisor_input = Composer::default();
            app.supervisor_mode = Some(SupervisorMode::Message {
                target: selected.unwrap(),
                follow_up: true,
            });
        }
        KeyCode::Char('c') if selected.is_some() => {
            let id = selected.unwrap();
            if app.cancel_armed == Some(id) {
                request_supervisor_cancel(tx, supervisor, id);
                app.cancel_armed = None;
            } else if app
                .agents
                .iter()
                .any(|agent| agent.id == id && agent.status.is_terminal())
            {
                app.status = "Completed agents cannot be cancelled".into();
            } else {
                app.cancel_armed = Some(id);
                app.status = "Press c again to cancel this agent; Esc returns".into();
            }
        }
        _ => app.cancel_armed = None,
    }
}

async fn handle_todo_key(
    key: KeyEvent,
    app: &mut App,
    tx: &mpsc::UnboundedSender<UiEvent>,
    store: Arc<TodoStore>,
) {
    let Some(mode) = app.todo_mode else { return };
    if let TodoMode::Input { target, action } = mode {
        match key.code {
            KeyCode::Esc => {
                app.todo_input = Composer::default();
                app.todo_mode =
                    target.map_or(Some(TodoMode::List), |id| Some(TodoMode::Inspect(id)));
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                app.todo_input.insert('\n')
            }
            KeyCode::Enter => {
                let value = app.todo_input.take();
                if !value.trim().is_empty()
                    || matches!(
                        action,
                        TodoInput::Block | TodoInput::Assign | TodoInput::Dependencies
                    )
                {
                    request_todo_input(tx, store, target, action, value);
                    app.todo_mode =
                        target.map_or(Some(TodoMode::List), |id| Some(TodoMode::Inspect(id)));
                }
            }
            KeyCode::Char(character) => app.todo_input.insert(character),
            KeyCode::Backspace => app.todo_input.backspace(),
            KeyCode::Delete => app.todo_input.delete(),
            KeyCode::Home => app.todo_input.line_start(),
            KeyCode::End => app.todo_input.line_end(),
            KeyCode::Left if app.todo_input.cursor > 0 => {
                app.todo_input.cursor = app.todo_input.text[..app.todo_input.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index);
            }
            KeyCode::Right => {
                if let Some(character) = app.todo_input.text[app.todo_input.cursor..].chars().next()
                {
                    app.todo_input.cursor += character.len_utf8();
                }
            }
            _ => {}
        }
        return;
    }
    let selected = match mode {
        TodoMode::List => app.todos.get(app.selected_todo).map(|item| item.id),
        TodoMode::Inspect(id) => Some(id),
        TodoMode::Input { .. } => unreachable!(),
    };
    match key.code {
        KeyCode::Esc => match mode {
            TodoMode::Inspect(_) => app.todo_mode = Some(TodoMode::List),
            TodoMode::List => app.todo_mode = None,
            TodoMode::Input { .. } => unreachable!(),
        },
        KeyCode::Up if mode == TodoMode::List => {
            app.selected_todo = app.selected_todo.saturating_sub(1)
        }
        KeyCode::Down if mode == TodoMode::List => {
            app.selected_todo = (app.selected_todo + 1).min(app.todos.len().saturating_sub(1));
        }
        KeyCode::Enter if mode == TodoMode::List => {
            if let Some(id) = selected {
                app.todo_mode = Some(TodoMode::Inspect(id));
            }
        }
        KeyCode::PageUp if matches!(mode, TodoMode::Inspect(_)) => {
            app.todo_scroll = app.todo_scroll.saturating_add(8)
        }
        KeyCode::PageDown if matches!(mode, TodoMode::Inspect(_)) => {
            app.todo_scroll = app.todo_scroll.saturating_sub(8)
        }
        KeyCode::Char('r') => request_todo_snapshot(tx, store),
        KeyCode::Char('n') => open_todo_input(app, None, TodoInput::Add, ""),
        KeyCode::Char('e') if selected.is_some() => {
            let item = app.todos.iter().find(|item| Some(item.id) == selected);
            let initial = item.map_or(String::new(), |item| {
                format!("{}\n{}", item.title, item.description)
            });
            open_todo_input(app, selected, TodoInput::Edit, &initial);
        }
        KeyCode::Char('b') if selected.is_some() => {
            open_todo_input(app, selected, TodoInput::Block, "")
        }
        KeyCode::Char('a') if selected.is_some() => {
            let initial = app
                .todos
                .iter()
                .find(|item| Some(item.id) == selected)
                .map(|item| {
                    item.assignees
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            open_todo_input(app, selected, TodoInput::Assign, &initial);
        }
        KeyCode::Char('d') if selected.is_some() => {
            let initial = app
                .todos
                .iter()
                .find(|item| Some(item.id) == selected)
                .map(|item| {
                    item.dependencies
                        .iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            open_todo_input(app, selected, TodoInput::Dependencies, &initial);
        }
        KeyCode::Char('p') if selected.is_some() => {
            open_todo_input(app, selected, TodoInput::Progress, "")
        }
        KeyCode::Char('o') if selected.is_some() => {
            open_todo_input(app, selected, TodoInput::Note, "")
        }
        KeyCode::Char('v') if selected.is_some() => {
            open_todo_input(app, selected, TodoInput::Evidence, "")
        }
        KeyCode::Char('u') if let Some(id) = selected => {
            request_todo_action(tx, store.clone(), async move {
                store
                    .set_blockers(id, Vec::new())
                    .await
                    .map(|_| "Blockers cleared".into())
            })
        }
        KeyCode::Char('t') if let Some(id) = selected => {
            if let Some(item) = app.todos.iter().find(|item| item.id == id) {
                let status = match item.status {
                    TodoStatus::Pending | TodoStatus::Blocked => TodoStatus::InProgress,
                    TodoStatus::InProgress => TodoStatus::Completed,
                    TodoStatus::Completed | TodoStatus::Cancelled => TodoStatus::Pending,
                };
                request_todo_action(tx, store.clone(), async move {
                    store
                        .set_status(id, status)
                        .await
                        .map(|_| format!("Todo is {}", todo_status_label(status)))
                });
            }
        }
        KeyCode::Char('c') if let Some(id) = selected => {
            request_todo_action(tx, store.clone(), async move {
                store
                    .set_status(id, TodoStatus::Cancelled)
                    .await
                    .map(|_| "Todo cancelled".into())
            })
        }
        KeyCode::Char('K') if let Some(id) = selected => reorder_todo(app, tx, store, id, -1),
        KeyCode::Char('J') if let Some(id) = selected => reorder_todo(app, tx, store, id, 1),
        KeyCode::Char('x') if let Some(id) = selected => {
            request_todo_action(tx, store.clone(), async move {
                store.archive(id).await.map(|_| "Todo archived".into())
            })
        }
        _ => {}
    }
}

fn filtered_models(app: &App) -> Vec<&ModelInfo> {
    let query = app.model_filter.text.trim().to_ascii_lowercase();
    app.models
        .iter()
        .filter(|model| {
            query.is_empty()
                || model.id.to_ascii_lowercase().contains(&query)
                || model.display_name.to_ascii_lowercase().contains(&query)
                || model.description.to_ascii_lowercase().contains(&query)
        })
        .collect()
}

async fn handle_model_key(
    key: KeyEvent,
    app: &mut App,
    agent: &Arc<Agent>,
    store: &SessionStore,
    tx: &mpsc::UnboundedSender<UiEvent>,
) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            app.model_picker = false;
            app.model_filter = Composer::default();
        }
        KeyCode::Tab => {
            app.model_manual = !app.model_manual;
            app.selected_model = 0;
            app.status = if app.model_manual {
                "Manual model ID · type an ID and press Enter".into()
            } else {
                "Model search".into()
            };
        }
        KeyCode::Up if !app.model_manual => {
            app.selected_model = app.selected_model.saturating_sub(1)
        }
        KeyCode::Down if !app.model_manual => {
            let count = filtered_models(app).len();
            app.selected_model = (app.selected_model + 1).min(count.saturating_sub(1));
        }
        KeyCode::Enter => {
            let selected = if app.model_manual {
                (!app.model_filter.text.trim().is_empty())
                    .then(|| app.model_filter.text.trim().to_owned())
            } else {
                filtered_models(app)
                    .get(app.selected_model)
                    .map(|model| model.id.clone())
            };
            if let Some(model) = selected {
                app.session.switch_model(&model)?;
                agent.set_model(&model)?;
                store.save(&mut app.session).await?;
                app.status = format!("Model switched to {model}");
                app.model_picker = false;
                app.model_filter = Composer::default();
            }
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            request_models(tx, agent.clone(), true)
        }
        KeyCode::Char(character) => {
            app.model_filter.insert(character);
            app.selected_model = 0;
        }
        KeyCode::Backspace => {
            app.model_filter.backspace();
            app.selected_model = 0;
        }
        KeyCode::Delete => app.model_filter.delete(),
        KeyCode::Left if app.model_filter.cursor > 0 => {
            app.model_filter.cursor = app.model_filter.text[..app.model_filter.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(index, _)| index);
        }
        KeyCode::Right => {
            if let Some(character) = app.model_filter.text[app.model_filter.cursor..]
                .chars()
                .next()
            {
                app.model_filter.cursor += character.len_utf8();
            }
        }
        _ => {}
    }
    Ok(())
}

fn request_models(tx: &mpsc::UnboundedSender<UiEvent>, agent: Arc<Agent>, refresh: bool) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = agent
            .models(refresh)
            .await
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::Models(result));
    });
}

fn open_todo_input(app: &mut App, target: Option<TodoId>, action: TodoInput, initial: &str) {
    app.todo_input = Composer::default();
    app.todo_input.insert_str(initial);
    app.todo_mode = Some(TodoMode::Input { target, action });
}

fn request_todo_snapshot(tx: &mpsc::UnboundedSender<UiEvent>, store: Arc<TodoStore>) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(UiEvent::TodoSnapshot(
            store.snapshot().await.map_err(|error| error.to_string()),
        ));
    });
}

fn request_todo_action<F>(tx: &mpsc::UnboundedSender<UiEvent>, store: Arc<TodoStore>, future: F)
where
    F: std::future::Future<Output = anyhow::Result<String>> + Send + 'static,
{
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = future.await.map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::TodoAction(result));
        let _ = tx.send(UiEvent::TodoSnapshot(
            store.snapshot().await.map_err(|error| error.to_string()),
        ));
    });
}

fn request_todo_input(
    tx: &mpsc::UnboundedSender<UiEvent>,
    store: Arc<TodoStore>,
    target: Option<TodoId>,
    action: TodoInput,
    value: String,
) {
    let operation_store = store.clone();
    request_todo_action(tx, store, async move {
        match (action, target) {
            (TodoInput::Add, None) => {
                let (title, description) = value.split_once('\n').unwrap_or((&value, ""));
                operation_store
                    .create(NewTodo {
                        title: title.to_owned(),
                        description: description.to_owned(),
                        priority: Priority::Normal,
                        order: None,
                        assignees: BTreeSet::new(),
                    })
                    .await
                    .map(|item| format!("Added {}", item.title))
            }
            (TodoInput::Edit, Some(id)) => {
                let (title, description) = value.split_once('\n').unwrap_or((&value, ""));
                operation_store
                    .edit(
                        id,
                        Some(title.to_owned()),
                        Some(description.to_owned()),
                        None,
                    )
                    .await
                    .map(|_| "Todo updated".into())
            }
            (TodoInput::Block, Some(id)) => operation_store
                .set_blockers(id, split_values(&value))
                .await
                .map(|_| "Blockers updated".into()),
            (TodoInput::Assign, Some(id)) => operation_store
                .assign(id, split_values(&value).into_iter().collect())
                .await
                .map(|_| "Assignees updated".into()),
            (TodoInput::Dependencies, Some(id)) => {
                let desired = parse_todo_ids(&value)?;
                let snapshot = operation_store.snapshot().await?;
                let current = snapshot
                    .items
                    .get(&id)
                    .ok_or_else(|| anyhow::anyhow!("unknown todo"))?
                    .dependencies
                    .clone();
                let add = desired.difference(&current).copied().collect();
                let remove = current.difference(&desired).copied().collect();
                operation_store.update_dependencies(id, add, remove).await?;
                Ok("Dependencies updated".into())
            }
            (TodoInput::Progress, Some(id)) => operation_store
                .append_note(id, EntryKind::Progress, value, Some("user".into()))
                .await
                .map(|_| "Progress added".into()),
            (TodoInput::Note, Some(id)) => operation_store
                .append_note(id, EntryKind::Note, value, Some("user".into()))
                .await
                .map(|_| "Note added".into()),
            (TodoInput::Evidence, Some(id)) => operation_store
                .append_note(id, EntryKind::Evidence, value, Some("user".into()))
                .await
                .map(|_| "Evidence added".into()),
            _ => anyhow::bail!("invalid todo action"),
        }
    });
}

fn split_values(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_todo_ids(value: &str) -> anyhow::Result<BTreeSet<TodoId>> {
    split_values(value)
        .into_iter()
        .map(|value| {
            uuid::Uuid::parse_str(&value)
                .map(TodoId)
                .map_err(|_| anyhow::anyhow!("invalid todo UUID: {value}"))
        })
        .collect()
}

fn reorder_todo(
    app: &App,
    tx: &mpsc::UnboundedSender<UiEvent>,
    store: Arc<TodoStore>,
    id: TodoId,
    direction: isize,
) {
    let Some(index) = app.todos.iter().position(|item| item.id == id) else {
        return;
    };
    let other = index
        .saturating_add_signed(direction)
        .min(app.todos.len().saturating_sub(1));
    if other == index {
        return;
    }
    let original_order = app.todos[index].order;
    let other_id = app.todos[other].id;
    let other_order = app.todos[other].order;
    request_todo_action(tx, store.clone(), async move {
        store.reorder(id, other_order).await?;
        store.reorder(other_id, original_order).await?;
        Ok("Todo reordered".into())
    });
}

fn request_supervisor_tree(
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = supervisor.tree().await.map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::SupervisorTree(result));
    });
}

fn request_supervisor_inspect(
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
    id: AgentId,
    after: Option<u64>,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = supervisor
            .inspect(id, after)
            .await
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::SupervisorInspect(id, result));
    });
}

fn request_supervisor_message(
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
    id: AgentId,
    message: String,
    follow_up: bool,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = if follow_up {
            supervisor
                .follow_up(id, message)
                .await
                .map(|child| format!("Follow-up queued as {child}"))
        } else {
            supervisor
                .send_message(id, message)
                .await
                .map(|()| "Message queued".into())
        }
        .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::SupervisorAction(result));
    });
}

fn request_supervisor_cancel(
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
    id: AgentId,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = supervisor
            .cancel(id)
            .await
            .map(|()| "Cancellation requested".into())
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::SupervisorAction(result));
    });
}

fn flatten_agent_tree(agents: Vec<AgentView>) -> Vec<AgentView> {
    fn visit(
        id: AgentId,
        all: &[AgentView],
        visited: &mut std::collections::HashSet<AgentId>,
        output: &mut Vec<AgentView>,
    ) {
        if !visited.insert(id) {
            return;
        }
        if let Some(agent) = all.iter().find(|agent| agent.id == id) {
            output.push(agent.clone());
            for child in all.iter().filter(|child| child.parent == Some(id)) {
                visit(child.id, all, visited, output);
            }
        }
    }
    let mut output = Vec::with_capacity(agents.len());
    let mut visited = std::collections::HashSet::new();
    for root in agents.iter().filter(|agent| {
        agent.parent.is_none()
            || !agents
                .iter()
                .any(|candidate| Some(candidate.id) == agent.parent)
    }) {
        visit(root.id, &agents, &mut visited, &mut output);
    }
    for agent in &agents {
        visit(agent.id, &agents, &mut visited, &mut output);
    }
    output
}

fn is_terminal_detach_key(key: KeyEvent) -> bool {
    (key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('t' | ']')))
        || key.code == KeyCode::Char('\u{1d}')
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
    if app.model_picker {
        draw_model_picker(frame, area, app);
        return;
    }
    if app.supervisor_mode.is_some() {
        draw_supervisor(frame, area, app);
        if let Some(approval) = &app.approval {
            draw_approval(frame, area, approval);
        }
        return;
    }
    if app.todo_mode.is_some() {
        draw_todos(frame, area, app);
        if let Some(approval) = &app.approval {
            draw_approval(frame, area, approval);
        }
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
                "  {title} · {} · {} · {}",
                app.session.model,
                app.provider_label,
                app.session.workspace.display()
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    let transcript = transcript(app, viewport_width_for(chunks[1]));
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
            "{}  │  ^D todos  ^A agents  ^M models  ^T terminals  ^S sessions  ^N new  ^B branch  ^K compact  ^E export",
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

fn draw_todos(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let input = matches!(app.todo_mode, Some(TodoMode::Input { .. }));
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(if input { 5 } else { 0 }),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " HELM TODOS ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {} active · revision {}",
                app.todos.len(),
                app.todo_revision
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    match app.todo_mode {
        Some(TodoMode::List) => draw_todo_list(frame, chunks[1], app),
        Some(TodoMode::Inspect(id))
        | Some(TodoMode::Input {
            target: Some(id), ..
        }) => draw_todo_inspect(frame, chunks[1], app, id),
        Some(TodoMode::Input { target: None, .. }) => draw_todo_list(frame, chunks[1], app),
        None => {}
    }
    if let Some(TodoMode::Input { action, .. }) = app.todo_mode {
        let title = match action {
            TodoInput::Add => " Add · title ",
            TodoInput::Edit => " Edit · title then description on next line ",
            TodoInput::Block => " Blockers · comma/newline separated ",
            TodoInput::Assign => " Assignees · comma/newline separated ",
            TodoInput::Dependencies => " Prerequisite UUIDs · comma/newline separated ",
            TodoInput::Progress => " Progress update ",
            TodoInput::Note => " Note ",
            TodoInput::Evidence => " Evidence ",
        };
        frame.render_widget(
            Paragraph::new(app.todo_input.text.as_str())
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(format!(
                            "{title}· Enter save · Alt+Enter newline · Esc return "
                        ))
                        .borders(Borders::ALL),
                ),
            chunks[2],
        );
        let (row, column) = cursor_position(
            &app.todo_input.text[..app.todo_input.cursor],
            chunks[2].width.saturating_sub(2).max(1),
        );
        frame.set_cursor_position((
            (chunks[2].x + 1 + column).min(chunks[2].right().saturating_sub(2)),
            (chunks[2].y + 1 + row).min(chunks[2].bottom().saturating_sub(2)),
        ));
    }
    let help = if area.width < 60 {
        "↑↓ Enter · n new · t next · Esc return"
    } else {
        "↑↓ · Enter inspect · n new · e edit · t next · b/u block · a assign · d deps · p progress · o note · v evidence · J/K reorder · c cancel · x archive · r reload · Esc"
    };
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::Gray)),
        chunks[3],
    );
}

fn draw_model_picker(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " HELM MODELS ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  current: {}", app.session.model)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(app.model_filter.text.as_str()).block(
            Block::default()
                .title(if app.model_manual {
                    " Manual model ID "
                } else {
                    " Search models "
                })
                .borders(Borders::ALL),
        ),
        chunks[1],
    );
    let models = filtered_models(app);
    let visible = chunks[2].height.saturating_sub(2).max(1) as usize;
    let start = app
        .selected_model
        .saturating_sub(visible.saturating_sub(1))
        .min(models.len().saturating_sub(visible));
    let items = if app.model_manual {
        vec![ListItem::new("Press Enter to use this exact model ID")]
    } else if models.is_empty() {
        vec![ListItem::new(
            "No matching discovered models · Tab enters an ID manually",
        )]
    } else {
        models
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, model)| {
                ListItem::new(format!(
                    "{}{} {} · {}",
                    if index == app.selected_model {
                        "▶ "
                    } else {
                        "  "
                    },
                    if model.is_default { "★" } else { " " },
                    model.display_name,
                    model.id
                ))
            })
            .collect()
    };
    frame.render_widget(
        List::new(items).block(Block::default().title(" Available ").borders(Borders::ALL)),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new(if area.width < 60 {
            "type filter · ↑↓ Enter · Tab manual · Esc"
        } else {
            "type to search · ↑↓ select · Enter switch · Tab manual ID · Ctrl+R refresh · Esc return"
        })
        .style(Style::default().fg(Color::Gray)),
        chunks[3],
    );
    frame.set_cursor_position((
        (chunks[1].x + 1 + app.model_filter.cursor as u16).min(chunks[1].right().saturating_sub(2)),
        chunks[1].y + 1,
    ));
}

fn draw_todo_list(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let start = app
        .selected_todo
        .saturating_sub(visible.saturating_sub(1))
        .min(app.todos.len().saturating_sub(visible));
    let items: Vec<_> = app
        .todos
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, item)| {
            let marker = if index == app.selected_todo {
                "▶"
            } else {
                " "
            };
            let blockers = if item.blockers.is_empty() {
                String::new()
            } else {
                format!(" · {} blocker(s)", item.blockers.len())
            };
            ListItem::new(format!(
                "{marker} [{}|{}] {}{}",
                todo_status_label(item.status),
                priority_label(item.priority),
                one_line(&item.title, 80),
                blockers
            ))
        })
        .collect();
    frame.render_widget(
        List::new(if items.is_empty() {
            vec![ListItem::new("No active todos · press n to add one")]
        } else {
            items
        })
        .block(
            Block::default()
                .title(" Workspace plan ")
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn draw_todo_inspect(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App, id: TodoId) {
    let Some(item) = app.todos.iter().find(|item| item.id == id) else {
        frame.render_widget(
            Paragraph::new("Todo was archived or removed; Esc returns to the list.")
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    };
    let mut lines = vec![
        Line::raw(format!(
            "{} [{} · {}]",
            item.title,
            todo_status_label(item.status),
            priority_label(item.priority)
        )),
        Line::raw(format!("ID: {} · order {}", item.id.0, item.order)),
        Line::raw(format!(
            "Assignees: {}",
            if item.assignees.is_empty() {
                "—".into()
            } else {
                item.assignees
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )),
        Line::raw(format!(
            "Dependencies: {}",
            if item.dependencies.is_empty() {
                "—".into()
            } else {
                item.dependencies
                    .iter()
                    .map(|id| id.0.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )),
        Line::raw(""),
        Line::raw(item.description.clone()),
    ];
    for blocker in &item.blockers {
        lines.push(Line::styled(
            format!("BLOCKED: {blocker}"),
            Style::default().fg(Color::Red),
        ));
    }
    for (label, entries) in [
        ("Progress", &item.progress),
        ("Evidence", &item.evidence),
        ("Notes", &item.notes),
    ] {
        if !entries.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                label,
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        lines.extend(entries.iter().map(|entry| {
            Line::raw(format!(
                "{} {}{}",
                entry.at.format("%Y-%m-%d %H:%M"),
                entry
                    .author
                    .as_deref()
                    .map_or(String::new(), |author| format!("{author}: ")),
                entry.text
            ))
        }));
    }
    let bottom = lines
        .len()
        .saturating_sub(area.height.saturating_sub(2) as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((bottom.saturating_sub(app.todo_scroll), 0))
            .block(
                Block::default()
                    .title(" Todo detail ")
                    .borders(Borders::ALL),
            ),
        area,
    );
}

fn todo_status_label(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Pending => "pending",
        TodoStatus::InProgress => "in progress",
        TodoStatus::Blocked => "blocked",
        TodoStatus::Completed => "completed",
        TodoStatus::Cancelled => "cancelled",
    }
}
fn priority_label(priority: Priority) -> &'static str {
    match priority {
        Priority::Low => "low",
        Priority::Normal => "normal",
        Priority::High => "high",
        Priority::Critical => "critical",
    }
}

fn draw_supervisor(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(
            if matches!(app.supervisor_mode, Some(SupervisorMode::Message { .. })) {
                5
            } else {
                0
            },
        ),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " HELM AGENTS ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {} supervised", app.agents.len())),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    match app.supervisor_mode {
        Some(SupervisorMode::Tree) => draw_agent_tree(frame, chunks[1], app),
        Some(SupervisorMode::Inspect(id)) | Some(SupervisorMode::Message { target: id, .. }) => {
            draw_agent_inspection(frame, chunks[1], app, id)
        }
        None => {}
    }
    if let Some(SupervisorMode::Message { follow_up, .. }) = app.supervisor_mode {
        frame.render_widget(
            Paragraph::new(app.supervisor_input.text.as_str())
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(if follow_up {
                            " Follow-up · Enter send · Alt+Enter newline · Esc return "
                        } else {
                            " Message · Enter send · Alt+Enter newline · Esc return "
                        })
                        .borders(Borders::ALL),
                ),
            chunks[2],
        );
        let (row, column) = cursor_position(
            &app.supervisor_input.text[..app.supervisor_input.cursor],
            chunks[2].width.saturating_sub(2),
        );
        frame.set_cursor_position((
            (chunks[2].x + 1 + column).min(chunks[2].right().saturating_sub(2)),
            (chunks[2].y + 1 + row).min(chunks[2].bottom().saturating_sub(2)),
        ));
    }
    let help = match (app.supervisor_mode, area.width < 60) {
        (Some(SupervisorMode::Tree), true) => "↑↓ Enter · Esc return",
        (Some(SupervisorMode::Inspect(_)), true) => "PgUp/PgDn · m/f send · Esc tree",
        (Some(SupervisorMode::Message { .. }), true) => "Enter send · Esc return",
        (Some(SupervisorMode::Tree), false) => {
            "↑↓ select · Enter inspect · m message · f follow-up · c,c cancel · r refresh · Esc return"
        }
        (Some(SupervisorMode::Inspect(_)), false) => {
            "m message · f follow-up · c,c cancel · PgUp/PgDn progress · r refresh · Esc tree"
        }
        (Some(SupervisorMode::Message { .. }), false) => {
            "Direct supervisor message; content is sent only to the selected agent"
        }
        (None, _) => "",
    };
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::Gray)),
        chunks[3],
    );
}

fn draw_agent_tree(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let start = app
        .selected_agent
        .saturating_sub(visible.saturating_sub(1))
        .min(app.agents.len().saturating_sub(visible));
    let items: Vec<_> = app
        .agents
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, agent)| {
            let depth = agent_depth(agent, &app.agents).min(8);
            let selected = if index == app.selected_agent {
                "▶"
            } else {
                " "
            };
            let progress = agent.recent_progress.last().map_or("", String::as_str);
            ListItem::new(format!(
                "{selected} {}{} [{}] {} · {} · {}",
                "  ".repeat(depth),
                short_id(agent.id),
                status_label(&agent.status),
                elapsed_label(agent.elapsed),
                one_line(&agent.task, 60),
                one_line(progress, 50)
            ))
        })
        .collect();
    let items = if items.is_empty() {
        vec![ListItem::new("No supervised agents")]
    } else {
        items
    };
    frame.render_widget(
        List::new(items).block(Block::default().title(" Agent tree ").borders(Borders::ALL)),
        area,
    );
}

fn draw_agent_inspection(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App, id: AgentId) {
    let Some(agent) = app.agents.iter().find(|agent| agent.id == id) else {
        frame.render_widget(
            Paragraph::new("Agent is no longer present; Esc returns to the tree.")
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    };
    let mut lines = vec![
        Line::raw(format!("Task: {}", agent.task)),
        Line::raw(format!(
            "State: {} · elapsed {}",
            status_label(&agent.status),
            elapsed_label(agent.elapsed)
        )),
        Line::raw(format!(
            "Parent: {}",
            agent
                .parent
                .map_or_else(|| "root".into(), |id| id.to_string())
        )),
        Line::raw(format!(
            "Worktree: {}",
            agent
                .worktree
                .as_ref()
                .map_or_else(|| "—".into(), |path| path.display().to_string())
        )),
        Line::raw(""),
        Line::styled(
            "Recent progress",
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ];
    lines.extend(
        agent
            .recent_progress
            .iter()
            .map(|progress| Line::raw(format!("• {progress}"))),
    );
    lines.extend(app.inspected_events.iter().map(|event| {
        Line::raw(format!(
            "{} #{} {}",
            event.timestamp.format("%H:%M:%S"),
            event.sequence,
            supervision_event_label(&event.kind)
        ))
    }));
    if let Some(result) = &agent.result {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Result",
            Style::default().add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(result.clone()));
    }
    if let Some(error) = &agent.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("Error: {error}"),
            Style::default().fg(Color::Red),
        ));
    }
    let height = area.height.saturating_sub(2) as usize;
    let bottom = lines.len().saturating_sub(height) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((bottom.saturating_sub(app.supervisor_scroll), 0))
            .block(
                Block::default()
                    .title(format!(" Agent {} ", short_id(id)))
                    .borders(Borders::ALL),
            ),
        area,
    );
}

fn agent_depth(agent: &AgentView, agents: &[AgentView]) -> usize {
    let mut parent = agent.parent;
    let mut depth = 0;
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = parent {
        if !seen.insert(id) {
            break;
        }
        depth += 1;
        parent = agents
            .iter()
            .find(|agent| agent.id == id)
            .and_then(|agent| agent.parent);
    }
    depth
}

fn short_id(id: AgentId) -> String {
    id.to_string().chars().take(8).collect()
}
fn status_label(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Queued => "queued",
        AgentStatus::Running => "running",
        AgentStatus::Waiting => "waiting",
        AgentStatus::Completed => "completed",
        AgentStatus::Failed => "failed",
        AgentStatus::TimedOut => "timed out",
        AgentStatus::Interrupted => "interrupted",
        AgentStatus::Cancelled => "cancelled",
    }
}
fn elapsed_label(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn supervision_event_label(kind: &SupervisionEventKind) -> String {
    match kind {
        SupervisionEventKind::Queued => "queued".into(),
        SupervisionEventKind::Started => "started".into(),
        SupervisionEventKind::Progress { text } => text.clone(),
        SupervisionEventKind::MessageQueued => "message queued".into(),
        SupervisionEventKind::FollowUpQueued { child } => child.map_or_else(
            || "follow-up queued".into(),
            |id| format!("follow-up queued as {}", short_id(id)),
        ),
        SupervisionEventKind::Completed { result } => {
            format!("completed: {}", one_line(result, 120))
        }
        SupervisionEventKind::Failed { error } => format!("failed: {}", one_line(error, 120)),
        SupervisionEventKind::TimedOut { error } => {
            format!("timed out: {}", one_line(error, 120))
        }
        SupervisionEventKind::Interrupted { reason } => {
            format!("interrupted: {}", one_line(reason, 120))
        }
        SupervisionEventKind::Cancelled => "cancelled".into(),
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
            " HELM TERMINAL · {title} · {state}{} · Ctrl+T detach (process keeps running)",
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

fn transcript(app: &App, width: usize) -> Text<'static> {
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
        if message.role == Role::Assistant {
            lines.extend(
                render_markdown(
                    &message.content,
                    RenderOptions {
                        width,
                        theme: app.markdown_theme,
                        syntax_highlighting: app.markdown_syntax_highlighting,
                        ..Default::default()
                    },
                )
                .lines,
            );
        } else {
            lines.extend(
                message
                    .content
                    .lines()
                    .map(|line| Line::raw(display_safe(line))),
            );
        }
        lines.push(Line::raw(""));
    }
    if !app.streaming_response.is_empty() {
        lines.push(Line::from(Span::styled(
            "helm · streaming",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.extend(
            render_markdown(
                &app.streaming_response,
                RenderOptions {
                    width,
                    theme: app.markdown_theme,
                    syntax_highlighting: app.markdown_syntax_highlighting,
                    ..Default::default()
                },
            )
            .lines,
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
                .map(|line| Line::styled(display_safe(line), Style::default().fg(Color::DarkGray))),
        );
    }
    Text::from(lines)
}

fn viewport_width_for(area: Rect) -> usize {
    area.width.saturating_sub(2).max(1) as usize
}

fn markdown_theme() -> MarkdownTheme {
    markdown_theme_for(std::env::var_os("NO_COLOR").is_some())
}

fn markdown_theme_for(no_color: bool) -> MarkdownTheme {
    if !no_color {
        return MarkdownTheme::default();
    }
    MarkdownTheme {
        text: Color::Reset,
        heading: Color::Reset,
        link: Color::Reset,
        code: Color::Reset,
        code_background: Color::Reset,
        quote: Color::Reset,
        rule: Color::Reset,
        table_header: Color::Reset,
        warning: Color::Reset,
    }
}

fn transcript_height(app: &App, width: usize) -> usize {
    transcript(app, width)
        .lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width.max(1)))
        .sum()
}

fn preserve_manual_anchor(app: &mut App, previous_height: usize) {
    if app.scroll == 0 {
        return;
    }
    let next = transcript_height(app, app.conversation_width);
    if next >= previous_height {
        app.scroll = app
            .scroll
            .saturating_add((next - previous_height).min(u16::MAX as usize) as u16);
    } else {
        app.scroll = app
            .scroll
            .saturating_sub((previous_height - next).min(u16::MAX as usize) as u16);
    }
}

fn resize_conversation(app: &mut App, width: usize, height: usize) {
    let previous = transcript_height(app, app.conversation_width);
    let previous_bottom = previous.saturating_sub(app.conversation_height);
    let anchored_top = previous_bottom.saturating_sub(app.scroll as usize);
    app.conversation_width = width.max(1);
    app.conversation_height = height.max(1);
    if app.scroll > 0 {
        let next_bottom =
            transcript_height(app, app.conversation_width).saturating_sub(app.conversation_height);
        app.scroll = next_bottom
            .saturating_sub(anchored_top)
            .min(u16::MAX as usize) as u16;
    }
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
            display_safe(&approval.action),
            display_safe(&approval.target),
            approval.id,
            display_safe(&approval.reason)
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

fn display_safe(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' | '\t' => character,
            _ if character.is_control() => '�',
            _ => character,
        })
        .collect()
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

async fn handle_command(
    command: &str,
    app: &mut App,
    store: &SessionStore,
    agent: Option<&Agent>,
) -> Result<bool> {
    let Some(command) = command.strip_prefix('/') else {
        return Ok(false);
    };
    let (name, argument) = command.split_once(' ').unwrap_or((command, ""));
    match name {
        "help" => {
            app.status =
                "/tools · /model [ID] · /name TITLE · /branch [TITLE] · /compact [KEEP] · /export [PATH] · /clear confirm"
                    .into()
        }
        "tools" => {
            if let Some(agent) = agent {
                let tools = agent.tool_inventory();
                app.activity.push("Available Helm tool calls:".into());
                app.activity.extend(
                    tools
                        .iter()
                        .map(|tool| format!("  {} — {}", tool.name, tool.description)),
                );
                app.status = format!("{} tool call(s) available", tools.len());
            } else {
                app.status = "Tool inventory unavailable while runtime is starting".into();
            }
        }
        "model" if argument.trim().is_empty() => {
            app.status = format!("Current model: {} · Ctrl+M opens the model picker", app.session.model)
        }
        "model" => {
            let model = argument.trim();
            app.session.switch_model(model)?;
            if let Some(agent) = agent {
                agent.set_model(model)?;
            }
            store.save(&mut app.session).await?;
            app.status = format!("Model switched to {model}");
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
            app.streaming_response.clear();
            app.scroll = 0;
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
            app.scroll = 0;
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
            app.streaming_response.clear();
            app.scroll = 0;
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
    use crate::supervision::SupervisionError;
    use crate::terminal::{TerminalCell, TerminalError, TerminalState};
    use chrono::Utc;
    use std::{sync::Mutex, time::Duration};
    use uuid::Uuid;

    struct FakeSupervisor {
        agents: Vec<AgentView>,
        events: Vec<SupervisionEvent>,
        actions: Mutex<Vec<String>>,
        sender: tokio::sync::broadcast::Sender<SupervisionEvent>,
    }

    impl FakeSupervisor {
        fn new(agents: Vec<AgentView>) -> Self {
            let (sender, _) = tokio::sync::broadcast::channel(16);
            Self {
                agents,
                events: Vec::new(),
                actions: Mutex::new(Vec::new()),
                sender,
            }
        }
    }

    #[async_trait]
    impl AgentSupervisor for FakeSupervisor {
        async fn tree(&self) -> Result<Vec<AgentView>, SupervisionError> {
            Ok(self.agents.clone())
        }

        async fn inspect(
            &self,
            id: AgentId,
            after: Option<u64>,
        ) -> Result<Vec<SupervisionEvent>, SupervisionError> {
            if !self.agents.iter().any(|agent| agent.id == id) {
                return Err(SupervisionError::NotFound(id));
            }
            Ok(self
                .events
                .iter()
                .filter(|event| {
                    event.agent_id == id && after.is_none_or(|seq| event.sequence > seq)
                })
                .cloned()
                .collect())
        }

        async fn send_message(&self, id: AgentId, text: String) -> Result<(), SupervisionError> {
            self.actions
                .lock()
                .unwrap()
                .push(format!("message:{id}:{text}"));
            Ok(())
        }

        async fn follow_up(&self, id: AgentId, text: String) -> Result<AgentId, SupervisionError> {
            self.actions
                .lock()
                .unwrap()
                .push(format!("follow:{id}:{text}"));
            Ok(AgentId(Uuid::new_v4()))
        }

        async fn cancel(&self, id: AgentId) -> Result<(), SupervisionError> {
            self.actions.lock().unwrap().push(format!("cancel:{id}"));
            Ok(())
        }

        fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SupervisionEvent> {
            self.sender.subscribe()
        }
    }

    fn agent(id: AgentId, parent: Option<AgentId>, task: &str) -> AgentView {
        AgentView {
            id,
            parent,
            task: task.into(),
            status: AgentStatus::Running,
            started_at: Some(Utc::now()),
            elapsed: Duration::from_secs(65),
            worktree: Some(PathBuf::from("/tmp/worktree")),
            recent_progress: vec!["running checks".into()],
            result: None,
            error: None,
        }
    }

    fn todo_store(directory: &tempfile::TempDir) -> Arc<TodoStore> {
        Arc::new(TodoStore::new(
            directory.path().join("todos.json"),
            crate::todo::TodoScope {
                workspace: directory.path().into(),
                session_id: None,
            },
        ))
    }

    async fn receive_todo_action(rx: &mut mpsc::UnboundedReceiver<UiEvent>) {
        assert!(matches!(rx.recv().await, Some(UiEvent::TodoAction(Ok(_)))));
        assert!(matches!(
            rx.recv().await,
            Some(UiEvent::TodoSnapshot(Ok(_)))
        ));
    }

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
    fn agent_tree_is_parent_first_and_cycle_safe() {
        let root = AgentId(Uuid::new_v4());
        let child = AgentId(Uuid::new_v4());
        let orphan = AgentId(Uuid::new_v4());
        let cycle_a = AgentId(Uuid::new_v4());
        let cycle_b = AgentId(Uuid::new_v4());
        let flattened = flatten_agent_tree(vec![
            agent(child, Some(root), "child"),
            agent(cycle_a, Some(cycle_b), "cycle a"),
            agent(root, None, "root"),
            agent(orphan, Some(AgentId(Uuid::new_v4())), "orphan"),
            agent(cycle_b, Some(cycle_a), "cycle b"),
        ]);
        assert_eq!(flattened.len(), 5);
        assert!(
            flattened.iter().position(|item| item.id == root)
                < flattened.iter().position(|item| item.id == child)
        );
        assert_eq!(
            flattened.iter().filter(|item| item.id == cycle_a).count(),
            1
        );
        assert_eq!(
            flattened.iter().filter(|item| item.id == cycle_b).count(),
            1
        );
    }

    #[test]
    fn interrupted_agents_are_terminal_and_render_distinctly() {
        assert!(AgentStatus::Interrupted.is_terminal());
        assert_eq!(status_label(&AgentStatus::Interrupted), "interrupted");
        assert_eq!(
            supervision_event_label(&SupervisionEventKind::Interrupted {
                reason: "operator stopped work".into(),
            }),
            "interrupted: operator stopped work"
        );
    }

    #[tokio::test]
    async fn supervisor_actions_are_dispatched_without_entering_chat_transcript() {
        let directory = tempfile::tempdir().unwrap();
        let id = AgentId(Uuid::new_v4());
        let supervisor = Arc::new(FakeSupervisor::new(vec![agent(id, None, "research")]));
        let mut app = App::new(
            Session::new(directory.path().into(), "test-model".into()),
            Vec::new(),
        );
        app.agents = supervisor.agents.clone();
        app.supervisor_mode = Some(SupervisorMode::Tree);
        let (tx, mut rx) = mpsc::unbounded_channel();

        handle_supervisor_key(
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
            &mut app,
            &tx,
            supervisor.clone(),
        )
        .await;
        app.supervisor_input.insert_str("status please");
        handle_supervisor_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
            &tx,
            supervisor.clone(),
        )
        .await;
        assert!(matches!(
            rx.recv().await,
            Some(UiEvent::SupervisorAction(Ok(_)))
        ));
        assert!(app.session.messages.is_empty());
        assert_eq!(
            supervisor.actions.lock().unwrap().as_slice(),
            &[format!("message:{id}:status please")]
        );

        handle_supervisor_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            &mut app,
            &tx,
            supervisor.clone(),
        )
        .await;
        assert!(supervisor.actions.lock().unwrap().len() == 1);
        handle_supervisor_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            &mut app,
            &tx,
            supervisor.clone(),
        )
        .await;
        assert!(matches!(
            rx.recv().await,
            Some(UiEvent::SupervisorAction(Ok(_)))
        ));
        assert_eq!(supervisor.actions.lock().unwrap().len(), 2);
    }

    #[test]
    fn agent_view_renders_at_minimum_supported_size_and_keeps_selection_visible() {
        let mut app = App::new(
            Session::new(PathBuf::from("/tmp"), "test-model".into()),
            Vec::new(),
        );
        app.supervisor_mode = Some(SupervisorMode::Tree);
        app.agents = (0..20)
            .map(|index| agent(AgentId(Uuid::new_v4()), None, &format!("task {index}")))
            .collect();
        app.selected_agent = 19;
        let selected_id = short_id(app.agents[19].id);
        let backend = ratatui::backend::TestBackend::new(32, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains(&selected_id));
        assert!(rendered.contains("Esc return"));
    }

    #[test]
    fn model_picker_filters_and_renders_manual_fallback() {
        let mut app = App::new(
            Session::new(PathBuf::from("/tmp"), "gpt-current".into()),
            Vec::new(),
        );
        app.model_picker = true;
        app.models = vec![
            ModelInfo::minimal("gpt-fast"),
            ModelInfo::minimal("gpt-careful"),
        ];
        app.model_filter.insert_str("care");
        assert_eq!(filtered_models(&app)[0].id, "gpt-careful");
        let backend = ratatui::backend::TestBackend::new(60, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("gpt-careful"));
        assert!(rendered.contains("current: gpt-current"));
    }

    #[tokio::test]
    async fn todo_composer_actions_are_async_and_do_not_enter_chat() {
        let directory = tempfile::tempdir().unwrap();
        let store = todo_store(&directory);
        let mut app = App::new(
            Session::new(directory.path().into(), "test-model".into()),
            Vec::new(),
        );
        app.todo_mode = Some(TodoMode::List);
        let (tx, mut rx) = mpsc::unbounded_channel();
        handle_todo_key(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            &mut app,
            &tx,
            store.clone(),
        )
        .await;
        app.todo_input.insert_str("Ship it\nRelease evidence");
        handle_todo_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
            &tx,
            store.clone(),
        )
        .await;
        assert!(matches!(rx.recv().await, Some(UiEvent::TodoAction(Ok(_)))));
        let Some(UiEvent::TodoSnapshot(Ok(snapshot))) = rx.recv().await else {
            panic!("todo action must trigger a live snapshot refresh")
        };
        let item = snapshot.ordered()[0];
        assert_eq!(item.title, "Ship it");
        assert_eq!(item.description, "Release evidence");
        assert!(app.session.messages.is_empty());
    }

    #[tokio::test]
    async fn todo_view_renders_selected_item_and_complete_inspection() {
        let directory = tempfile::tempdir().unwrap();
        let store = todo_store(&directory);
        for index in 0..20 {
            store
                .create(NewTodo {
                    title: format!("todo {index}"),
                    description: format!("description {index}"),
                    priority: Priority::Normal,
                    order: None,
                    assignees: BTreeSet::new(),
                })
                .await
                .unwrap();
        }
        let snapshot = store.snapshot().await.unwrap();
        let mut app = App::new(
            Session::new(directory.path().into(), "test-model".into()),
            Vec::new(),
        );
        app.todo_mode = Some(TodoMode::List);
        app.todos = snapshot.ordered().into_iter().cloned().collect();
        app.selected_todo = 19;
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("todo 19"));
        assert!(rendered.contains("Esc return"));

        app.todo_mode = Some(TodoMode::Inspect(app.todos[19].id));
        let backend = ratatui::backend::TestBackend::new(60, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("description 19"));
        assert!(rendered.contains("Assignees"));
    }

    #[tokio::test]
    async fn todo_ui_maps_edit_block_assign_evidence_reorder_transition_and_archive() {
        let directory = tempfile::tempdir().unwrap();
        let store = todo_store(&directory);
        let first = store
            .create(NewTodo {
                title: "first".into(),
                description: String::new(),
                priority: Priority::Normal,
                order: None,
                assignees: BTreeSet::new(),
            })
            .await
            .unwrap();
        let second = store
            .create(NewTodo {
                title: "second".into(),
                description: String::new(),
                priority: Priority::Normal,
                order: None,
                assignees: BTreeSet::new(),
            })
            .await
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();

        request_todo_input(
            &tx,
            store.clone(),
            Some(first.id),
            TodoInput::Edit,
            "renamed\ndetails".into(),
        );
        receive_todo_action(&mut rx).await;
        request_todo_input(
            &tx,
            store.clone(),
            Some(first.id),
            TodoInput::Block,
            "waiting on access".into(),
        );
        receive_todo_action(&mut rx).await;
        request_todo_input(
            &tx,
            store.clone(),
            Some(first.id),
            TodoInput::Assign,
            "alice, bob".into(),
        );
        receive_todo_action(&mut rx).await;
        request_todo_input(
            &tx,
            store.clone(),
            Some(first.id),
            TodoInput::Evidence,
            "report.txt".into(),
        );
        receive_todo_action(&mut rx).await;
        request_todo_input(
            &tx,
            store.clone(),
            Some(first.id),
            TodoInput::Progress,
            "halfway".into(),
        );
        receive_todo_action(&mut rx).await;
        request_todo_input(
            &tx,
            store.clone(),
            Some(first.id),
            TodoInput::Note,
            "remember rollback".into(),
        );
        receive_todo_action(&mut rx).await;
        request_todo_input(
            &tx,
            store.clone(),
            Some(first.id),
            TodoInput::Dependencies,
            second.id.0.to_string(),
        );
        receive_todo_action(&mut rx).await;

        let snapshot = store.snapshot().await.unwrap();
        let item = snapshot.items.get(&first.id).unwrap();
        assert_eq!(item.title, "renamed");
        assert_eq!(item.description, "details");
        assert_eq!(item.status, TodoStatus::Blocked);
        assert_eq!(item.assignees.len(), 2);
        assert_eq!(item.evidence[0].text, "report.txt");
        assert_eq!(item.progress[0].text, "halfway");
        assert_eq!(item.notes[0].text, "remember rollback");
        assert!(item.dependencies.contains(&second.id));

        request_todo_input(
            &tx,
            store.clone(),
            Some(first.id),
            TodoInput::Dependencies,
            "not-a-uuid".into(),
        );
        assert!(matches!(rx.recv().await, Some(UiEvent::TodoAction(Err(_)))));
        assert!(matches!(
            rx.recv().await,
            Some(UiEvent::TodoSnapshot(Ok(_)))
        ));
        assert!(
            store.snapshot().await.unwrap().items[&first.id]
                .dependencies
                .contains(&second.id)
        );
        request_todo_input(
            &tx,
            store.clone(),
            Some(first.id),
            TodoInput::Dependencies,
            String::new(),
        );
        receive_todo_action(&mut rx).await;
        assert!(
            store.snapshot().await.unwrap().items[&first.id]
                .dependencies
                .is_empty()
        );

        let mut app = App::new(
            Session::new(directory.path().into(), "test-model".into()),
            Vec::new(),
        );
        app.todos = snapshot.ordered().into_iter().cloned().collect();
        reorder_todo(&app, &tx, store.clone(), first.id, 1);
        receive_todo_action(&mut rx).await;
        let snapshot = store.snapshot().await.unwrap();
        assert!(snapshot.items[&first.id].order >= snapshot.items[&second.id].order);

        store
            .set_status(second.id, TodoStatus::Completed)
            .await
            .unwrap();
        store.set_blockers(first.id, Vec::new()).await.unwrap();
        let mut app = App::new(
            Session::new(directory.path().into(), "test-model".into()),
            Vec::new(),
        );
        app.todo_mode = Some(TodoMode::Inspect(first.id));
        app.todos = store
            .snapshot()
            .await
            .unwrap()
            .ordered()
            .into_iter()
            .cloned()
            .collect();
        handle_todo_key(
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
            &mut app,
            &tx,
            store.clone(),
        )
        .await;
        receive_todo_action(&mut rx).await;
        assert_eq!(
            store.snapshot().await.unwrap().items[&first.id].status,
            TodoStatus::InProgress
        );
        app.todos = store
            .snapshot()
            .await
            .unwrap()
            .ordered()
            .into_iter()
            .cloned()
            .collect();
        handle_todo_key(
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
            &mut app,
            &tx,
            store.clone(),
        )
        .await;
        receive_todo_action(&mut rx).await;
        assert_eq!(
            store.snapshot().await.unwrap().items[&first.id].status,
            TodoStatus::Completed
        );
        app.todos = store
            .snapshot()
            .await
            .unwrap()
            .ordered()
            .into_iter()
            .cloned()
            .collect();
        handle_todo_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            &mut app,
            &tx,
            store.clone(),
        )
        .await;
        receive_todo_action(&mut rx).await;
        assert!(store.snapshot().await.unwrap().items[&first.id].archived());
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

    #[test]
    fn detach_accepts_both_crossterm_control_encodings() {
        assert!(is_terminal_detach_key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL
        )));
        assert!(is_terminal_detach_key(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::CONTROL
        )));
        assert!(is_terminal_detach_key(KeyEvent::new(
            KeyCode::Char('\u{1d}'),
            KeyModifiers::NONE
        )));
    }

    #[test]
    fn display_text_cannot_emit_terminal_control_sequences() {
        assert_eq!(display_safe("before\u{1b}[2Jafter\r"), "before�[2Jafter�");
        assert_eq!(
            display_safe("line one\nline two\tvalue"),
            "line one\nline two\tvalue"
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
    fn transcript_renders_only_assistant_content_as_markdown() {
        let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        session
            .messages
            .push(crate::Message::new(Role::User, "**user literal**"));
        session
            .messages
            .push(crate::Message::new(Role::Assistant, "# Heading\n\n`code`"));
        session
            .messages
            .push(crate::Message::new(Role::Tool, "# tool literal"));
        let app = App::new(session, Vec::new());
        let rendered = transcript(&app, 40);
        let symbols = rendered
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(symbols.contains("**user literal**"));
        assert!(symbols.contains("Heading"));
        assert!(!symbols.contains("# tool literal"));
        assert!(
            rendered
                .lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
        );
    }

    #[test]
    fn no_color_markdown_theme_has_no_foreground_or_background_colors() {
        let theme = markdown_theme_for(true);
        assert_eq!(theme.text, Color::Reset);
        assert_eq!(theme.heading, Color::Reset);
        assert_eq!(theme.link, Color::Reset);
        assert_eq!(theme.code, Color::Reset);
        assert_eq!(theme.code_background, Color::Reset);
        assert_eq!(theme.quote, Color::Reset);
        assert_eq!(theme.table_header, Color::Reset);
        assert_eq!(theme.warning, Color::Reset);
    }

    #[test]
    fn markdown_full_screen_buffer_keeps_borders_and_inner_width() {
        let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        session
            .messages
            .push(crate::Message::new(Role::User, "# literal user"));
        session.messages.push(crate::Message::new(
            Role::Assistant,
            "# Rendered\n\n| A | B |\n|---|---|\n| one | two |",
        ));
        let app = App::new(session, Vec::new());
        let backend = ratatui::backend::TestBackend::new(48, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let rows = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let snapshot = rows.join("\n");
        assert!(snapshot.contains("# literal user"));
        assert!(snapshot.contains("Rendered"));
        assert!(snapshot.contains("one"));
        assert!(snapshot.contains("Conversation"));
        assert!(rows.iter().all(|row| row.chars().count() == 48));
    }

    #[test]
    fn incomplete_markdown_stream_is_stable_and_canonical() {
        let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        let mut app = App::new(session, Vec::new());
        let source = "## Live\n\n```rust\nfn main() {}\n```";
        for character in source.chars() {
            app.streaming_response.push(character);
            let rendered = transcript(&app, 24);
            assert!(rendered.lines.len() < 100);
        }
        assert_eq!(app.streaming_response, source);
        let rendered = transcript(&app, 24);
        let symbols = rendered
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(symbols.contains("fn main() {}"));
    }

    #[test]
    fn manual_scroll_anchor_survives_stream_growth_and_resize() {
        let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        session.messages.push(crate::Message::new(
            Role::Assistant,
            (0..40)
                .map(|line| format!("line {line}\n"))
                .collect::<String>(),
        ));
        let mut app = App::new(session, Vec::new());
        app.conversation_width = 30;
        app.conversation_height = 8;
        app.scroll = 7;
        let before_height = transcript_height(&app, app.conversation_width);
        let before_top = before_height
            .saturating_sub(app.conversation_height)
            .saturating_sub(app.scroll as usize);
        app.streaming_response
            .push_str("new streamed line\nsecond line");
        preserve_manual_anchor(&mut app, before_height);
        let grown_top = transcript_height(&app, app.conversation_width)
            .saturating_sub(app.conversation_height)
            .saturating_sub(app.scroll as usize);
        assert_eq!(grown_top, before_top);
        resize_conversation(&mut app, 18, 12);
        let resized_top = transcript_height(&app, app.conversation_width)
            .saturating_sub(app.conversation_height)
            .saturating_sub(app.scroll as usize);
        assert_eq!(resized_top, before_top);
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
        handle_command("/clear", &mut app, &store, None)
            .await
            .unwrap();
        assert_eq!(app.session.messages.len(), 1);
        handle_command("/clear confirm", &mut app, &store, None)
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
            handle_command("/name field work", &mut app, &store, None)
                .await
                .unwrap()
        );
        assert_eq!(app.session.name.as_deref(), Some("field work"));
        let export = directory.path().join("export.md");
        handle_command(
            &format!("/export {}", export.display()),
            &mut app,
            &store,
            None,
        )
        .await
        .unwrap();
        assert!(export.exists());
    }
}
