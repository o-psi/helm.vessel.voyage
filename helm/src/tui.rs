//! Full-screen terminal frontend and cross-feature coordinator.
//!
//! Feature panels own their state and take explicit dependencies; this module
//! retains the event loop, cancellation and auditable input/modal precedence.
//! Public entrypoints are unchanged. See `docs/tui-architecture.md` for ownership
//! boundaries and regression checks.

mod bridge;
pub use bridge::{ApprovalRequest, QuestionRequest, UiBridge, UiEvent, bridge};
mod composer;
use composer::*;
mod questions;
use questions::*;
mod lifecycle;
use lifecycle::*;
mod palette;
use palette::*;
mod todos;
use todos::*;
mod supervisor;
use supervisor::*;
mod models;
use models::*;
mod terminals;
use terminals::*;
mod checkpoint;
mod conversation;
use conversation::*;
mod render;
use render::*;
mod text;
mod tool_output;
use text::*;
mod commands;
use commands::*;

use crate::{
    Agent, AgentEvent,
    agent::{SteeringSender, steering_channel},
    config::AccessMode,
    markdown::MarkdownTheme,
    model::Role,
    session::{Session, SessionStore, compact_messages},
    supervision::AgentSupervisor,
    terminal::InteractiveTerminals,
    todo::TodoStore,
    tools::ApprovalOutcome,
};
use anyhow::{Context, Result};
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

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

struct App {
    diagnostic_agent: Option<Arc<Agent>>,
    palette: PaletteState,
    model_panel: ModelPanel,
    terminal_panel: TerminalPanel,
    supervisor_panel: SupervisorPanel,
    todo_panel: TodoPanel,
    session: Session,
    sessions: Vec<Session>,
    provider_label: String,
    access_mode: AccessMode,
    composer: Composer,
    prompt_history: PromptHistory,
    activity: Vec<String>,
    show_activity: bool,
    tool_details: bool,
    live_messages: Vec<crate::Message>,
    working_since: Instant,
    streaming_response: String,
    status: String,
    scroll: u16,
    conversation_width: usize,
    conversation_height: usize,
    markdown_theme: MarkdownTheme,
    markdown_syntax_highlighting: bool,
    running: Option<Running>,
    checkpoint: Option<checkpoint::State>,
    title_job: Option<TitleJob>,
    approval: Option<ApprovalRequest>,
    question: Option<QuestionDialog>,
    show_sessions: bool,
    selected_session: usize,
    shortcut_help: bool,
    exit: Option<TuiExit>,
    quit: bool,
}

struct TitleJob {
    task: tokio::task::JoinHandle<()>,
    cancel: tokio_util::sync::CancellationToken,
}

impl Drop for TitleJob {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

fn start_title_job(app: &mut App, agent: &Arc<Agent>, tx: &mpsc::UnboundedSender<UiEvent>) {
    app.title_job = None;
    let session_id = app.session.id;
    let completed_runs = app
        .session
        .title_state
        .as_ref()
        .expect("completed run")
        .completed_runs;
    let messages = app.session.messages.clone();
    let agent = agent.clone();
    let events = tx.clone();
    let cancel = tokio_util::sync::CancellationToken::new();
    let token = cancel.clone();
    let task = tokio::spawn(async move {
        if let Some(result) = agent.generate_title(&messages, token).await {
            let _ = events.send(UiEvent::TitleReady {
                session_id,
                completed_runs,
                result: Some(result),
            });
        }
    });
    app.title_job = Some(TitleJob { task, cancel });
}

struct Running {
    task: tokio::task::JoinHandle<()>,
    cancel: tokio_util::sync::CancellationToken,
    steering: SteeringSender,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

impl App {
    fn insert_composer(&mut self, text: &str) {
        if self.is_running()
            && self.composer.text.len().saturating_add(text.len())
                > crate::agent::MAX_STEERING_BYTES
        {
            self.status = "Steering input limit is 64 KiB; draft preserved".into();
        } else {
            self.composer.insert_str(text);
        }
    }

    fn palette_context(&self) -> PaletteContext<'_> {
        PaletteContext {
            input: &self.composer.text,
            running: self.is_running(),
            dismissed: self.palette.slash_palette_dismissed,
            selected: self.palette.selected_slash_command,
            models: &self.model_panel.models,
            sessions: &self.sessions,
            workspace: &self.session.workspace,
        }
    }

