//! Full-screen terminal frontend. This module intentionally depends only on the
//! agent's event and approval contracts, so providers can add finer-grained
//! streaming without changing the UI.

use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, EventStream, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
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

use crate::config::{AccessMode, CONFIG_OVERRIDE_SPECS, ConfigValueKind};
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
    Question(QuestionRequest),
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

#[derive(Debug)]
#[doc(hidden)]
pub struct QuestionRequest {
    question: crate::tools::Question,
    response: oneshot::Sender<crate::tools::QuestionAnswer>,
}

struct QuestionDialog {
    request: QuestionRequest,
    selected: usize,
    custom: Composer,
    scroll: Option<u16>,
}

impl QuestionDialog {
    fn insert(&mut self, text: &str) {
        if self.selected != self.request.question.options.len() {
            return;
        }
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            if self.custom.text.len() + ch.len_utf8() > crate::tools::MAX_ANSWER_BYTES {
                break;
            }
            self.custom.insert(ch);
        }
        self.scroll = None;
    }

    fn key(&mut self, key: KeyEvent) -> Option<crate::tools::QuestionAnswer> {
        use crate::tools::QuestionAnswer;
        let count = self.request.question.options.len();
        match key.code {
            KeyCode::Esc => return Some(QuestionAnswer::Cancelled),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(QuestionAnswer::Cancelled);
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll = None;
            }
            KeyCode::Down | KeyCode::Tab => {
                self.selected = (self.selected + 1) % (count + 1);
                self.scroll = None;
            }
            KeyCode::PageUp => self.scroll = Some(self.scroll.unwrap_or(0).saturating_sub(5)),
            KeyCode::PageDown => self.scroll = Some(self.scroll.unwrap_or(0).saturating_add(5)),
            KeyCode::Enter if self.selected < count => {
                return Some(QuestionAnswer::Selected {
                    index: self.selected,
                    answer: self.request.question.options[self.selected].clone(),
                });
            }
            KeyCode::Enter if !self.custom.text.trim().is_empty() => {
                return Some(QuestionAnswer::Custom {
                    answer: self.custom.text.clone(),
                });
            }
            KeyCode::Backspace if self.selected == count => self.custom.backspace(),
            KeyCode::Delete if self.selected == count => self.custom.delete(),
            KeyCode::Home if self.selected == count => self.custom.line_start(),
            KeyCode::End if self.selected == count => self.custom.line_end(),
            KeyCode::Left if self.selected == count => {
                self.custom.cursor = self.custom.text[..self.custom.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i);
            }
            KeyCode::Right if self.selected == count => {
                if let Some(ch) = self.custom.text[self.custom.cursor..].chars().next() {
                    self.custom.cursor += ch.len_utf8();
                }
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&ch.to_string())
            }
            _ => {}
        }
        None
    }
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
    async fn ask_question(
        &self,
        question: &crate::tools::Question,
    ) -> crate::tools::QuestionAnswer {
        let (response, receive) = oneshot::channel();
        if self
            .tx
            .send(UiEvent::Question(QuestionRequest {
                question: question.clone(),
                response,
            }))
            .is_err()
        {
            return crate::tools::QuestionAnswer::Unavailable;
        }
        receive
            .await
            .unwrap_or(crate::tools::QuestionAnswer::Unavailable)
    }

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliRequest {
    pub arguments: Vec<String>,
    pub use_active_config: bool,
    pub resume_after: bool,
    pub verbose: Option<bool>,
    pub log_format: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TuiExit {
    #[default]
    Quit,
    Launch(CliRequest),
}

struct TerminalGuard;

#[derive(Clone, Copy)]
struct SlashCommand {
    name: &'static str,
    usage: &'static str,
    description: &'static str,
    completion: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SlashPaletteItem {
    usage: String,
    description: String,
    completion: String,
}

const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "help",
        usage: "/help",
        description: "Show command help",
        completion: "/help",
    },
    SlashCommand {
        name: "access",
        usage: "/access [MODE]",
        description: "Show or change access mode",
        completion: "/access ",
    },
    SlashCommand {
        name: "provider",
        usage: "/provider [ID]",
        description: "Show or change provider",
        completion: "/provider ",
    },
    SlashCommand {
        name: "workspace",
        usage: "/workspace [PATH]",
        description: "Show or change workspace",
        completion: "/workspace ",
    },
    SlashCommand {
        name: "config",
        usage: "/config [PATH]",
        description: "Show config or relaunch with a file",
        completion: "/config ",
    },
    SlashCommand {
        name: "set",
        usage: "/set KEY VALUE",
        description: "Set any validated config value",
        completion: "/set ",
    },
    SlashCommand {
        name: "tools",
        usage: "/tools",
        description: "List available tool calls",
        completion: "/tools",
    },
    SlashCommand {
        name: "activity",
        usage: "/activity [on|off]",
        description: "Show or hide tool activity",
        completion: "/activity ",
    },
    SlashCommand {
        name: "model",
        usage: "/model [ID]",
        description: "Show or switch the model",
        completion: "/model ",
    },
    SlashCommand {
        name: "new",
        usage: "/new [TITLE]",
        description: "Start a new session",
        completion: "/new",
    },
    SlashCommand {
        name: "name",
        usage: "/name TITLE",
        description: "Rename the current session",
        completion: "/name ",
    },
    SlashCommand {
        name: "branch",
        usage: "/branch [TITLE]",
        description: "Branch the current session",
        completion: "/branch ",
    },
    SlashCommand {
        name: "compact",
        usage: "/compact [KEEP]",
        description: "Compact older context",
        completion: "/compact ",
    },
    SlashCommand {
        name: "export",
        usage: "/export [PATH]",
        description: "Export the session as Markdown",
        completion: "/export ",
    },
    SlashCommand {
        name: "clear",
        usage: "/clear confirm",
        description: "Permanently clear the conversation",
        completion: "/clear confirm",
    },
    SlashCommand {
        name: "auth",
        usage: "/auth ACTION",
        description: "Login, logout, status, or import",
        completion: "/auth ",
    },
    SlashCommand {
        name: "doctor",
        usage: "/doctor",
        description: "Run runtime diagnostics",
        completion: "/doctor",
    },
    SlashCommand {
        name: "sessions",
        usage: "/sessions",
        description: "Browse saved sessions",
        completion: "/sessions",
    },
    SlashCommand {
        name: "models",
        usage: "/models [json]",
        description: "Browse or print available models",
        completion: "/models",
    },
    SlashCommand {
        name: "resume",
        usage: "/resume REF",
        description: "Open a saved session",
        completion: "/resume ",
    },
    SlashCommand {
        name: "verbose",
        usage: "/verbose on|off",
        description: "Relaunch with debug logging",
        completion: "/verbose ",
    },
    SlashCommand {
        name: "log-format",
        usage: "/log-format text|json",
        description: "Relaunch with a log format",
        completion: "/log-format ",
    },
    SlashCommand {
        name: "plain",
        usage: "/plain",
        description: "Continue in line-oriented chat",
        completion: "/plain",
    },
    SlashCommand {
        name: "run",
        usage: "/run [--no-save] PROMPT",
        description: "Run a one-shot task",
        completion: "/run ",
    },
    SlashCommand {
        name: "voyage",
        usage: "/voyage [URL] [NAME]",
        description: "Connect this Helm to Vessel",
        completion: "/voyage ",
    },
    SlashCommand {
        name: "completions",
        usage: "/completions SHELL",
        description: "Generate shell completions",
        completion: "/completions ",
    },
    SlashCommand {
        name: "manpage",
        usage: "/manpage",
        description: "Generate the Helm manpage",
        completion: "/manpage",
    },
];

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        ) {
            let _ = execute!(
                io::stdout(),
                DisableMouseCapture,
                DisableBracketedPaste,
                LeaveAlternateScreen
            );
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
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
    access_mode: AccessMode,
    composer: Composer,
    activity: Vec<String>,
    show_activity: bool,
    streaming_response: String,
    status: String,
    scroll: u16,
    conversation_width: usize,
    conversation_height: usize,
    markdown_theme: MarkdownTheme,
    markdown_syntax_highlighting: bool,
    running: Option<Running>,
    approval: Option<ApprovalRequest>,
    question: Option<QuestionDialog>,
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
    selected_slash_command: usize,
    slash_palette_dismissed: bool,
    slash_models_requested: bool,
    shortcut_help: bool,
    exit: Option<TuiExit>,
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
            access_mode: AccessMode::Approval,
            composer: Composer::default(),
            activity: Vec::new(),
            show_activity: false,
            streaming_response: String::new(),
            status: "Ready".into(),
            scroll: 0,
            conversation_width: 78,
            conversation_height: 13,
            markdown_theme: markdown_theme(),
            markdown_syntax_highlighting: std::env::var_os("NO_COLOR").is_none(),
            running: None,
            approval: None,
            question: None,
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
            selected_slash_command: 0,
            slash_palette_dismissed: false,
            slash_models_requested: false,
            shortcut_help: false,
            exit: None,
            quit: false,
        }
    }

    fn is_running(&self) -> bool {
        self.running.is_some()
    }

    fn cancel(&mut self) {
        if let Some(question) = self.question.take() {
            let _ = question
                .request
                .response
                .send(crate::tools::QuestionAnswer::Cancelled);
        }
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
    access_mode: AccessMode,
) -> Result<TuiExit> {
    let sessions = store.list().await?;
    let mut app = App::new(session, sessions);
    app.provider_label = provider_label;
    app.access_mode = access_mode;
    refresh_terminals(&mut app, terminals.as_ref()).await;
    let mut terminal_events = Some(terminals.subscribe());
    let mut supervisor_events = Some(supervisor.subscribe());
    let mut todo_refresh = tokio::time::interval(std::time::Duration::from_secs(1));
    let _guard = TerminalGuard::enter().context("failed to initialize terminal")?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    if let Ok(size) = terminal.size() {
        app.conversation_width = size.width.max(1) as usize;
        app.conversation_height = size.height.saturating_sub(9).max(1) as usize;
    }
    let mut input = EventStream::new();
    let termination = termination_signal();
    tokio::pin!(termination);

    while !app.quit {
        if app
            .question
            .as_ref()
            .is_some_and(|q| q.request.response.is_closed())
        {
            app.question = None;
        }
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
                            columns.max(1) as usize,
                            rows.saturating_sub(9).max(1) as usize,
                        );
                        if let Some(id) = app.attached_terminal {
                            let _ = terminals.resize(id, columns, rows.saturating_sub(1)).await;
                        }
                        // Discard any stale cells after the terminal changes its backing grid.
                        terminal.clear()?;
                    }
                    Some(Ok(Event::Mouse(mouse))) => handle_mouse(mouse, &mut app),
                    Some(Ok(Event::Paste(text))) if app.question.is_some() => {
                        if let Some(question) = &mut app.question { question.insert(&text); }
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
                                reset_slash_palette(&mut app);
                                request_slash_models_if_needed(&mut app, &agent, &tx);
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
    Ok(app.exit.unwrap_or_default())
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
            app.activity.push(format!(
                "▶ {}: {}",
                compact_line(&name, 40),
                compact_line(&arguments.to_string(), 100)
            ));
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
                compact_line(&result, 120)
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
                compact_line(&error, 120)
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
        UiEvent::Question(request) => {
            if request.response.is_closed() {
                return Ok(());
            }
            if app.attached_terminal.is_some() || app.approval.is_some() || app.question.is_some() {
                let _ = request
                    .response
                    .send(crate::tools::QuestionAnswer::Unavailable);
            } else {
                app.status = "Answer a question · answers are shared with the model".into();
                app.question = Some(QuestionDialog {
                    request,
                    selected: 0,
                    custom: Composer::default(),
                    scroll: None,
                });
            }
        }
        UiEvent::Approval(request) => {
            if app.attached_terminal.is_some() || app.question.is_some() {
                let _ = request.response.send(ApprovalOutcome::Unavailable);
                app.status = "Agent approval denied while direct terminal input is attached".into();
            } else {
                app.status = "Approval required".into();
                app.approval = Some(request);
            }
        }
        UiEvent::Finished(result) => {
            app.question = None;
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
                Err(error) => {
                    let detail = compact_line(&error, 1_000);
                    app.activity.push(format!("✗ provider error: {detail}"));
                    app.status = format!("Error: {}", compact_line(&error, 120));
                }
            }
            app.streaming_response.clear();
            preserve_manual_anchor(app, before);
        }
        UiEvent::SupervisorTree(result) => match result {
            Ok(agents) => {
                app.agents = flatten_agent_tree(agents);
                app.selected_agent = app.selected_agent.min(app.agents.len().saturating_sub(1));
                app.status = supervisor_summary(&app.agents);
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

fn handle_question_key(key: KeyEvent, app: &mut App) -> bool {
    if let Some(mut question) = app.question.take() {
        if !question.request.response.is_closed() {
            if let Some(answer) = question.key(key) {
                let cancelled = answer == crate::tools::QuestionAnswer::Cancelled;
                let _ = question.request.response.send(answer);
                app.status = if cancelled {
                    "Question cancelled"
                } else {
                    "Answer sent"
                }
                .into();
            } else {
                app.question = Some(question);
            }
        }
        return true;
    }
    false
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
    if handle_question_key(key, app) {
        return Ok(());
    }
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
    if handle_shortcut_help_key(key, app) {
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
                if let Some(session) = app.sessions.get(app.selected_session) {
                    let arguments = vec!["chat".into(), "--resume".into(), session.id.to_string()];
                    request_cli(app, store, arguments, true, false).await?;
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
            KeyCode::Char('l') => {
                app.show_activity = !app.show_activity;
                app.status = if app.show_activity {
                    "Activity visible · Ctrl+L hides it"
                } else {
                    "Activity hidden · Ctrl+L shows it"
                }
                .into();
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
            KeyCode::Char('n') if !app.is_running() => start_new_session(app, None),
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
    if !slash_palette_items(app).is_empty() {
        match key.code {
            KeyCode::Up => {
                app.selected_slash_command = app.selected_slash_command.saturating_sub(1);
                return Ok(());
            }
            KeyCode::Down => {
                app.selected_slash_command = (app.selected_slash_command + 1)
                    .min(slash_palette_items(app).len().saturating_sub(1));
                return Ok(());
            }
            KeyCode::Tab => {
                complete_selected_slash_command(app);
                return Ok(());
            }
            KeyCode::Enter if !slash_input_is_complete(app) => {
                complete_selected_slash_command(app);
                return Ok(());
            }
            KeyCode::Esc => {
                app.slash_palette_dismissed = true;
                return Ok(());
            }
            _ => {}
        }
    }
    match key.code {
        KeyCode::Esc if app.is_running() => app.cancel(),
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => app.composer.insert('\n'),
        KeyCode::Enter if !app.is_running() => {
            let prompt = app.composer.take();
            reset_slash_palette(app);
            if !prompt.trim().is_empty() {
                if handle_command(&prompt, app, store, Some(agent), Some(tx)).await? {
                    return Ok(());
                }
                if app.session.messages.len() > 96 {
                    let removed = compact_messages(&mut app.session.messages, 64);
                    app.activity
                        .push(format!("context: compacted {removed} older messages"));
                }
                let history = app.session.messages.clone();
                app.session
                    .messages
                    .push(crate::Message::new(Role::User, prompt.clone()));
                store.save(&mut app.session).await?;
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
        KeyCode::Char(character) if !app.is_running() => {
            app.composer.insert(character);
            reset_slash_palette(app);
            request_slash_models_if_needed(app, agent, tx);
        }
        KeyCode::Backspace if !app.is_running() => {
            app.composer.backspace();
            reset_slash_palette(app);
            request_slash_models_if_needed(app, agent, tx);
        }
        KeyCode::Delete if !app.is_running() => {
            app.composer.delete();
            reset_slash_palette(app);
            request_slash_models_if_needed(app, agent, tx);
        }
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
        KeyCode::PageUp => scroll_conversation(app, 8),
        KeyCode::PageDown => scroll_conversation(app, -8),
        _ => {}
    }
    Ok(())
}

fn handle_shortcut_help_key(key: KeyEvent, app: &mut App) -> bool {
    if app.shortcut_help {
        if matches!(key.code, KeyCode::F(1) | KeyCode::Esc) {
            app.shortcut_help = false;
        }
        return true;
    }
    if key.code == KeyCode::F(1) {
        app.shortcut_help = true;
        return true;
    }
    false
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

fn request_slash_models_if_needed(
    app: &mut App,
    agent: &Arc<Agent>,
    tx: &mpsc::UnboundedSender<UiEvent>,
) {
    let wants_models = app.composer.text.starts_with("/model ")
        || app.composer.text.starts_with("/models ")
        || app.composer.text.starts_with("/set model ");
    if !wants_models {
        app.slash_models_requested = false;
    } else if app.models.is_empty() && !app.slash_models_requested {
        app.slash_models_requested = true;
        request_models(tx, agent.clone(), false);
        app.status = "Discovering models…".into();
    }
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

fn supervisor_counts(agents: &[AgentView]) -> (usize, usize) {
    let active = agents
        .iter()
        .filter(|agent| !agent.status.is_terminal())
        .count();
    (active, agents.len().saturating_sub(active))
}

fn supervisor_summary(agents: &[AgentView]) -> String {
    let (active, retained) = supervisor_counts(agents);
    let noun = if active == 1 { "agent" } else { "agents" };
    format!("Supervising {active} active {noun} · {retained} retained")
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
    if let Some(question) = &app.question {
        draw_question(frame, area, question);
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
    if app.shortcut_help && app.approval.is_none() {
        draw_shortcut_help(frame, area, app);
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
    let title = app.session.display_name();
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
                "  {title} · {} · {} · access: {} · {}",
                app.session.model,
                app.provider_label,
                app.access_mode,
                app.session.workspace.display()
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    let transcript = transcript(app, viewport_width_for(chunks[1]));
    let viewport_height = chunks[1].height as usize;
    let viewport_width = chunks[1].width.max(1) as usize;
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
            .scroll((offset, 0)),
        chunks[1],
    );
    let composer_width = chunks[2].width.max(1);
    let composer_height = chunks[2].height.saturating_sub(1).max(1);
    let (composer_row, composer_column) =
        cursor_position(&app.composer.text[..app.composer.cursor], composer_width);
    let composer_scroll = composer_row.saturating_sub(composer_height - 1);
    frame.render_widget(
        Paragraph::new(app.composer.text.as_str())
            .wrap(Wrap { trim: false })
            .scroll((composer_scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .style(if app.is_running() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            }),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new(format!("F1 shortcuts  │  {}", app.status))
            .style(Style::default().fg(Color::Gray)),
        chunks[3],
    );
    draw_slash_palette(frame, chunks[2], app);
    if !app.is_running() {
        frame.set_cursor_position((
            (chunks[2].x + composer_column).min(chunks[2].right().saturating_sub(1)),
            (chunks[2].y + 1 + composer_row.saturating_sub(composer_scroll))
                .min(chunks[2].bottom().saturating_sub(1)),
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

fn slash_command_query(app: &App) -> Option<&str> {
    if app.is_running() || app.slash_palette_dismissed {
        return None;
    }
    let query = app.composer.text.strip_prefix('/')?;
    (!query.chars().any(char::is_whitespace)).then_some(query)
}

fn slash_palette_matches(app: &App) -> Vec<SlashCommand> {
    let Some(query) = slash_command_query(app) else {
        return Vec::new();
    };
    SLASH_COMMANDS
        .iter()
        .copied()
        .filter(|command| command.name.starts_with(query))
        .collect()
}

fn fixed_suggestions(prefix: &str, query: &str, values: &[(&str, &str)]) -> Vec<SlashPaletteItem> {
    let query = query.trim().to_ascii_lowercase();
    values
        .iter()
        .filter(|(value, _)| value.to_ascii_lowercase().contains(&query))
        .map(|(value, description)| SlashPaletteItem {
            usage: (*value).into(),
            description: (*description).into(),
            completion: format!("{prefix}{value}"),
        })
        .collect()
}

fn argument_hint(usage: &str, description: &str) -> SlashPaletteItem {
    SlashPaletteItem {
        usage: usage.into(),
        description: description.into(),
        completion: String::new(),
    }
}

fn expand_user_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(suffix) = raw.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(suffix);
    }
    PathBuf::from(raw)
}

fn filesystem_suggestions<F>(
    raw: &str,
    workspace: &Path,
    directories_only: bool,
    completion: F,
) -> Vec<SlashPaletteItem>
where
    F: Fn(&str, bool) -> String,
{
    let raw = raw.trim();
    let (scan_input, rendered_prefix) = if raw == "~" || raw.starts_with("~/") {
        let suffix = raw
            .strip_prefix('~')
            .unwrap_or_default()
            .trim_start_matches('/');
        let Some(home) = dirs::home_dir() else {
            return Vec::new();
        };
        (home.join(suffix), "~/")
    } else {
        (PathBuf::from(raw), "")
    };
    let trailing_separator = raw == "~" || raw.ends_with(std::path::MAIN_SEPARATOR);
    let (directory, needle, display_parent) = if raw.is_empty() {
        (workspace.to_path_buf(), String::new(), String::new())
    } else if trailing_separator {
        let directory = if scan_input.is_absolute() {
            scan_input.clone()
        } else {
            workspace.join(&scan_input)
        };
        (
            directory,
            String::new(),
            if raw == "~" {
                "~/".into()
            } else {
                raw.to_owned()
            },
        )
    } else {
        let parent = scan_input
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""));
        let directory = if scan_input.is_absolute() {
            parent.to_path_buf()
        } else {
            workspace.join(parent)
        };
        let needle = scan_input
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let display_parent = if rendered_prefix.is_empty() {
            parent.to_string_lossy().into_owned()
        } else {
            let suffix = parent
                .strip_prefix(dirs::home_dir().unwrap_or_default())
                .unwrap_or(parent)
                .to_string_lossy();
            if suffix.is_empty() {
                rendered_prefix.into()
            } else {
                format!("{rendered_prefix}{suffix}/")
            }
        };
        (directory, needle, display_parent)
    };
    let mut entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries.filter_map(Result::ok).collect::<Vec<_>>(),
        Err(_) => return Vec::new(),
    };
    entries.sort_by_key(|entry| {
        (
            !entry.file_type().is_ok_and(|kind| kind.is_dir()),
            entry.file_name().to_string_lossy().to_ascii_lowercase(),
        )
    });
    entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name
                .to_ascii_lowercase()
                .starts_with(&needle.to_ascii_lowercase())
                || (name.starts_with('.') && !needle.starts_with('.'))
            {
                return None;
            }
            let is_directory = entry.file_type().ok()?.is_dir();
            if directories_only && !is_directory {
                return None;
            }
            let separator = if !display_parent.is_empty()
                && !display_parent.ends_with(std::path::MAIN_SEPARATOR)
            {
                std::path::MAIN_SEPARATOR_STR
            } else {
                ""
            };
            let mut value = format!("{display_parent}{separator}{name}");
            if is_directory {
                value.push(std::path::MAIN_SEPARATOR);
            }
            Some(SlashPaletteItem {
                usage: value.clone(),
                description: if is_directory { "directory" } else { "file" }.into(),
                completion: completion(&value, is_directory),
            })
        })
        .take(100)
        .collect()
}

fn model_suggestions(app: &App, query: &str, prefix: &str) -> Vec<SlashPaletteItem> {
    let query = query.trim().to_ascii_lowercase();
    app.models
        .iter()
        .filter(|model| {
            query.is_empty()
                || model.id.to_ascii_lowercase().contains(&query)
                || model.display_name.to_ascii_lowercase().contains(&query)
        })
        .map(|model| SlashPaletteItem {
            usage: model.id.clone(),
            description: model.display_name.clone(),
            completion: format!("{prefix}{}", model.id),
        })
        .collect()
}

fn environment_name_suggestions(query: &str, prefix: &str) -> Vec<SlashPaletteItem> {
    let query = query.to_ascii_lowercase();
    let mut names = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| name.to_ascii_lowercase().contains(&query))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .take(100)
        .map(|name| SlashPaletteItem {
            usage: name.clone(),
            description: "environment variable".into(),
            completion: format!("{prefix}{name} "),
        })
        .collect()
}

fn set_value_suggestions(app: &App, key: &str, value: &str) -> Vec<SlashPaletteItem> {
    let root_key = key.split('.').next().unwrap_or(key);
    let Some(spec) = CONFIG_OVERRIDE_SPECS
        .iter()
        .find(|spec| spec.key == root_key)
    else {
        return Vec::new();
    };
    let prefix = format!("/set {key} ");
    match spec.kind {
        ConfigValueKind::Provider => fixed_suggestions(&prefix, value, PROVIDER_VALUES),
        ConfigValueKind::Model => model_suggestions(app, value, &prefix),
        ConfigValueKind::EnvironmentName => fixed_suggestions(
            &prefix,
            value,
            &[
                ("OPENAI_API_KEY", "OpenAI API credential"),
                ("ANTHROPIC_API_KEY", "Anthropic API credential"),
            ],
        ),
        ConfigValueKind::Url => fixed_suggestions(
            &prefix,
            value,
            &[
                ("https://api.openai.com/v1", "OpenAI public API"),
                ("https://api.anthropic.com", "Anthropic public API"),
            ],
        ),
        ConfigValueKind::PositiveInteger | ConfigValueKind::NonNegativeInteger => {
            let values: &[(&str, &str)] = match key {
                "max_turns" => &[
                    ("16", "short"),
                    ("32", "standard"),
                    ("64", "extended"),
                    ("128", "large"),
                ],
                "max_tokens" => &[
                    ("2048", "small"),
                    ("4096", "standard"),
                    ("8192", "large"),
                    ("16384", "very large"),
                ],
                "provider_retry_attempts" => &[
                    ("0", "disabled"),
                    ("2", "light"),
                    ("4", "standard"),
                    ("8", "persistent"),
                ],
                "provider_retry_initial_ms" => {
                    &[("250", "fast"), ("500", "standard"), ("1000", "one second")]
                }
                "provider_retry_max_ms" => &[
                    ("4000", "four seconds"),
                    ("8000", "standard"),
                    ("16000", "sixteen seconds"),
                ],
                "command_timeout_secs" => &[
                    ("30", "short"),
                    ("120", "standard"),
                    ("300", "five minutes"),
                    ("900", "fifteen minutes"),
                ],
                "max_output_bytes" | "terminal_max_unread_bytes" => &[
                    ("65536", "64 KiB"),
                    ("131072", "128 KiB"),
                    ("1048576", "1 MiB"),
                    ("8388608", "8 MiB"),
                ],
                "terminal_max_count" | "subagent_max_concurrency" => &[
                    ("1", "one"),
                    ("4", "four"),
                    ("8", "eight"),
                    ("16", "sixteen"),
                ],
                "subagent_max_agents" => &[
                    ("16", "small"),
                    ("32", "standard"),
                    ("64", "large"),
                    ("128", "very large"),
                ],
                "subagent_event_history" => {
                    &[("512", "small"), ("2048", "standard"), ("8192", "large")]
                }
                _ => &[("1", "minimum"), ("16", "small"), ("64", "standard")],
            };
            fixed_suggestions(&prefix, value, values)
        }
        ConfigValueKind::Temperature => fixed_suggestions(
            &prefix,
            value,
            &[
                ("0", "deterministic"),
                ("0.2", "focused"),
                ("0.7", "creative"),
                ("1.0", "high variance"),
            ],
        ),
        ConfigValueKind::Access => fixed_suggestions(&prefix, value, ACCESS_VALUES),
        ConfigValueKind::Approval => fixed_suggestions(
            &prefix,
            value,
            &[
                ("always", "approve every action"),
                ("on-risk", "approve risky actions"),
                ("never", "never prompt"),
            ],
        ),
        ConfigValueKind::UnattendedApproval => fixed_suggestions(
            &prefix,
            value,
            &[
                ("deny", "deny without a user"),
                ("allow", "allow without a user"),
            ],
        ),
        ConfigValueKind::Path => {
            filesystem_suggestions(value, &app.session.workspace, true, |path, _| {
                format!("{prefix}{}", expand_user_path(path).display())
            })
        }
        ConfigValueKind::PathList => filesystem_suggestions(
            value.trim_matches(|ch| matches!(ch, '[' | ']' | '"')),
            &app.session.workspace,
            true,
            |path, _| format!("{prefix}[\"{}\"]", expand_user_path(path).display()),
        ),
        ConfigValueKind::StringList => match key {
            "deny_commands" => fixed_suggestions(
                &prefix,
                value,
                &[(
                    "[\"shutdown\", \"reboot\", \"mkfs\"]",
                    "safe default deny list",
                )],
            ),
            "inherit_env" => fixed_suggestions(
                &prefix,
                value,
                &[(
                    "[\"PATH\", \"LANG\", \"LC_ALL\", \"TERM\"]",
                    "safe terminal environment",
                )],
            ),
            _ if value.trim().is_empty() => vec![argument_hint(
                "[\"VALUE\", ...]",
                "Enter a TOML string array",
            )],
            _ => Vec::new(),
        },
        ConfigValueKind::Executable => fixed_suggestions(
            &prefix,
            value,
            &[("codex", "Codex compatibility executable")],
        ),
        ConfigValueKind::Text if value.trim().is_empty() => {
            vec![argument_hint("<TEXT>", "Enter a free-form value")]
        }
        ConfigValueKind::StringMap if value.trim().is_empty() => {
            vec![argument_hint("<VALUE>", "Enter the map entry value")]
        }
        ConfigValueKind::McpServers if value.trim().is_empty() => vec![argument_hint(
            "<VALUE>",
            "Enter a string, TOML array, or map entry value",
        )],
        ConfigValueKind::Text | ConfigValueKind::StringMap | ConfigValueKind::McpServers => {
            Vec::new()
        }
    }
}

const ACCESS_VALUES: &[(&str, &str)] = &[
    ("read-only", "Inspect without mutations"),
    ("approval", "Ask before consequential actions"),
    ("unrestricted", "Proceed without approval prompts"),
];

const PROVIDER_VALUES: &[(&str, &str)] = &[
    ("openai-responses", "OpenAI Responses API"),
    ("openai-chat", "OpenAI-compatible Chat Completions"),
    ("chatgpt-oauth", "Native ChatGPT subscription"),
    ("anthropic", "Anthropic Messages API"),
    ("codex-compatibility", "External Codex compatibility bridge"),
    ("openai", "Legacy alias for openai-chat"),
    ("codex-subscription", "Legacy alias for codex-compatibility"),
];

fn slash_argument_suggestions(app: &App) -> Vec<SlashPaletteItem> {
    let Some(input) = app.composer.text.strip_prefix('/') else {
        return Vec::new();
    };
    let Some((command, argument)) = input.split_once(char::is_whitespace) else {
        return Vec::new();
    };
    if command == "set" {
        if let Some((key, value)) = argument.split_once(char::is_whitespace) {
            return set_value_suggestions(app, key, value);
        }
        if let Some(query) = argument.strip_prefix("env.") {
            let suggestions = environment_name_suggestions(query, "/set env.");
            return if suggestions.is_empty() && query.is_empty() {
                vec![argument_hint(
                    "<NAME>",
                    "Enter an environment variable name",
                )]
            } else {
                suggestions
            };
        }
        if let Some(rest) = argument.strip_prefix("mcp_servers.") {
            if let Some((server, query)) = rest.split_once(".env.") {
                return environment_name_suggestions(
                    query,
                    &format!("/set mcp_servers.{server}.env."),
                );
            }
            let (server, field_query) = rest.split_once('.').unwrap_or((rest, ""));
            if server.is_empty() {
                return vec![argument_hint("<NAME>", "Enter an MCP server name")];
            }
            return fixed_suggestions(
                &format!("/set mcp_servers.{server}."),
                field_query,
                &[
                    ("command ", "Server executable"),
                    ("args ", "TOML argument array"),
                    ("env.", "Server environment variable"),
                ],
            );
        }
        let query = argument.trim().to_ascii_lowercase();
        return CONFIG_OVERRIDE_SPECS
            .iter()
            .filter(|spec| spec.key.contains(&query))
            .map(|spec| SlashPaletteItem {
                usage: spec.key.into(),
                description: spec.description.into(),
                completion: format!(
                    "/set {}{}",
                    spec.key,
                    if matches!(
                        spec.kind,
                        ConfigValueKind::StringMap | ConfigValueKind::McpServers
                    ) {
                        "."
                    } else {
                        " "
                    }
                ),
            })
            .collect();
    }
    if command == "workspace" {
        let suggestions =
            filesystem_suggestions(argument, &app.session.workspace, true, |path, _| {
                format!("/workspace {path}")
            });
        return if suggestions.is_empty() && argument.trim().is_empty() {
            vec![argument_hint("<PATH>", "Enter a workspace directory")]
        } else {
            suggestions
        };
    }
    if matches!(command, "config" | "export") {
        let suggestions =
            filesystem_suggestions(argument, &app.session.workspace, false, |path, _| {
                format!("/{command} {path}")
            });
        return if suggestions.is_empty() && argument.trim().is_empty() {
            vec![argument_hint("<PATH>", "Enter a file path")]
        } else {
            suggestions
        };
    }
    if command == "auth" && argument.starts_with("import-codex --path ") {
        let path = argument.trim_start_matches("import-codex --path ");
        return filesystem_suggestions(path, &app.session.workspace, false, |path, _| {
            let path = expand_user_path(path).to_string_lossy().into_owned();
            format!("/auth import-codex --path {}", shell_words::quote(&path))
        });
    }
    if command == "auth" && argument.starts_with("import-codex --force --path ") {
        let path = argument.trim_start_matches("import-codex --force --path ");
        return filesystem_suggestions(path, &app.session.workspace, false, |path, _| {
            let path = expand_user_path(path).to_string_lossy().into_owned();
            format!(
                "/auth import-codex --force --path {}",
                shell_words::quote(&path)
            )
        });
    }
    if command == "run"
        && let Some(prompt) = argument.strip_prefix("--no-save ")
    {
        return if prompt.trim().is_empty() {
            vec![argument_hint("<PROMPT>", "Enter the unsaved task to run")]
        } else {
            Vec::new()
        };
    }
    let query = argument.trim().to_ascii_lowercase();
    let fixed: &[(&str, &str)] = match command {
        "access" => ACCESS_VALUES,
        "provider" => PROVIDER_VALUES,
        "activity" | "verbose" => &[("on", "Enable"), ("off", "Disable")],
        "log-format" => &[("text", "Human-readable logs"), ("json", "JSON logs")],
        "compact" => &[
            ("24", "keep recent context"),
            ("64", "keep extended context"),
            ("96", "keep large context"),
        ],
        "run" => &[("--no-save ", "Run without saving the result")],
        "voyage" => &[("http://127.0.0.1:9480", "Local Vessel")],
        "completions" => &[
            ("bash", "Bash"),
            ("elvish", "Elvish"),
            ("fish", "Fish"),
            ("powershell", "PowerShell"),
            ("zsh", "Zsh"),
        ],
        "auth" if argument.starts_with("login ") => {
            &[("login --device", "Sign in with a device code")]
        }
        "auth" if argument.starts_with("import-codex ") => &[
            ("import-codex --path ", "Choose a Codex auth file"),
            ("import-codex --force", "Replace existing Helm credentials"),
            (
                "import-codex --force --path ",
                "Replace credentials using a selected file",
            ),
        ],
        "auth" => &[
            ("status", "Show credential status"),
            ("login", "Sign in with a browser callback"),
            ("login --device", "Sign in with a device code"),
            ("logout", "Remove Helm credentials"),
            ("import-codex", "Import an existing Codex login once"),
        ],
        "models" => &[("json", "Print the provider model catalog as JSON")],
        _ => &[],
    };
    let prefix = format!("/{command} ");
    let mut suggestions = fixed_suggestions(&prefix, &query, fixed);
    if argument.trim().is_empty() {
        match command {
            "new" | "name" | "branch" => {
                suggestions.push(argument_hint("<TITLE>", "Enter a session title"))
            }
            "run" => suggestions.push(argument_hint("<PROMPT>", "Enter the task to run")),
            "voyage" => suggestions.push(argument_hint(
                "<URL> [NAME]",
                "Enter a Vessel URL and optional Helm name",
            )),
            _ => {}
        }
    }
    if matches!(command, "model" | "models") {
        suggestions.extend(model_suggestions(app, &query, "/model "));
    }
    if command == "resume" {
        suggestions.extend(
            app.sessions
                .iter()
                .filter(|session| {
                    let id = session.id.to_string();
                    query.is_empty()
                        || id.contains(&query)
                        || session
                            .name
                            .as_deref()
                            .is_some_and(|name| name.to_ascii_lowercase().contains(&query))
                })
                .map(|session| SlashPaletteItem {
                    usage: session.id.to_string(),
                    description: session.display_name(),
                    completion: format!("/resume {}", session.id),
                }),
        );
    }
    suggestions
}

fn slash_palette_items(app: &App) -> Vec<SlashPaletteItem> {
    let commands = slash_palette_matches(app);
    if !commands.is_empty() {
        return commands
            .into_iter()
            .map(|command| SlashPaletteItem {
                usage: command.usage.into(),
                description: command.description.into(),
                completion: command.completion.into(),
            })
            .collect();
    }
    slash_argument_suggestions(app)
}

fn slash_input_is_complete(app: &App) -> bool {
    let Some(query) = slash_command_query(app) else {
        return slash_palette_items(app)
            .iter()
            .any(|item| item.completion == app.composer.text);
    };
    SLASH_COMMANDS.iter().any(|command| command.name == query)
}

fn reset_slash_palette(app: &mut App) {
    app.selected_slash_command = 0;
    app.slash_palette_dismissed = false;
}

fn complete_selected_slash_command(app: &mut App) {
    let items = slash_palette_items(app);
    let Some(item) = items.get(app.selected_slash_command) else {
        return;
    };
    if item.completion.is_empty() {
        return;
    }
    app.composer.text.clone_from(&item.completion);
    app.composer.cursor = app.composer.text.len();
    reset_slash_palette(app);
}

fn draw_slash_palette(frame: &mut ratatui::Frame<'_>, composer_area: Rect, app: &App) {
    let items = slash_palette_items(app);
    if items.is_empty() || composer_area.y < 3 || composer_area.width < 8 {
        return;
    }
    let selected = app
        .selected_slash_command
        .min(items.len().saturating_sub(1));
    let available_rows = composer_area.y.saturating_sub(2).max(1) as usize;
    let columns = usize::from(items.len() > available_rows && composer_area.width >= 72) + 1;
    let rows_needed = items.len().div_ceil(columns);
    let visible_rows = rows_needed.min(available_rows);
    let capacity = visible_rows * columns;
    let start = (selected / capacity) * capacity;
    let cell_width = (composer_area.width.saturating_sub(2) as usize / columns).max(1);
    let mut lines = Vec::with_capacity(visible_rows);
    for row in 0..visible_rows {
        let mut spans = Vec::new();
        for column in 0..columns {
            let index = start + row + column * visible_rows;
            let Some(item) = items.get(index) else {
                continue;
            };
            let content = format!("{:<18} {}", item.usage, item.description);
            let mut content = one_line(&content, cell_width.saturating_sub(1));
            let padding = cell_width.saturating_sub(content.chars().count());
            content.extend(std::iter::repeat_n(' ', padding));
            let style = if index == selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            spans.push(Span::styled(content, style));
        }
        lines.push(Line::from(spans));
    }
    let height = visible_rows as u16 + 2;
    let popup = Rect {
        x: composer_area.x,
        y: composer_area.y.saturating_sub(height),
        width: composer_area.width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(
            Block::default()
                .title(" Commands and options · ↑↓ select · Enter/Tab complete · Esc close ")
                .borders(Borders::ALL),
        ),
        popup,
    );
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
    let (active, retained) = supervisor_counts(&app.agents);
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
            Span::raw(format!("  {active} active · {retained} retained")),
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
    if app.show_activity && !app.activity.is_empty() {
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
    area.width.max(1) as usize
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

fn max_conversation_scroll(app: &App) -> u16 {
    transcript_height(app, app.conversation_width)
        .saturating_sub(app.conversation_height)
        .min(u16::MAX as usize) as u16
}

fn scroll_conversation(app: &mut App, lines: i16) {
    let next = if lines >= 0 {
        app.scroll.saturating_add(lines as u16)
    } else {
        app.scroll.saturating_sub(lines.unsigned_abs())
    };
    app.scroll = next.min(max_conversation_scroll(app));
}

fn handle_mouse(mouse: MouseEvent, app: &mut App) {
    let conversation_is_visible = app.question.is_none()
        && app.attached_terminal.is_none()
        && app.approval.is_none()
        && !app.show_sessions
        && !app.terminal_picker
        && !app.model_picker
        && !app.shortcut_help
        && app.supervisor_mode.is_none()
        && app.todo_mode.is_none();
    if !conversation_is_visible {
        return;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => scroll_conversation(app, 3),
        MouseEventKind::ScrollDown => scroll_conversation(app, -3),
        _ => {}
    }
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
                session.display_name(),
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

// Wrap as plain terminal-safe text, never interpret model options as Markdown.
fn question_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let size = grapheme.width();
        if used + size > width.max(1) && !current.is_empty() {
            lines.push(Line::from(std::mem::take(&mut current)));
            used = 0;
        }
        current.push_str(grapheme);
        used += size;
    }
    lines.push(Line::from(current));
    lines
}

fn draw_question(frame: &mut ratatui::Frame<'_>, area: Rect, dialog: &QuestionDialog) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);
    let width = chunks[0].width.saturating_sub(2).max(1) as usize;
    let height = chunks[0].height.saturating_sub(2).max(1) as usize;
    let mut lines = question_lines(&display_safe(&dialog.request.question.question), width);
    lines.push(Line::from(""));
    let mut selected_line = 0;
    for (index, option) in dialog
        .request
        .question
        .options
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("Other / custom answer"))
        .enumerate()
    {
        let selected = index == dialog.selected;
        if selected {
            selected_line = lines.len();
        }
        let prefix = if selected { "▶" } else { " " };
        let mut option_lines = question_lines(
            &format!("{prefix} {}. {}", index + 1, display_safe(option)),
            width,
        );
        if selected {
            for line in &mut option_lines {
                *line = line.clone().style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
        lines.extend(option_lines);
    }
    if dialog.selected == dialog.request.question.options.len() {
        lines.push(Line::from(""));
        let (before, after) = dialog.custom.text.split_at(dialog.custom.cursor);
        let input = format!("Answer: {}▏{}", display_safe(before), display_safe(after));
        let cursor_line = question_lines(&format!("Answer: {}▏", display_safe(before)), width)
            .len()
            .saturating_sub(1);
        selected_line = lines.len() + cursor_line;
        lines.extend(question_lines(&input, width));
    }
    let max_scroll = lines.len().saturating_sub(height);
    let scroll = dialog
        .scroll
        .map(usize::from)
        .unwrap_or_else(|| selected_line.saturating_sub(height.saturating_sub(2)))
        .min(max_scroll);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll.min(u16::MAX as usize) as u16, 0))
            .block(
                Block::default()
                    .title(" Question · shared with model ")
                    .borders(Borders::ALL),
            ),
        chunks[0],
    );
    frame.render_widget(Paragraph::new("↑/↓/Tab choose · Enter submit · Esc cancel · PgUp/PgDn scroll. Answers are saved; do not enter secrets.")
        .wrap(Wrap { trim: true }), chunks[1]);
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

fn draw_shortcut_help(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let (context, detailed, compact) = if app.model_picker {
        (
            "Models",
            "type: filter\n↑/↓: select\nEnter: switch\nTab: enter model ID manually\nCtrl+R: refresh models\nEsc: return",
            "type filter · ↑↓ select · Enter switch · Tab manual · Ctrl+R refresh",
        )
    } else if app.supervisor_mode.is_some() {
        (
            "Agents",
            "↑/↓: select\nEnter: inspect\nm: message\nf: follow-up\nc twice: cancel\nr: refresh\nPageUp/PageDown: progress\nEsc: return",
            "↑↓ select · Enter inspect · m message · f follow-up · c,c cancel · Esc",
        )
    } else if app.todo_mode.is_some() {
        (
            "Todos",
            "↑/↓: select\nEnter: inspect\nn: new\ne: edit\nt: next status\nb/u: block/unblock\na: assign\nd: dependencies\np/o/v: progress/note/evidence\nJ/K: reorder\nx: archive\nr: reload\nEsc: return",
            "↑↓ select · Enter inspect · n new · e edit · t status · Esc",
        )
    } else if app.terminal_picker {
        (
            "Terminals",
            "↑/↓: select\nEnter: attach\nr: refresh\nEsc: return\n\nWhile attached, Ctrl+T or Ctrl+] detaches without stopping the process.",
            "↑↓ select · Enter attach · r refresh · Esc return",
        )
    } else if app.show_sessions {
        (
            "Sessions",
            "↑/↓: select\nEnter: open\nEsc: return",
            "↑↓ select · Enter open · Esc return",
        )
    } else {
        (
            "Conversation",
            "Enter: send\nAlt+Enter: newline\nPageUp/PageDown: scroll\nEsc: cancel active work\nCtrl+D: todos\nCtrl+A: agents\nCtrl+M: models\nCtrl+T: terminals\nCtrl+L: activity\nCtrl+S: sessions\nCtrl+N: new session\nCtrl+B: branch\nCtrl+K: compact\nCtrl+E: export\nCtrl+C: cancel or quit\nCtrl+Q: quit",
            "^D todos · ^A agents · ^M models · ^T terminals · ^L activity · ^S sessions · ^N new · ^B branch · ^K compact · ^E export",
        )
    };
    let content = if area.height < 18 { compact } else { detailed };
    let popup = centered(area, 82, if area.height < 18 { 55 } else { 82 });
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(content).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(format!(" {context} shortcuts · F1/Esc close "))
                .borders(Borders::ALL),
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

fn compact_line(text: &str, max: usize) -> String {
    let safe = display_safe(text);
    let compact = safe.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = compact.chars().take(max).collect::<String>();
    if compact.chars().count() > max {
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

async fn request_cli(
    app: &mut App,
    store: &SessionStore,
    arguments: Vec<String>,
    use_active_config: bool,
    resume_after: bool,
) -> Result<()> {
    store.save(&mut app.session).await?;
    app.exit = Some(TuiExit::Launch(CliRequest {
        arguments,
        use_active_config,
        resume_after,
        verbose: None,
        log_format: None,
    }));
    app.quit = true;
    Ok(())
}

fn resume_chat_arguments(app: &App) -> Vec<String> {
    vec!["chat".into(), "--resume".into(), app.session.id.to_string()]
}

fn parse_words(argument: &str, usage: &str) -> Result<Vec<String>> {
    shell_words::split(argument).with_context(|| format!("invalid arguments; usage: {usage}"))
}

fn start_new_session(app: &mut App, name: Option<&str>) {
    let mut session = Session::new(app.session.workspace.clone(), app.session.model.clone());
    if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
        session.name = Some(name.trim().to_owned());
    }
    let name = session.display_name();
    app.session = session;
    app.activity.clear();
    app.streaming_response.clear();
    app.scroll = 0;
    app.status = format!("New session: {name}");
}

async fn handle_command(
    command: &str,
    app: &mut App,
    store: &SessionStore,
    agent: Option<&Arc<Agent>>,
    tx: Option<&mpsc::UnboundedSender<UiEvent>>,
) -> Result<bool> {
    let Some(command) = command.strip_prefix('/') else {
        return Ok(false);
    };
    let (name, argument) = command.split_once(' ').unwrap_or((command, ""));
    match name {
        "help" => {
            app.show_activity = true;
            app.activity.push("Slash commands:".into());
            app.activity.extend(
                SLASH_COMMANDS
                    .iter()
                    .map(|command| format!("  {:<24} {}", command.usage, command.description)),
            );
            app.status = "Slash command help added to the conversation".into();
        }
        "access" if argument.trim().is_empty() => {
            app.status = format!("Current access mode: {}", app.access_mode)
        }
        "access" => {
            let mode = argument.trim();
            if !matches!(mode, "read-only" | "approval" | "unrestricted") {
                app.status = "Usage: /access read-only|approval|unrestricted".into();
            } else {
                let mut arguments = vec!["--access".into(), mode.into()];
                arguments.extend(resume_chat_arguments(app));
                request_cli(app, store, arguments, true, false).await?;
            }
        }
        "provider" if argument.trim().is_empty() => {
            app.status = format!("Current provider: {}", app.provider_label)
        }
        "provider" => {
            let provider = argument.trim();
            if !matches!(
                provider,
                "openai-responses"
                    | "openai-chat"
                    | "openai"
                    | "chatgpt-oauth"
                    | "anthropic"
                    | "codex-compatibility"
                    | "codex-subscription"
            ) {
                app.status = "Usage: /provider openai-responses|openai-chat|chatgpt-oauth|anthropic|codex-compatibility".into();
            } else {
                let mut arguments = vec!["--provider".into(), provider.into()];
                arguments.extend(resume_chat_arguments(app));
                request_cli(app, store, arguments, true, false).await?;
            }
        }
        "workspace" if argument.trim().is_empty() => {
            app.status = format!("Current workspace: {}", app.session.workspace.display())
        }
        "workspace" => {
            let path = expand_user_path(argument.trim());
            match path.canonicalize() {
                Ok(path) if path.is_dir() => {
                    request_cli(
                        app,
                        store,
                        vec![
                            "--workspace".into(),
                            path.to_string_lossy().into_owned(),
                            "chat".into(),
                        ],
                        true,
                        false,
                    )
                    .await?;
                }
                _ => app.status = format!("Workspace does not exist: {}", path.display()),
            }
        }
        "config" if argument.trim().is_empty() => {
            request_cli(app, store, vec!["config".into()], true, true).await?;
        }
        "config" => {
            let path = expand_user_path(argument.trim());
            if !path.is_file() {
                app.status = format!("Config file does not exist: {}", path.display());
            } else {
                let mut arguments = vec!["--config".into(), path.to_string_lossy().into_owned()];
                arguments.extend(resume_chat_arguments(app));
                request_cli(app, store, arguments, false, false).await?;
            }
        }
        "set" => {
            let Some((key, value)) = argument.trim().split_once(char::is_whitespace) else {
                app.status = "Usage: /set KEY VALUE".into();
                return Ok(true);
            };
            let mut arguments = vec!["--set".into(), format!("{}={}", key.trim(), value.trim())];
            arguments.extend(resume_chat_arguments(app));
            request_cli(app, store, arguments, true, false).await?;
        }
        "tools" => {
            if let Some(agent) = agent {
                app.show_activity = true;
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
        "activity" => {
            app.show_activity = match argument.trim() {
                "" => !app.show_activity,
                "on" => true,
                "off" => false,
                _ => {
                    app.status = "Usage: /activity [on|off]".into();
                    return Ok(true);
                }
            };
            app.status = if app.show_activity {
                "Activity visible · Ctrl+L hides it"
            } else {
                "Activity hidden · Ctrl+L shows it"
            }
            .into();
        }
        "model" if argument.trim().is_empty() => {
            app.status = format!(
                "Current model: {} · Ctrl+M opens the model picker",
                app.session.model
            )
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
        "models" if argument.trim().is_empty() => {
            if let (Some(agent), Some(tx)) = (agent, tx) {
                app.model_picker = true;
                app.model_manual = false;
                app.model_filter = Composer::default();
                app.selected_model = 0;
                request_models(tx, (*agent).clone(), false);
            } else {
                app.status = "Model picker unavailable while runtime is starting".into();
            }
        }
        "models" if argument.trim() == "json" => {
            request_cli(
                app,
                store,
                vec!["models".into(), "--json".into()],
                true,
                true,
            )
            .await?;
        }
        "models" => app.status = "Usage: /models [json]".into(),
        "sessions" => {
            app.sessions = store.list().await?;
            app.selected_session = 0;
            app.show_sessions = true;
        }
        "resume" if !argument.trim().is_empty() => {
            match store.load_reference(argument.trim()).await {
                Ok(session) => {
                    request_cli(
                        app,
                        store,
                        vec!["chat".into(), "--resume".into(), session.id.to_string()],
                        true,
                        false,
                    )
                    .await?;
                }
                Err(error) => app.status = format!("Cannot resume session: {error}"),
            }
        }
        "resume" => app.status = "Usage: /resume SESSION".into(),
        "new" => start_new_session(
            app,
            (!argument.trim().is_empty()).then_some(argument.trim()),
        ),
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
                expand_user_path(argument.trim())
            };
            store.export_markdown(&app.session, &path).await?;
            app.status = format!("Exported to {}", path.display());
        }
        "auth" => {
            let words = parse_words(
                argument,
                "/auth status|login [--device]|logout|import-codex",
            )?;
            let valid = matches!(words.as_slice(), [action] if matches!(action.as_str(), "status" | "logout"))
                || matches!(words.as_slice(), [action] if action == "login")
                || matches!(words.as_slice(), [action, flag] if action == "login" && flag == "--device")
                || words.first().is_some_and(|action| action == "import-codex");
            if !valid {
                app.status =
                    "Usage: /auth status|login [--device]|logout|import-codex [--path PATH] [--force]"
                        .into();
            } else {
                let mut arguments = vec!["auth".into()];
                arguments.extend(words);
                request_cli(app, store, arguments, true, true).await?;
            }
        }
        "doctor" => {
            request_cli(app, store, vec!["doctor".into()], true, true).await?;
        }
        "verbose" => {
            let value = argument.trim();
            if !matches!(value, "on" | "off") {
                app.status = "Usage: /verbose on|off".into();
            } else {
                let arguments = resume_chat_arguments(app);
                request_cli(app, store, arguments, true, false).await?;
                if let Some(TuiExit::Launch(request)) = &mut app.exit {
                    request.verbose = Some(value == "on");
                }
            }
        }
        "log-format" => {
            let format = argument.trim();
            if !matches!(format, "text" | "json") {
                app.status = "Usage: /log-format text|json".into();
            } else {
                let arguments = resume_chat_arguments(app);
                request_cli(app, store, arguments, true, false).await?;
                if let Some(TuiExit::Launch(request)) = &mut app.exit {
                    request.log_format = Some(format.into());
                }
            }
        }
        "plain" => {
            let mut arguments = resume_chat_arguments(app);
            arguments.push("--plain".into());
            request_cli(app, store, arguments, true, false).await?;
        }
        "run" if !argument.trim().is_empty() => {
            let mut words = parse_words(argument, "/run [--no-save] PROMPT")?;
            let no_save = words.first().is_some_and(|word| word == "--no-save");
            if no_save {
                words.remove(0);
            }
            if words.is_empty() {
                app.status = "Usage: /run [--no-save] PROMPT".into();
            } else {
                let mut arguments =
                    vec!["run".into(), "--resume".into(), app.session.id.to_string()];
                if no_save {
                    arguments.push("--no-save".into());
                }
                arguments.extend(words);
                request_cli(app, store, arguments, true, true).await?;
            }
        }
        "run" => app.status = "Usage: /run [--no-save] PROMPT".into(),
        "voyage" => {
            let words = parse_words(argument, "/voyage [URL] [NAME]")?;
            if words.len() > 2 {
                app.status = "Usage: /voyage [URL] [NAME]".into();
            } else {
                let mut arguments = vec![match words.first() {
                    Some(url) => format!("--voyage={url}"),
                    None => "--voyage".into(),
                }];
                if let Some(name) = words.get(1) {
                    arguments.extend(["--name".into(), name.clone()]);
                }
                request_cli(app, store, arguments, true, false).await?;
            }
        }
        "completions" => {
            let shell = argument.trim();
            if !matches!(shell, "bash" | "elvish" | "fish" | "powershell" | "zsh") {
                app.status = "Usage: /completions bash|elvish|fish|powershell|zsh".into();
            } else {
                request_cli(
                    app,
                    store,
                    vec!["completions".into(), shell.into()],
                    true,
                    true,
                )
                .await?;
            }
        }
        "manpage" => {
            request_cli(app, store, vec!["manpage".into()], true, true).await?;
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

    #[test]
    fn supervisor_summary_separates_active_agents_from_retained_history() {
        let running = agent(AgentId(Uuid::new_v4()), None, "running");
        let mut completed = agent(AgentId(Uuid::new_v4()), None, "completed");
        completed.status = AgentStatus::Completed;
        let mut timed_out = agent(AgentId(Uuid::new_v4()), None, "timed out");
        timed_out.status = AgentStatus::TimedOut;
        let agents = vec![running, completed, timed_out];

        assert_eq!(supervisor_counts(&agents), (1, 2));
        assert_eq!(
            supervisor_summary(&agents),
            "Supervising 1 active agent · 2 retained"
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
        assert_eq!(
            compact_line(
                "HTTP 400: {\n  \"error\": {\n    \"message\": \"No tool call found\"\n  }\n}",
                200,
            ),
            "HTTP 400: { \"error\": { \"message\": \"No tool call found\" } }"
        );
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
    fn submitted_prompt_is_visible_while_activity_is_opt_in() {
        let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        session
            .messages
            .push(crate::Message::new(Role::User, "inspect this now"));
        let mut app = App::new(session, Vec::new());
        app.activity.push("▶ shell: noisy internal detail".into());

        let hidden = transcript(&app, 60).to_string();
        assert!(hidden.contains("inspect this now"));
        assert!(!hidden.contains("noisy internal detail"));

        app.show_activity = true;
        let visible = transcript(&app, 60).to_string();
        assert!(visible.contains("noisy internal detail"));
    }

    #[tokio::test]
    async fn failed_run_does_not_remove_submitted_prompt_from_session() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = Session::new(directory.path().into(), "test-model".into());
        session
            .messages
            .push(crate::Message::new(Role::User, "keep this request"));
        let mut app = App::new(session, Vec::new());
        let store = SessionStore::new(directory.path().join("sessions"));
        store.save(&mut app.session).await.unwrap();

        handle_ui_event(
            UiEvent::Finished(Err("provider unavailable".into())),
            &mut app,
            &store,
            &crate::terminal::NoInteractiveTerminals::default(),
        )
        .await
        .unwrap();

        assert_eq!(app.session.messages.last().unwrap().role, Role::User);
        assert_eq!(
            app.session.messages.last().unwrap().content,
            "keep this request"
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
    fn markdown_full_screen_buffer_uses_borderless_chat_regions() {
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
        assert!(!snapshot.contains("Conversation"));
        assert!(!snapshot.contains("Prompt"));
        assert!(rows.iter().all(|row| row.chars().count() == 48));
    }

    #[test]
    fn global_ctrl_shortcuts_are_hidden_until_f1_help_is_opened() {
        let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        let mut app = App::new(session, Vec::new());
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let normal: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(normal.contains("F1 shortcuts"));
        assert!(!normal.contains("Ctrl+D"));
        assert!(!normal.contains("^D todos"));

        assert!(handle_shortcut_help_key(
            KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
            &mut app,
        ));
        assert!(app.shortcut_help);
        assert!(handle_shortcut_help_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            &mut app,
        ));
        assert!(app.composer.text.is_empty());
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let help: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(help.contains("Conversation shortcuts"));
        assert!(help.contains("Ctrl+D: todos"));

        assert!(handle_shortcut_help_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut app,
        ));
        assert!(!app.shortcut_help);
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
    fn mouse_wheel_scrolls_conversation_within_transcript_bounds() {
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
        let wheel_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let wheel_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            ..wheel_up
        };

        handle_mouse(wheel_up, &mut app);
        assert_eq!(app.scroll, 3);
        for _ in 0..100 {
            handle_mouse(wheel_up, &mut app);
        }
        assert_eq!(app.scroll, max_conversation_scroll(&app));
        handle_mouse(wheel_down, &mut app);
        assert_eq!(app.scroll, max_conversation_scroll(&app) - 3);

        app.show_sessions = true;
        let previous = app.scroll;
        handle_mouse(wheel_down, &mut app);
        assert_eq!(app.scroll, previous);
    }

    #[test]
    fn composer_has_a_separator_above_its_input_area() {
        let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        let app = App::new(session, Vec::new());
        let backend = ratatui::backend::TestBackend::new(48, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let separator = (0..buffer.area.width)
            .map(|x| buffer[(x, 24)].symbol())
            .collect::<String>();
        assert!(separator.chars().all(|character| character == '─'));
    }

    #[test]
    fn slash_palette_lists_all_commands_above_the_composer_and_filters() {
        let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        let mut app = App::new(session, Vec::new());
        app.composer.insert('/');
        assert_eq!(slash_palette_matches(&app).len(), SLASH_COMMANDS.len());

        let backend = ratatui::backend::TestBackend::new(100, 30);
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
        assert!(snapshot.contains("Commands"));
        for command in SLASH_COMMANDS {
            assert!(
                snapshot.contains(command.usage),
                "palette omitted {}",
                command.usage
            );
        }
        let palette_row = rows
            .iter()
            .position(|row| row.contains("Commands"))
            .unwrap();
        assert!(palette_row < 24);

        app.composer.text = "/co".into();
        app.composer.cursor = app.composer.text.len();
        let matches = slash_palette_matches(&app);
        assert_eq!(
            matches
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["config", "compact", "completions"]
        );
    }

    #[test]
    fn slash_palette_offers_contextual_values_and_dynamic_records() {
        let mut first = Session::new(PathBuf::from("/tmp"), "test-model".into());
        first.name = Some("deployment notes".into());
        let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        let mut app = App::new(session, vec![first.clone()]);

        app.composer.insert_str("/access ");
        assert_eq!(
            slash_palette_items(&app)
                .iter()
                .map(|item| item.usage.as_str())
                .collect::<Vec<_>>(),
            ["read-only", "approval", "unrestricted"]
        );
        app.selected_slash_command = 1;
        complete_selected_slash_command(&mut app);
        assert_eq!(app.composer.text, "/access approval");
        assert!(slash_input_is_complete(&app));

        app.models = vec![ModelInfo::minimal("gpt-dynamic")];
        app.composer = Composer::default();
        app.composer.insert_str("/models dynamic");
        let models = slash_palette_items(&app);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].completion, "/model gpt-dynamic");

        app.composer = Composer::default();
        app.composer.insert_str("/resume deploy");
        let sessions = slash_palette_items(&app);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].completion, format!("/resume {}", first.id));

        app.composer = Composer::default();
        app.composer.insert_str("/set ");
        assert_eq!(slash_palette_items(&app).len(), CONFIG_OVERRIDE_SPECS.len());
        assert!(
            slash_palette_items(&app)
                .iter()
                .any(|item| item.usage == "codex_command")
        );

        app.composer = Composer::default();
        app.composer.insert_str("/set access ");
        assert_eq!(
            slash_palette_items(&app)
                .iter()
                .map(|item| item.usage.as_str())
                .collect::<Vec<_>>(),
            ["read-only", "approval", "unrestricted"]
        );

        for input in ["/name ", "/branch ", "/run ", "/voyage "] {
            app.composer = Composer::default();
            app.composer.insert_str(input);
            assert!(
                !slash_palette_items(&app).is_empty(),
                "{input} should explain or suggest its next argument"
            );
        }

        app.composer = Composer::default();
        app.composer.insert_str("/set provider_retry_attempts ");
        assert!(
            slash_palette_items(&app)
                .iter()
                .any(|item| item.usage == "0")
        );

        app.composer = Composer::default();
        app.composer.insert_str("/provider codex-");
        assert!(
            slash_palette_items(&app)
                .iter()
                .any(|item| item.usage == "codex-subscription")
        );

        app.composer = Composer::default();
        app.composer.insert_str("/run --no-save ");
        assert_eq!(slash_palette_items(&app)[0].usage, "<PROMPT>");
    }

    #[test]
    fn slash_palette_completes_filesystem_arguments() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("project-alpha")).unwrap();
        std::fs::write(directory.path().join("helm-special.toml"), "model = 'test'").unwrap();
        let session = Session::new(directory.path().into(), "test-model".into());
        let mut app = App::new(session, Vec::new());

        app.composer.insert_str("/workspace pro");
        let workspace = slash_palette_items(&app);
        assert_eq!(workspace.len(), 1);
        assert_eq!(workspace[0].completion, "/workspace project-alpha/");

        app.composer = Composer::default();
        app.composer.insert_str("/config helm-");
        let config = slash_palette_items(&app);
        assert_eq!(config.len(), 1);
        assert_eq!(config[0].completion, "/config helm-special.toml");
    }

    #[test]
    fn slash_palette_completion_and_dismissal_preserve_composer_input() {
        let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
        let mut app = App::new(session, Vec::new());
        app.composer.insert_str("/na");
        complete_selected_slash_command(&mut app);
        assert_eq!(app.composer.text, "/name ");
        assert_eq!(app.composer.cursor, app.composer.text.len());
        assert!(slash_palette_matches(&app).is_empty());

        app.composer = Composer::default();
        app.composer.insert_str("/help");
        assert!(slash_input_is_complete(&app));
        app.slash_palette_dismissed = true;
        assert!(slash_palette_matches(&app).is_empty());
        assert_eq!(app.composer.text, "/help");
        reset_slash_palette(&mut app);
        assert_eq!(slash_palette_matches(&app).len(), 1);
    }

    #[tokio::test]
    async fn startup_only_controls_create_explicit_session_preserving_handoffs() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));

        let mut access = App::new(
            Session::new(directory.path().into(), "test-model".into()),
            Vec::new(),
        );
        let access_id = access.session.id.to_string();
        handle_command("/access unrestricted", &mut access, &store, None, None)
            .await
            .unwrap();
        assert_eq!(
            access.exit,
            Some(TuiExit::Launch(CliRequest {
                arguments: vec![
                    "--access".into(),
                    "unrestricted".into(),
                    "chat".into(),
                    "--resume".into(),
                    access_id,
                ],
                use_active_config: true,
                resume_after: false,
                verbose: None,
                log_format: None,
            }))
        );

        let mut setting = App::new(
            Session::new(directory.path().into(), "test-model".into()),
            Vec::new(),
        );
        handle_command("/set max_turns 32", &mut setting, &store, None, None)
            .await
            .unwrap();
        let Some(TuiExit::Launch(request)) = setting.exit else {
            panic!("expected CLI handoff");
        };
        assert_eq!(request.arguments[0..2], ["--set", "max_turns=32"]);
        assert!(request.use_active_config);

        let mut doctor = App::new(
            Session::new(directory.path().into(), "test-model".into()),
            Vec::new(),
        );
        handle_command("/doctor", &mut doctor, &store, None, None)
            .await
            .unwrap();
        let Some(TuiExit::Launch(request)) = doctor.exit else {
            panic!("expected CLI handoff");
        };
        assert_eq!(request.arguments, ["doctor"]);
        assert!(request.resume_after);
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
        handle_command("/clear", &mut app, &store, None, None)
            .await
            .unwrap();
        assert_eq!(app.session.messages.len(), 1);
        handle_command("/clear confirm", &mut app, &store, None, None)
            .await
            .unwrap();
        assert!(app.session.messages.is_empty());
    }

    #[tokio::test]
    async fn new_command_starts_named_empty_session() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let mut session = Session::new(directory.path().into(), "test-model".into());
        session
            .messages
            .push(crate::Message::new(Role::User, "old conversation"));
        let old_id = session.id;
        let mut app = App::new(session, Vec::new());

        handle_command("/new field work", &mut app, &store, None, None)
            .await
            .unwrap();

        assert_ne!(app.session.id, old_id);
        assert_eq!(app.session.name.as_deref(), Some("field work"));
        assert!(app.session.messages.is_empty());
        assert_eq!(app.session.model, "test-model");
        assert_eq!(app.session.workspace, directory.path());
    }

    #[tokio::test]
    async fn local_commands_name_and_export_session() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let session = Session::new(directory.path().into(), "test-model".into());
        let mut app = App::new(session, Vec::new());
        assert!(
            handle_command("/name field work", &mut app, &store, None, None)
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
            None,
        )
        .await
        .unwrap();
        assert!(export.exists());
    }
    fn question_dialog() -> (
        QuestionDialog,
        oneshot::Receiver<crate::tools::QuestionAnswer>,
    ) {
        let (response, receive) = oneshot::channel();
        (
            QuestionDialog {
                request: QuestionRequest {
                    question: crate::tools::Question {
                        question: "Which format?".into(),
                        options: vec!["JSON".into(), "Markdown".into()],
                    },
                    response,
                },
                selected: 0,
                custom: Composer::default(),
                scroll: None,
            },
            receive,
        )
    }

    #[test]
    fn questions_keyboard_selection_custom_editing_paste_and_limits() {
        use crate::tools::QuestionAnswer;
        let (mut dialog, _receive) = question_dialog();
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        dialog.insert("not on custom");
        assert!(dialog.custom.text.is_empty());
        dialog.key(key(KeyCode::Down));
        assert_eq!(
            dialog.key(key(KeyCode::Enter)),
            Some(QuestionAnswer::Selected {
                index: 1,
                answer: "Markdown".into()
            })
        );
        dialog.key(key(KeyCode::Tab));
        assert!(dialog.key(key(KeyCode::Enter)).is_none());
        dialog.insert("日本語\n\u{001b}🛶");
        assert_eq!(dialog.custom.text, "日本語🛶");
        dialog.key(key(KeyCode::Left));
        dialog.key(key(KeyCode::Backspace));
        dialog.key(key(KeyCode::Char('文')));
        assert_eq!(dialog.custom.text, "日本文🛶");
        assert_eq!(
            dialog.key(key(KeyCode::Enter)),
            Some(QuestionAnswer::Custom {
                answer: "日本文🛶".into()
            })
        );
        dialog.key(key(KeyCode::Up));
        dialog.key(key(KeyCode::Down));
        assert_eq!(dialog.custom.text, "日本文🛶");
        dialog.insert(&"x".repeat(5000));
        assert!(dialog.custom.text.len() <= crate::tools::MAX_ANSWER_BYTES);
        assert_eq!(
            dialog.key(key(KeyCode::Esc)),
            Some(QuestionAnswer::Cancelled)
        );
    }

    #[tokio::test]
    async fn questions_modal_routes_keys_without_touching_draft_or_other_views() {
        use crate::tools::QuestionAnswer;
        let mut app = App::new(Session::new(PathBuf::from("/tmp"), "test".into()), vec![]);
        app.composer.insert_str("unsent draft");
        app.shortcut_help = true;
        app.model_picker = true;
        let (dialog, receive) = question_dialog();
        app.question = Some(dialog);
        for code in [
            KeyCode::F(1),
            KeyCode::Char('x'),
            KeyCode::Down,
            KeyCode::Enter,
        ] {
            assert!(handle_question_key(
                KeyEvent::new(code, KeyModifiers::NONE),
                &mut app
            ));
        }
        assert_eq!(
            receive.await.unwrap(),
            QuestionAnswer::Selected {
                index: 1,
                answer: "Markdown".into()
            }
        );
        assert_eq!(app.composer.text, "unsent draft");
        assert!(app.shortcut_help && app.model_picker);
        assert!(app.question.is_none());
        let (dialog, receive) = question_dialog();
        app.question = Some(dialog);
        app.cancel();
        assert_eq!(receive.await.unwrap(), QuestionAnswer::Cancelled);
        let (dialog, receive) = question_dialog();
        drop(receive);
        app.question = Some(dialog);
        assert!(handle_question_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app
        ));
        assert!(app.question.is_none());
        assert_eq!(app.composer.text, "unsent draft");
    }

    #[tokio::test]
    async fn questions_reject_concurrent_requests_and_attached_terminal_input() {
        use crate::tools::QuestionAnswer;
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let terminals = FakeTerminals::new();
        let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
        app.attached_terminal = Some(terminals.id);
        let (dialog, receive) = question_dialog();
        handle_ui_event(
            UiEvent::Question(dialog.request),
            &mut app,
            &store,
            &terminals,
        )
        .await
        .unwrap();
        assert_eq!(receive.await.unwrap(), QuestionAnswer::Unavailable);
        assert!(app.question.is_none());
        app.attached_terminal = None;
        let (dialog, first) = question_dialog();
        handle_ui_event(
            UiEvent::Question(dialog.request),
            &mut app,
            &store,
            &terminals,
        )
        .await
        .unwrap();
        let (dialog, second) = question_dialog();
        handle_ui_event(
            UiEvent::Question(dialog.request),
            &mut app,
            &store,
            &terminals,
        )
        .await
        .unwrap();
        assert_eq!(second.await.unwrap(), QuestionAnswer::Unavailable);
        app.cancel();
        assert_eq!(first.await.unwrap(), QuestionAnswer::Cancelled);
        let (dialog, stale) = question_dialog();
        drop(stale);
        handle_ui_event(
            UiEvent::Question(dialog.request),
            &mut app,
            &store,
            &terminals,
        )
        .await
        .unwrap();
        assert!(app.question.is_none());
        assert!(terminals.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn questions_bridge_delivers_answers_and_handles_closed_frontend() {
        let (bridge, mut events) = bridge();
        let (dialog, _) = question_dialog();
        let task_bridge = bridge.clone();
        let question = dialog.request.question;
        let task = tokio::spawn(async move { task_bridge.ask_question(&question).await });
        let Some(UiEvent::Question(request)) = events.recv().await else {
            panic!("missing question event")
        };
        assert_eq!(request.question.question, "Which format?");
        request
            .response
            .send(crate::tools::QuestionAnswer::Custom {
                answer: "CSV".into(),
            })
            .unwrap();
        assert_eq!(
            task.await.unwrap(),
            crate::tools::QuestionAnswer::Custom {
                answer: "CSV".into()
            }
        );
        drop(events);
        let (dialog, _) = question_dialog();
        assert_eq!(
            bridge.ask_question(&dialog.request.question).await,
            crate::tools::QuestionAnswer::Unavailable
        );
    }

    #[test]
    fn questions_render_safely_and_remain_visible_over_other_views_at_all_sizes() {
        use ratatui::backend::TestBackend;
        let mut app = App::new(Session::new(PathBuf::from("/tmp"), "test".into()), vec![]);
        let (mut dialog, _receive) = question_dialog();
        dialog.request.question.question = "Untrusted \u{001b}[2J 日本語 é ".repeat(30);
        dialog.selected = 2;
        dialog.insert("custom-text");
        app.question = Some(dialog);
        app.model_picker = true;
        app.shortcut_help = true;
        for (width, height) in [(1, 1), (20, 6), (32, 10), (80, 24), (120, 40)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(!rendered.contains('\u{001b}'));
            if width >= 32 {
                assert!(rendered.contains("custom-text"));
            }
        }
    }
}