    fn new(session: Session, sessions: Vec<Session>) -> Self {
        let mut prompt_history = PromptHistory::default();
        for message in &session.messages {
            if message.role == Role::User {
                prompt_history.record(&message.content);
            }
        }
        Self {
            diagnostic_agent: None,
            palette: PaletteState::default(),
            model_panel: ModelPanel::default(),
            terminal_panel: TerminalPanel::default(),
            supervisor_panel: SupervisorPanel::default(),
            todo_panel: TodoPanel::default(),
            session,
            sessions,
            provider_label: "provider unknown".into(),
            access_mode: AccessMode::Approval,
            composer: Composer::default(),
            prompt_history,
            activity: Vec::new(),
            show_activity: false,
            tool_details: false,
            live_messages: Vec::new(),
            working_since: Instant::now(),
            streaming_response: String::new(),
            status: "Ready".into(),
            scroll: 0,
            conversation_width: 78,
            conversation_height: 13,
            markdown_theme: markdown_theme(),
            markdown_syntax_highlighting: std::env::var_os("NO_COLOR").is_none(),
            running: None,
            checkpoint: None,
            title_job: None,
            approval: None,
            question: None,
            show_sessions: false,
            selected_session: 0,
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
            self.status = "Cancelling; partial response retained as interrupted output".into();
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
    store: &mut SessionStore,
    session: Session,
    mut rx: mpsc::UnboundedReceiver<UiEvent>,
    tx: mpsc::UnboundedSender<UiEvent>,
    terminals: Arc<dyn InteractiveTerminals>,
    supervisor: Arc<dyn AgentSupervisor>,
    todos: Arc<TodoStore>,
    provider_label: String,
    access_mode: AccessMode,
) -> Result<TuiExit> {
    anyhow::ensure!(
        store.owned_session_id() == Some(session.id),
        "TUI session requires execution ownership"
    );
    let sessions = store.list().await?;
    let mut app = App::new(session, sessions);
    app.diagnostic_agent = Some(agent.clone());
    app.provider_label = provider_label;
    app.access_mode = access_mode;
    refresh_terminals(&mut app.terminal_panel, &mut app.status, terminals.as_ref()).await;
    let mut terminal_events = Some(terminals.subscribe());
    let mut supervisor_events = Some(supervisor.subscribe());
    let mut todo_refresh = tokio::time::interval(std::time::Duration::from_secs(1));
    let _guard = TerminalGuard::enter().context("failed to initialize terminal")?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    // Fullscreen redraws need no cursor-position query from the terminal.
    terminal.resize(terminal.size()?.into())?;
    if let Ok(size) = terminal.size() {
        app.conversation_width = size.width.max(1) as usize;
        app.conversation_height = size.height.saturating_sub(9).max(1) as usize;
    }
    let mut animation = tokio::time::interval(Duration::from_millis(100));
    animation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
        let size = terminal.size()?;
        let viewport = conversation_layout(
            ratatui::layout::Rect::new(0, 0, size.width, size.height),
            &app,
        )[1];
        if app.conversation_width != viewport.width.max(1) as usize
            || app.conversation_height != viewport.height.max(1) as usize
        {
            resize_conversation(&mut app, viewport.width as usize, viewport.height as usize);
        }
        terminal.draw(|frame| draw(frame, &app))?;
        tokio::select! {
            event = input.next() => {
                match event {
                    Some(Ok(Event::Key(key))) if key.is_press() => {
                        handle_key(key, &mut app, &agent, store, &tx, terminals.as_ref(), supervisor.clone(), todos.clone()).await?;
                    }
                    Some(Ok(Event::Resize(columns, rows))) => {
                        let viewport = conversation_layout(
                            ratatui::layout::Rect::new(0, 0, columns, rows), &app,
                        )[1];
                        resize_conversation(&mut app, viewport.width as usize, viewport.height as usize);
                        if let Some(id) = app.terminal_panel.attached_terminal {
                            let _ = terminals.resize(id, columns, rows.saturating_sub(1)).await;
                        }
                        // Discard any stale cells after the terminal changes its backing grid.
                        terminal.resize(ratatui::layout::Rect::new(0, 0, columns, rows))?;
                    }
                    Some(Ok(Event::Mouse(mouse))) => handle_mouse(mouse, &mut app),
                    Some(Ok(Event::Paste(text))) if app.question.is_some() => {
                        if let Some(question) = &mut app.question { question.insert(&text); }
                    }
                    Some(Ok(Event::Paste(text))) if app.approval.is_none() && !app.show_sessions => {
                        if let Some(id) = app.terminal_panel.attached_terminal {
                            if let Err(error) = terminals.write(id, text.into_bytes()).await {
                                app.status = format!("Terminal input failed: {error}");
                            }
                        } else if !app.terminal_panel.terminal_picker {
                            let text = text.replace("\r\n", "\n").replace('\r', "\n");
                            if app.model_panel.model_picker {
                                app.model_panel.model_filter.insert_str(&text);
                                app.model_panel.selected_model = 0;
                            } else if matches!(app.supervisor_panel.supervisor_mode, Some(SupervisorMode::Message { .. })) {
                                app.supervisor_panel.supervisor_input.insert_str(&text);
                            } else if matches!(app.todo_panel.todo_mode, Some(TodoMode::Input { .. })) {
                                app.todo_panel.todo_input.insert_str(&text);
                            } else if app.supervisor_panel.supervisor_mode.is_none() {
                                app.insert_composer(&text);
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
                let title_due = matches!(&event, UiEvent::Finished(Ok(outcome)) if matches!(outcome.stop_reason, crate::agent::StopReason::Completed))
                    && app.session.title_due_after_turn();
                handle_ui_event(event, &mut app, store, terminals.as_ref()).await?;
                if title_due { start_title_job(&mut app, &agent, &tx); }
            }
            _ = &mut termination => {
                app.status = "Terminal closing; cancelling active work".into();
                app.quit = true;
            }
            event = async { terminal_events.as_mut().expect("guarded terminal receiver").recv().await }, if terminal_events.is_some() => {
                match event {
                    Ok(event) => handle_terminal_event(event, &mut app.terminal_panel, &mut app.status, terminals.as_ref()).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => refresh_terminals(&mut app.terminal_panel, &mut app.status, terminals.as_ref()).await,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        terminal_events = None;
                        app.terminal_panel.attached_terminal = None;
                        app.terminal_panel.terminal_snapshot = None;
                        app.status = "Terminal manager disconnected".into();
                    }
                }
            }
            event = async { supervisor_events.as_mut().expect("guarded supervisor receiver").recv().await }, if supervisor_events.is_some() => {
                match event {
                    Ok(event) => {
                        if matches!(app.supervisor_panel.supervisor_mode, Some(SupervisorMode::Inspect(id)) if id == event.agent_id) {
                            app.supervisor_panel.inspected_events.push(event);
                            if app.supervisor_panel.inspected_events.len() > 500 { app.supervisor_panel.inspected_events.drain(..100); }
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
            _ = animation.tick(), if app.is_running() => {}
            _ = todo_refresh.tick(), if app.todo_panel.todo_mode.is_some() => request_todo_snapshot(&tx, todos.clone()),
        }
    }
    app.cancel();
    terminal.show_cursor()?;
    Ok(app.exit.unwrap_or_default())
}

async fn handle_ui_event(
    event: UiEvent,
    app: &mut App,
    store: &SessionStore,
    terminals: &dyn InteractiveTerminals,
) -> Result<()> {
    match event {
        UiEvent::Checkpoint(request) => checkpoint::handle(request, app, store).await,
        UiEvent::Agent(AgentEvent::CompletionState {
            phase,
            readiness,
            detail,
        }) => {
            if matches!(phase, crate::agent::CompletionPhase::Reconciling)
                && !app.streaming_response.is_empty()
            {
                app.live_messages.push(crate::Message::new(
                    Role::Assistant,
                    std::mem::take(&mut app.streaming_response),
                ));
            }
            let detail = detail.map(|text| {
                app.diagnostic_agent
                    .as_ref()
                    .map(|agent| agent.redact_diagnostic(&text))
                    .unwrap_or(text)
            });
            let label = format!("{phase:?}").to_lowercase();
            app.status = format!(
                "Run {label}{}",
                detail
                    .as_deref()
                    .map(|text| format!(" · {}", compact_line(text, 160)))
                    .unwrap_or_default()
            );
            // Only the acknowledged checkpoint or Finished can classify canonical
            // messages; an early event alone cannot mark a saved answer final.
            if !matches!(
                phase,
                crate::agent::CompletionPhase::Completed
                    | crate::agent::CompletionPhase::Incomplete
            ) {
                app.session.update_run_summary(
                    phase,
                    readiness,
                    detail.map(|text| compact_line(&text, 4000)),
                );
                store.save(&mut app.session).await?;
            }
        }
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
            if app.checkpoint.is_some() {
                app.status = "Response saved · checking remaining work".into();
                return Ok(());
            }
            let before = transcript_height(app, app.conversation_width);
            app.streaming_response = text.clone();
            preserve_manual_anchor(app, before);
            app.status = "Receiving response…  Esc cancels".into();
        }
        UiEvent::Agent(AgentEvent::ToolStarted { name, arguments }) => {
            if app.checkpoint.is_some() {
                app.status = format!("Running {name}…  Esc cancels");
                return Ok(());
            }
            let before = transcript_height(app, app.conversation_width);
            if !app.streaming_response.is_empty() {
                app.live_messages.push(crate::Message::new(
                    Role::Assistant,
                    std::mem::take(&mut app.streaming_response),
                ));
            }
            let mut message = crate::Message::new(Role::Assistant, "");
            message.tool_calls.push(crate::model::ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.clone(),
                arguments,
            });
            app.live_messages.push(message);
            preserve_manual_anchor(app, before);
            app.status = format!("Running {name}…  Esc cancels");
        }
        UiEvent::Agent(AgentEvent::ToolFinished {
            name,
            result,
            success,
        }) => {
            if app.checkpoint.is_some() {
                return Ok(());
            }
            let before = transcript_height(app, app.conversation_width);
            if let Some(call) = app
                .live_messages
                .iter()
                .rev()
                .flat_map(|message| &message.tool_calls)
                .find(|call| call.name == name)
            {
                app.live_messages.push(crate::Message::tool_result(
                    call.id.clone(),
                    result,
                    success,
                ));
            }
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
        UiEvent::Agent(AgentEvent::ContextBudget(report)) => {
            app.status = format!(
                "Context: estimated {}/{} tokens; {} older messages omitted",
                report.estimated, report.limit, report.omitted_messages
            );
        }
        UiEvent::Agent(AgentEvent::SteeringApplied { mut history }) => {
            // The boundary snapshot carries real tool IDs and the completed response.
            // Keep later accepted inputs which have not reached this boundary yet.
            for message in &app.session.messages {
                if let Some(receipt) = &message.steering
                    && receipt.status == crate::model::SteeringStatus::Queued
                    && !history.iter().any(|item| {
                        item.steering
                            .as_ref()
                            .is_some_and(|other| other.id == receipt.id)
                    })
                {
                    history.push(message.clone());
                }
            }
            app.session.replace_messages(history);
            app.live_messages.clear();
            app.streaming_response.clear();
            store.save(&mut app.session).await?;
            app.status = "Steering applied · continuing…  Esc cancels".into();
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
            if app.terminal_panel.attached_terminal.is_some()
                || app.approval.is_some()
                || app.question.is_some()
            {
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
            if app.terminal_panel.attached_terminal.is_some() || app.question.is_some() {
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
            let checkpoint = app.checkpoint.take();
            match result {
                Ok(outcome) => {
                    app.live_messages.clear();
                    if let Some(checkpoint) = &checkpoint {
                        app.session.usage = checkpoint.baseline.clone();
                        app.session
                            .recover_context_failure(&crate::agent::CanonicalRecovery {
                                messages: outcome.messages,
                                usage: outcome.usage.clone(),
                            })?;
                    } else {
                        app.session.replace_messages(outcome.messages);
                        app.session.usage.input_tokens += outcome.usage.input_tokens;
                        app.session.usage.output_tokens += outcome.usage.output_tokens;
                    }
                    for message in &mut app.session.messages {
                        if let Some(receipt) = &mut message.steering
                            && receipt.status == crate::model::SteeringStatus::Queued
                        {
                            receipt.status = crate::model::SteeringStatus::NotApplied;
                        }
                    }
                    app.session.finish_run_summary(&outcome.stop_reason);
                    let completed =
                        matches!(outcome.stop_reason, crate::agent::StopReason::Completed);
                    if completed {
                        app.session.record_completed_turn();
                    }
                    app.session.terminals = terminals.list().await.unwrap_or_default();
                    store.save(&mut app.session).await?;
                    app.status = match outcome.stop_reason {
                        crate::agent::StopReason::Completed => {
                            format!("Completed · {} model turn(s)", outcome.turns)
                        }
                        crate::agent::StopReason::Incomplete { reason, .. } => {
                            format!("Incomplete · {}", compact_line(&reason, 160))
                        }
                    };
                }
                Err(error) => {
                    if let Some(recovery) = error.recovery() {
                        if let Some(checkpoint) = &checkpoint {
                            app.session.usage = checkpoint.baseline.clone();
                        }
                        app.session.recover_context_failure(recovery)?;
                        // All completed responses/tools are represented by canonical IDs.
                        app.live_messages.clear();
                        app.streaming_response.clear();
                    }
                    let error = app
                        .diagnostic_agent
                        .as_ref()
                        .map(|agent| agent.redact_diagnostic(error.to_string()))
                        .unwrap_or_else(|| error.to_string());
                    let detail = compact_line(&error, 1_000);
                    app.activity.push(format!("✗ provider error: {detail}"));
                    let mut undelivered = 0;
                    for message in &mut app.session.messages {
                        if let Some(receipt) = &mut message.steering
                            && receipt.status == crate::model::SteeringStatus::Queued
                        {
                            receipt.status = crate::model::SteeringStatus::NotApplied;
                            undelivered += 1;
                        }
                    }
                    if checkpoint.is_none() && !app.streaming_response.is_empty() {
                        app.session.messages.push(crate::Message::new(
                            Role::Assistant,
                            std::mem::take(&mut app.streaming_response),
                        ));
                    }
                    app.session.interrupt_run_summary(detail);
                    store.save(&mut app.session).await?;
                    app.status = if undelivered > 0 {
                        format!(
                            "Run stopped; {undelivered} steering message(s) not applied · {}",
                            compact_line(&error, 80)
                        )
                    } else {
                        format!("Error: {}", compact_line(&error, 120))
                    };
                }
            }
            app.streaming_response.clear();
            preserve_manual_anchor(app, before);
        }
        UiEvent::TitleReady {
            session_id,
            completed_runs,
            result,
        } => {
            // Delayed metadata cannot rename another session or supersede newer work.
            if app.session.id == session_id
                && app
                    .session
                    .title_state
                    .as_ref()
                    .is_some_and(|state| state.completed_runs == completed_runs)
                && !app.is_running()
            {
                app.title_job = None;
                if let Some(result) = result {
                    app.session.apply_generated_title(result);
                    store.save(&mut app.session).await?;
                    app.sessions = store.list().await?;
                }
            }
        }
        UiEvent::SupervisorTree(result) => match result {
            Ok(agents) => {
                app.supervisor_panel.agents = flatten_agent_tree(agents);
                app.supervisor_panel.selected_agent = app
                    .supervisor_panel
                    .selected_agent
                    .min(app.supervisor_panel.agents.len().saturating_sub(1));
                app.status = supervisor_summary(&app.supervisor_panel.agents);
            }
            Err(error) => app.status = format!("Supervisor refresh failed: {error}"),
        },
        UiEvent::SupervisorInspect(id, result) => match result {
            Ok(events) => {
                app.supervisor_panel.inspected_events = events;
                app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Inspect(id));
            }
            Err(error) => app.status = format!("Agent inspection failed: {error}"),
        },
        UiEvent::SupervisorAction(result) => {
            app.status =
                result.unwrap_or_else(|error| format!("Supervisor action failed: {error}"));
            app.supervisor_panel.cancel_armed = None;
        }
        UiEvent::TodoSnapshot(result) => match result {
            Ok(list)
                if list.revision != app.todo_panel.todo_revision
                    || app.todo_panel.todos.is_empty() =>
            {
                let selected = app
                    .todo_panel
                    .todos
                    .get(app.todo_panel.selected_todo)
                    .map(|item| item.id);
                app.todo_panel.todo_revision = list.revision;
                app.todo_panel.todos = list.ordered().into_iter().cloned().collect();
                app.todo_panel.selected_todo = selected
                    .and_then(|id| app.todo_panel.todos.iter().position(|item| item.id == id))
                    .unwrap_or_else(|| {
                        app.todo_panel
                            .selected_todo
                            .min(app.todo_panel.todos.len().saturating_sub(1))
                    });
            }
            Ok(_) => {}
            Err(error) => app.status = format!("Todo refresh failed: {error}"),
        },
        UiEvent::TodoAction(result) => {
            app.status = result.unwrap_or_else(|error| format!("Todo action failed: {error}"));
        }
        UiEvent::Models(result) => match result {
            Ok(models) => {
                app.model_panel.models = models;
                app.model_panel.selected_model = 0;
                app.status = format!("{} model(s) available", app.model_panel.models.len());
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
    store: &mut SessionStore,
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
    if let Some(id) = app.terminal_panel.attached_terminal {
        handle_attached_key(id, key, &mut app.terminal_panel, &mut app.status, terminals).await;
        return Ok(());
    }
    if handle_shortcut_help_key(key, app) {
        return Ok(());
    }
    if app.model_panel.model_picker {
        handle_model_key(
            key,
            &mut app.model_panel,
            &mut app.session,
            &mut app.status,
            agent,
            store,
            tx,
        )
        .await?;
        return Ok(());
    }
    if app.supervisor_panel.supervisor_mode.is_some() {
        handle_supervisor_key(
            key,
            &mut app.supervisor_panel,
            &mut app.status,
            tx,
            supervisor,
        )
        .await;
        return Ok(());
    }
    if app.todo_panel.todo_mode.is_some() {
        handle_todo_key(key, &mut app.todo_panel, tx, todos).await;
        return Ok(());
    }
    if app.terminal_panel.terminal_picker {
        match key.code {
            KeyCode::Esc | KeyCode::Char('t') => app.terminal_panel.terminal_picker = false,
            KeyCode::Up => {
                app.terminal_panel.selected_terminal =
                    app.terminal_panel.selected_terminal.saturating_sub(1)
            }
            KeyCode::Down => {
                app.terminal_panel.selected_terminal = (app.terminal_panel.selected_terminal + 1)
                    .min(app.terminal_panel.terminals.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(summary) = app
                    .terminal_panel
                    .terminals
                    .get(app.terminal_panel.selected_terminal)
                {
                    let id = summary.id;
                    match terminals.snapshot(id).await {
                        Ok(snapshot) => {
                            app.terminal_panel.attached_terminal = Some(id);
                            app.terminal_panel.terminal_snapshot = Some(snapshot);
                            app.terminal_panel.terminal_picker = false;
                        }
                        Err(error) => app.status = format!("Cannot attach: {error}"),
                    }
                }
            }
            KeyCode::Char('r') => {
                refresh_terminals(&mut app.terminal_panel, &mut app.status, terminals).await
            }
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
                refresh_terminals(&mut app.terminal_panel, &mut app.status, terminals).await;
                app.terminal_panel.terminal_picker = true;
            }
            KeyCode::Char('o') => {
                let previous = transcript_height(app, app.conversation_width);
                app.tool_details = !app.tool_details;
                preserve_manual_anchor(app, previous);
                app.status = if app.tool_details {
                    "Full tool details · Ctrl+O collapses output"
                } else {
                    "Tool previews · Ctrl+O expands output"
                }
                .into();
            }
            KeyCode::Char('l') => {
                app.show_activity = !app.show_activity;
                app.status = if app.show_activity {
                    "Full activity visible · Ctrl+L returns to recent calls"
                } else {
                    "Last 3 calls per reply · Ctrl+L shows full activity"
                }
                .into();
            }
            KeyCode::Char('a') => {
                app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Tree);
                request_supervisor_tree(tx, supervisor);
            }
            KeyCode::Char('d') => {
                app.todo_panel.todo_mode = Some(TodoMode::List);
                request_todo_snapshot(tx, todos);
            }
            KeyCode::Char('m') if !app.is_running() => {
                app.model_panel.model_picker = true;
                app.model_panel.model_manual = false;
                app.model_panel.model_filter = Composer::default();
                app.model_panel.selected_model = 0;
                request_models(tx, agent.clone(), false);
            }
            KeyCode::Char('n') if !app.is_running() => start_new_session(app, store, None).await?,
            KeyCode::Char('b') if !app.is_running() => {
                app.title_job = None;
                let (owner, branch) = store.branch_owned(&app.session, None).await?;
                app.session = branch;
                *store = owner;
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
    if !slash_palette_items(&app.palette_context()).is_empty() {
        match key.code {
            KeyCode::Up => {
                app.palette.selected_slash_command =
                    app.palette.selected_slash_command.saturating_sub(1);
                return Ok(());
            }
            KeyCode::Down => {
                app.palette.selected_slash_command = (app.palette.selected_slash_command + 1).min(
                    slash_palette_items(&app.palette_context())
                        .len()
                        .saturating_sub(1),
                );
                return Ok(());
            }
            KeyCode::Tab => {
                complete_selected_slash_command(app);
                return Ok(());
            }
            KeyCode::Enter
                if !key.modifiers.contains(KeyModifiers::SHIFT)
                    && !slash_input_is_complete(&app.palette_context()) =>
            {
                complete_selected_slash_command(app);
                return Ok(());
            }
            KeyCode::Esc => {
                app.palette.slash_palette_dismissed = true;
                return Ok(());
            }
            _ => {}
        }
    }
    match key.code {
        KeyCode::Esc if app.is_running() => app.cancel(),
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => app.insert_composer("\n"),
        KeyCode::Enter if app.is_running() => {
            let message = app.composer.take();
            app.prompt_history.reset_navigation();
            reset_slash_palette(app);
            if !message.trim().is_empty() {
                if !agent.supports_steering() {
                    app.composer.insert_str(&message);
                    app.status = "This compatibility provider cannot steer active runs; draft kept for the next turn".into();
                    return Ok(());
                }
                let steering = app
                    .running
                    .as_ref()
                    .expect("running state checked")
                    .steering
                    .clone();
                let queued = crate::Message::steering(message.clone());
                if message.len() > crate::agent::MAX_STEERING_BYTES {
                    app.composer.insert_str(&message);
                    app.status =
                        "Steering exceeds 64 KiB; shorten the message before sending".into();
                    return Ok(());
                }
                // Persist before making the input visible to the running agent.
                app.session.messages.push(queued.clone());
                if let Err(error) = store.save(&mut app.session).await {
                    app.session.messages.pop();
                    app.composer.insert_str(&message);
                    app.status = format!(
                        "Steering was not sent: {}",
                        compact_line(&error.to_string(), 100)
                    );
                    return Ok(());
                }
                let result = steering.try_send_message(queued);
                if result.is_err() {
                    app.session.messages.pop();
                    store.save(&mut app.session).await?;
                }
                match result {
                    Ok(()) => {
                        app.prompt_history.record(&message);
                        app.status =
                            "Steering queued for the next model boundary · Esc cancels".into();
                    }
                    Err(crate::agent::SteeringError::TooLarge(message)) => {
                        app.composer.insert_str(&message);
                        app.status =
                            "Steering exceeds 64 KiB; shorten the message before sending".into();
                    }
                    Err(crate::agent::SteeringError::Full(message)) => {
                        app.composer.insert_str(&message);
                        app.status = "Steering queue is full; message kept in composer".into();
                    }
                    Err(crate::agent::SteeringError::Closed(message)) => {
                        app.composer.insert_str(&message);
                        app.status =
                            "Run already finished; press Enter to send as a new turn".into();
                    }
                }
            }
        }
        KeyCode::Enter if !app.is_running() => {
            let prompt = app.composer.take();
            app.prompt_history.reset_navigation();
            reset_slash_palette(app);
            if !prompt.trim().is_empty() {
                app.title_job = None;
                if handle_command(&prompt, app, store, Some(agent), Some(tx)).await? {
                    return Ok(());
                }
                if let Err(error) = agent.check_current_policy() {
                    app.composer.insert_str(&prompt);
                    app.status = format!(
                        "Policy changed; restart or rebuild: {}",
                        compact_line(&error.to_string(), 120)
                    );
                    return Ok(());
                }
                let scope = match agent.prepare_run(&app.session).await {
                    Ok(scope) => scope,
                    Err(error) => {
                        app.composer.insert_str(&prompt);
                        app.status = format!("Cannot start run: {error}");
                        return Ok(());
                    }
                };
                app.prompt_history.record(&prompt);
                if let Some(scope) = &scope {
                    app.session.completion_runs.push(scope.reference());
                }
                let history = app.session.messages.clone();
                app.session
                    .messages
                    .push(crate::Message::new(Role::User, prompt.clone()));
                let run_id = scope
                    .as_ref()
                    .map(|scope| scope.run_id())
                    .unwrap_or_else(uuid::Uuid::new_v4);
                app.session.begin_run_summary(run_id);
                store.save(&mut app.session).await?;
                app.checkpoint = Some(checkpoint::State::new(&app.session, run_id));
                let agent = agent.clone();
                let events = tx.clone();
                app.live_messages.clear();
                app.working_since = Instant::now();
                app.status = "Starting…  Esc cancels".into();
                let cancel = tokio_util::sync::CancellationToken::new();
                let run_cancel = cancel.clone();
                let (steering, steering_input) = steering_channel(64);
                let checkpoint = checkpoint::UiCheckpoint {
                    run_id,
                    tx: events.clone(),
                    cancel: run_cancel.clone(),
                };
                let model = agent.model();
                let run_owner = store.clone();
                let task = tokio::spawn(async move {
                    let _run_owner = run_owner;
                    let result = agent
                        .run_checkpointed_scoped(
                            history,
                            prompt,
                            run_cancel,
                            Some(steering_input),
                            &checkpoint,
                            model,
                            scope,
                        )
                        .await;
                    let _ = events.send(UiEvent::Finished(result));
                });
                app.running = Some(Running {
                    task,
                    cancel,
                    steering,
                });
            }
        }
        KeyCode::Char(character) => {
            app.insert_composer(character.encode_utf8(&mut [0; 4]));
            reset_slash_palette(app);
            request_slash_models_if_needed(app, agent, tx);
        }
        KeyCode::Backspace => {
            app.composer.backspace();
            reset_slash_palette(app);
            request_slash_models_if_needed(app, agent, tx);
        }
        KeyCode::Delete => {
            app.composer.delete();
            reset_slash_palette(app);
            request_slash_models_if_needed(app, agent, tx);
        }
        KeyCode::Up | KeyCode::Down => {
            app.prompt_history
                .navigate(&mut app.composer, key.code == KeyCode::Up);
            reset_slash_palette(app);
        }
        KeyCode::Home => app.composer.line_start(),
        KeyCode::End => app.composer.line_end(),
        KeyCode::Left => {
            if app.composer.cursor > 0 {
                app.composer.cursor = app.composer.text[..app.composer.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index);
            }
        }
        KeyCode::Right => {
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

fn request_slash_models_if_needed(
    app: &mut App,
    agent: &Arc<Agent>,
    tx: &mpsc::UnboundedSender<UiEvent>,
) {
    let wants_models = app.composer.text.starts_with("/model ")
        || app.composer.text.starts_with("/models ")
        || app.composer.text.starts_with("/set model ");
    if !wants_models {
        app.palette.slash_models_requested = false;
    } else if app.model_panel.models.is_empty() && !app.palette.slash_models_requested {
        app.palette.slash_models_requested = true;
        request_models(tx, agent.clone(), false);
        app.status = "Discovering models…".into();
    }
}

fn reset_slash_palette(app: &mut App) {
    app.palette.selected_slash_command = 0;
    app.palette.slash_palette_dismissed = false;
}

fn complete_selected_slash_command(app: &mut App) {
    let items = slash_palette_items(&app.palette_context());
    let Some(item) = items.get(app.palette.selected_slash_command) else {
        return;
    };
    if item.completion.is_empty() {
        return;
    }
    app.composer.text.clone_from(&item.completion);
    app.composer.cursor = app.composer.text.len();
    reset_slash_palette(app);
}

fn handle_mouse(mouse: MouseEvent, app: &mut App) {
    let conversation_is_visible = app.question.is_none()
        && app.terminal_panel.attached_terminal.is_none()
        && app.approval.is_none()
        && !app.show_sessions
        && !app.terminal_panel.terminal_picker
        && !app.model_panel.model_picker
        && !app.shortcut_help
        && app.supervisor_panel.supervisor_mode.is_none()
        && app.todo_panel.todo_mode.is_none();
    if !conversation_is_visible {
        return;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => scroll_conversation(app, 3),
        MouseEventKind::ScrollDown => scroll_conversation(app, -3),
        _ => {}
    }
}
#[cfg(test)]
mod tests;
