mod provider_attempts;
mod retry;
#[cfg(test)]
mod retry_tests;

mod tool_replay;
pub use retry::RetryJitter;
#[cfg(test)]
mod completion_tests;
#[cfg(test)]
mod context_tests;
mod gate;

pub use gate::{CompletionPhase, FinalizationFailure, OwnedShutdown};

use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{
    config::AccessMode,
    model::{Message, ModelRequest, ToolDefinition, Usage},
    provider::{ModelInfo, Provider, ProviderError, normalize_models},
    tools::{ToolContext, ToolRegistry},
};

/// Each message is bounded independently of the queue count (UTF-8 bytes).
pub const MAX_STEERING_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum SteeringError {
    #[error("steering queue is full")]
    Full(String),
    #[error("run already finished")]
    Closed(String),
    #[error("steering exceeds the 64 KiB message limit")]
    TooLarge(String),
}

struct SteeringState {
    open: bool,
    capacity: usize,
    messages: VecDeque<Message>,
}

struct SteeringShared {
    state: std::sync::Mutex<SteeringState>,
    space: tokio::sync::Notify,
}

/// Cloneable input handle for adding guidance at safe boundaries in an active run.
#[derive(Clone)]
pub struct SteeringSender(Arc<SteeringShared>);

/// The run-owned side of a steering channel.
pub struct SteeringReceiver(Arc<SteeringShared>);

/// Create a bounded steering channel. Messages are applied before the next provider request.
pub fn steering_channel(capacity: usize) -> (SteeringSender, SteeringReceiver) {
    let shared = Arc::new(SteeringShared {
        state: std::sync::Mutex::new(SteeringState {
            open: true,
            capacity: capacity.max(1),
            messages: VecDeque::new(),
        }),
        space: tokio::sync::Notify::new(),
    });
    (SteeringSender(shared.clone()), SteeringReceiver(shared))
}

impl SteeringSender {
    pub fn try_send(&self, message: String) -> Result<(), SteeringError> {
        self.try_send_message(Message::steering(message))
    }

    pub fn try_send_message(&self, message: Message) -> Result<(), SteeringError> {
        if message.content.len() > MAX_STEERING_BYTES {
            return Err(SteeringError::TooLarge(message.content));
        }
        let mut state = self.0.state.lock().expect("steering channel poisoned");
        if !state.open {
            return Err(SteeringError::Closed(message.content));
        }
        if state.messages.len() >= state.capacity {
            return Err(SteeringError::Full(message.content));
        }
        state.messages.push_back(message);
        Ok(())
    }

    pub async fn send(
        &self,
        mut message: String,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<String>> {
        loop {
            let space = self.0.space.notified();
            match self.try_send(message) {
                Ok(()) => return Ok(()),
                Err(SteeringError::Closed(message) | SteeringError::TooLarge(message)) => {
                    return Err(tokio::sync::mpsc::error::SendError(message));
                }
                Err(SteeringError::Full(value)) => message = value,
            }
            space.await;
        }
    }
}

impl SteeringReceiver {
    fn drain(&mut self) -> Vec<Message> {
        let mut state = self.0.state.lock().expect("steering channel poisoned");
        let messages = state.messages.drain(..).collect();
        drop(state);
        self.0.space.notify_waiters();
        messages
    }

    /// Atomically refuse later sends if no guidance is waiting. This closes the
    /// completion race without imposing an artificial grace period.
    fn drain_or_close(&mut self) -> Vec<Message> {
        let mut state = self.0.state.lock().expect("steering channel poisoned");
        if state.messages.is_empty() {
            state.open = false;
            drop(state);
            self.0.space.notify_waiters();
            Vec::new()
        } else {
            let messages = state.messages.drain(..).collect();
            drop(state);
            self.0.space.notify_waiters();
            messages
        }
    }
}

impl Drop for SteeringReceiver {
    fn drop(&mut self) {
        self.0.state.lock().expect("steering channel poisoned").open = false;
        self.0.space.notify_waiters();
    }
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    Thinking {
        turn: usize,
    },
    AssistantText(String),
    AssistantTextDelta(String),
    ToolStarted {
        name: String,
        arguments: serde_json::Value,
    },
    ToolFinished {
        name: String,
        result: String,
        success: bool,
    },
    ProviderRetry {
        attempt: usize,
        delay: Duration,
        error: String,
    },
    ContextBudget(crate::context::ContextReport),
    InferenceWarning(crate::inference::Status),
    CompletionState {
        phase: CompletionPhase,
        readiness: Option<crate::completion::Readiness>,
        detail: Option<String>,
    },
    SteeringApplied {
        history: Vec<Message>,
    },
    Cancelled,
}

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent);
}

/// Local durable persistence boundary, not a best-effort event notification. Implementations
/// must commit before returning success and keep provider continuation on the executing machine.
#[async_trait]
pub trait RunCheckpoint: Send + Sync {
    fn run_id(&self) -> uuid::Uuid;
    async fn canonical(&self, messages: &[Message], usage: &Usage) -> Result<(), CheckpointError>;
    /// Content-free attempt state, persisted before dispatch and before retry.
    /// Nonpersistent embedders may ignore it; managed and saved sessions must commit.
    async fn provider_attempt(
        &self,
        _attempt: &voyage_protocol::provider_attempt::ProviderAttempt,
    ) -> Result<(), CheckpointError> {
        Ok(())
    }
    /// Durable request-only reductions; canonical history remains independently readable.
    async fn working_context(&self) -> Result<crate::context::WorkingContext, CheckpointError> {
        Ok(Default::default())
    }
    async fn save_working_context(
        &self,
        _context: &crate::context::WorkingContext,
    ) -> Result<(), CheckpointError> {
        Err(CheckpointError)
    }

    /// Safe provisional presentation only; never a tool admission.
    async fn tool_previews(
        &self,
        _previews: &[voyage_protocol::tool_preview::ToolPreview],
    ) -> Result<(), CheckpointError> {
        Ok(())
    }

    /// Publish and return the canonical history. Managed journals may atomically
    /// reject expired, not-yet-canonical steering; accepted history is immutable.
    async fn reconciled(
        &self,
        messages: &[Message],
        usage: &Usage,
    ) -> Result<Vec<Message>, CheckpointError> {
        self.canonical(messages, usage).await?;
        Ok(messages.to_vec())
    }

    async fn partial(&self, text: &str) -> Result<(), CheckpointError>;
    /// Called after canonical publication for assistant text that had no text
    /// deltas. It is still provisional. Canonical-only frontends need no extra
    /// record; live-event journals may durably project it without acceptance.
    async fn unstreamed(&self, _text: &str) -> Result<(), CheckpointError> {
        Ok(())
    }

    /// Called only after the durable run decision is sealed. A prior canonical
    /// checkpoint is still provisional and must not imply successful completion.
    async fn accepted(
        &self,
        _messages: &[Message],
        _usage: &Usage,
        _reason: &StopReason,
    ) -> Result<(), CheckpointError> {
        Ok(())
    }
}

#[derive(Debug, Error)]
#[error("durable checkpoint failed; execution stopped")]
pub struct CheckpointError;

pub struct SilentSink;
#[async_trait]
impl EventSink for SilentSink {
    async fn emit(&self, _: AgentEvent) {}
}

#[derive(Debug)]
pub struct AgentOutcome {
    pub messages: Vec<Message>,
    pub answer: String,
    pub usage: Usage,
    pub turns: usize,
    pub stop_reason: StopReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    Completed,
    Incomplete {
        reason: String,
        readiness: Option<crate::completion::Readiness>,
    },
}

/// Canonical run state retained when a locally rejected request stops execution.
/// Runtime-only system messages are excluded; provider continuation stays local.
#[derive(Clone)]
pub struct CanonicalRecovery {
    pub messages: Vec<Message>,
    pub usage: Usage,
}

impl std::fmt::Debug for CanonicalRecovery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalRecovery")
            .field("message_count", &self.messages.len())
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
#[error("{source}")]
pub struct ContextFailure {
    #[source]
    pub source: crate::context::ContextError,
    pub recovery: Option<Box<CanonicalRecovery>>,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("local inference admission/accounting failed: {0}")]
    Inference(String),
    #[error(transparent)]
    Finalization(Box<FinalizationFailure>),
    #[error("completion ownership failed: {0}")]
    Completion(String),
    #[error(transparent)]
    Context(#[from] ContextFailure),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("cannot start run under current system policy: {0}")]
    Policy(String),
    #[error("cannot load workspace instructions: {0}")]
    WorkspaceInstructions(String),
    #[error("agent run was cancelled")]
    Cancelled,
    #[error(transparent)]
    Checkpoint(#[from] CheckpointError),
    #[error(
        "Provider context is exhausted and no further safe reduction remains. Full history is retained; narrow the task or select a model with a larger context."
    )]
    ContextExhausted(Option<Box<CanonicalRecovery>>),
    #[error("provider usage accounting overflowed")]
    UsageOverflow,
}

impl From<crate::context::ContextError> for AgentError {
    fn from(source: crate::context::ContextError) -> Self {
        Self::Context(ContextFailure {
            source,
            recovery: None,
        })
    }
}

impl AgentError {
    pub(crate) fn is_incomplete(&self) -> bool {
        match self {
            Self::Finalization(failure) => failure.source.is_incomplete(),
            Self::Provider(error) => error.is_incomplete(),
            _ => false,
        }
    }
    /// Only locally authored text may cross the public failure surface.
    pub(crate) fn public_failure_reason(&self) -> &'static str {
        match self {
            Self::Finalization(failure) => failure.source.public_failure_reason(),
            Self::Provider(error) => error.public_failure_reason(),
            Self::Inference(_) => "Local inference admission or accounting failed.",
            Self::Completion(_) => "Completion records could not be verified.",
            Self::Context(_) => "Configured context limit prevented the request.",
            Self::Policy(_) => "Execution policy prevented the run.",
            Self::WorkspaceInstructions(_) => "Workspace instructions could not be loaded.",
            Self::Cancelled => "run cancelled",
            Self::Checkpoint(_) => "durable checkpoint failed",
            Self::UsageOverflow => "Provider usage accounting overflowed.",
            Self::ContextExhausted(_) => {
                "Provider context exhausted after safe compaction. Full history is retained; narrow the task or select a larger-context model."
            }
        }
    }

    pub fn recovery(&self) -> Option<&CanonicalRecovery> {
        match self {
            Self::Context(failure) => failure.recovery.as_deref(),
            Self::ContextExhausted(recovery) => recovery.as_deref(),
            Self::Finalization(failure) => Some(&failure.recovery),
            _ => None,
        }
    }

    fn with_recovery(mut self, messages: &[Message], usage: &Usage) -> Self {
        if let Self::ContextExhausted(recovery) = &mut self {
            *recovery = Some(Box::new(CanonicalRecovery {
                messages: messages.to_vec(),
                usage: usage.clone(),
            }));
        }
        if let Self::Context(failure) = &mut self {
            failure.recovery = Some(Box::new(CanonicalRecovery {
                messages: messages
                    .iter()
                    .filter(|message| message.role != crate::model::Role::System)
                    .cloned()
                    .collect(),
                usage: usage.clone(),
            }));
        }
        self
    }
}

pub struct Agent {
    inference: Option<crate::inference::runtime::Accounting>,
    provider: Box<dyn Provider>,
    tools: ToolRegistry,
    context: ToolContext,
    sink: Arc<dyn EventSink>,
    model: RwLock<String>,
    model_mirror: Option<Arc<RwLock<String>>>,
    model_cache: tokio::sync::Mutex<Option<(std::time::Instant, Vec<ModelInfo>)>>,
    system_prompt: String,
    max_tokens: u32,
    context_window: usize,
    completion_coordinator: Option<crate::completion::runtime::Coordinator>,
    completion_gate: Option<gate::GateResources>,
    temperature: Option<f32>,
    reasoning_effort: Option<String>,
    service_tier: Option<String>,
    inference_provider: Option<crate::ProviderKind>,
    resolved_inference:
        tokio::sync::Mutex<Option<(String, voyage_protocol::inference::InferenceResolution)>>,
    retry: RetryPolicy,
    retry_jitter: Arc<dyn RetryJitter>,
}

mod operator;

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    /// Maximum elapsed time in which another attempt may be dispatched.
    pub max_elapsed: Duration,
    pub response_timeout: Duration,
    pub stream_idle: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            max_elapsed: Duration::from_secs(120),
            response_timeout: Duration::from_secs(60),
            stream_idle: Duration::from_secs(300),
        }
    }
}

impl Agent {
    pub fn github_operator_authority(&self, write: bool) -> anyhow::Result<()> {
        self.check_current_policy()?;
        anyhow::ensure!(
            !write || self.context.policy.access_mode() != crate::config::AccessMode::ReadOnly,
            "GitHub reference edits are denied in read-only mode"
        );
        Ok(())
    }
    pub async fn github_command(
        &self,
        session: uuid::Uuid,
        words: Vec<String>,
        cancel: CancellationToken,
    ) -> anyhow::Result<crate::github::operator::CommandResult> {
        self.github_command_with_approver(session, words, cancel, self.context.approver.clone())
            .await
    }

    pub async fn github_command_with_approver(
        &self,
        session: uuid::Uuid,
        words: Vec<String>,
        cancel: CancellationToken,
        approver: Arc<dyn crate::tools::Approver>,
    ) -> anyhow::Result<crate::github::operator::CommandResult> {
        let outcome: anyhow::Result<crate::github::operator::CommandResult> = async {
        self.check_current_policy()?;
        anyhow::ensure!(!cancel.is_cancelled(), "GitHub operator action cancelled");
        let mut context = self.context.clone();
        context.completion = None;
        context.cancellation = cancel.clone();
        context.approver = approver;
        let mut result = crate::github::operator::execute(context, Some(session), words).await?;
        self.check_current_policy()?;
        anyhow::ensure!(
            !cancel.is_cancelled(),
            "GitHub operator action cancelled; inspect its receipt if publication began"
        );
        if let Some(feedback) = result.feedback.take() {
            let todos = &self
                .completion_gate
                .as_ref()
                .ok_or_else(|| {
                    anyhow::anyhow!("GitHub feedback needs the current workspace task store")
                })?
                .todos;
            let item = todos
                .import_github_feedback(feedback, self.context.policy.clone(), cancel)
                .await?;
            result.display.push_str(&format!("\nFeedback task {} is retained with source provenance; repeated imports preserve its current edits and status.", item.id.0));
        }
        Ok(result)
        }.await;
        outcome.map_err(|error| {
            let mut text = self.context.redactor.redact(error.to_string());
            if let Some(token) = self
                .context
                .environment
                .get("HELM_GITHUB_TOKEN")
                .filter(|token| !token.is_empty())
            {
                text = text.replace(token, "[REDACTED]");
            }
            anyhow::anyhow!(text)
        })
    }

    fn effective_system_prompt(&self, workspace: Option<&str>, extensions: &str) -> String {
        let mut base = self.system_prompt.clone();
        if let Some(instructions) = workspace {
            base.push_str("\n\n## Workspace instructions (AGENTS.md / agents.md)\n\nThese project instructions do not override Helm runtime authority, configured roots or approval requirements.\n\n");
            base.push_str(&self.context.redactor.redact(instructions));
            base.push_str("\n\n## End workspace instructions");
        }
        base.push_str(&self.context.redactor.redact(extensions));
        runtime_guidance(
            &base,
            &self.tools.definitions(),
            self.context.policy.access_mode(),
        )
    }

    pub fn tool_inventory(&self) -> Vec<ToolDefinition> {
        self.tools.definitions()
    }
    /// Current process manager only; persisted terminal metadata is never attachable.
    pub fn plain_terminals(
        &self,
    ) -> Result<
        (
            std::sync::Arc<dyn crate::terminal::InteractiveTerminals>,
            std::sync::Arc<crate::policy::Policy>,
        ),
        AgentError,
    > {
        self.check_current_policy()?;
        let manager: std::sync::Arc<dyn crate::terminal::InteractiveTerminals> =
            match self.tools.terminals() {
                Some(manager) => std::sync::Arc::new(manager),
                None => std::sync::Arc::new(crate::terminal::NoInteractiveTerminals::default()),
            };
        Ok((manager, self.context.policy.clone()))
    }
    pub async fn shutdown_plain_terminals(&self) -> crate::tools::TerminalShutdown {
        self.tools.shutdown_terminals(Duration::from_secs(3)).await
    }

    /// Operator presentation only; executable names and schemas stay in the registry.
    pub fn tool_inventory_display(&self) -> Vec<String> {
        self.tools.definitions().iter().map(|tool| {
            // Redact original bytes before making terminal controls visible.
            let text = self.context.redactor.redact_public_prefix(&format!("{} — {}", tool.name, tool.description));
            let mut display = String::new();
            for ch in text.chars() {
                if ch.is_control() || matches!(ch, '\u{61c}' | '\u{200e}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                    display.extend(ch.escape_default());
                } else {
                    display.push(ch);
                }
            }
            // Also guard replacement-marker collisions and values formed by
            // presentation escaping, without touching executable metadata.
            if self.context.redactor.contains_secret(&display) {
                display.clear();
            }
            display
        }).collect()
    }
    pub fn terminal_metadata(&self) -> Vec<crate::terminal::TerminalSummary> {
        self.tools
            .terminals()
            .and_then(|manager| manager.metadata().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|item| crate::terminal::TerminalSummary {
                id: crate::terminal::TerminalId(item.id),
                title: item.name.unwrap_or(item.command),
                state: if item.state == "running" {
                    crate::terminal::TerminalState::Running
                } else {
                    crate::terminal::TerminalState::Exited { code: None }
                },
            })
            .collect()
    }
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Box<dyn Provider>,
        tools: ToolRegistry,
        context: ToolContext,
        sink: Arc<dyn EventSink>,
        model: String,
        system_prompt: String,
        max_tokens: u32,
        temperature: Option<f32>,
    ) -> Self {
        Self {
            inference: None,
            provider,
            tools,
            context,
            sink,
            model: RwLock::new(model),
            model_mirror: None,
            model_cache: tokio::sync::Mutex::new(None),
            system_prompt,
            max_tokens,
            context_window: crate::context::DEFAULT_CONTEXT_WINDOW,
            completion_coordinator: None,
            completion_gate: None,
            temperature,
            reasoning_effort: None,
            service_tier: None,
            inference_provider: None,
            resolved_inference: tokio::sync::Mutex::new(None),
            retry: RetryPolicy::default(),
            retry_jitter: Arc::new(retry::RandomJitter),
        }
    }

    /// Applied at agent construction; request-boundary validation remains in the adapter.
    pub fn with_inference_provider(mut self, provider: crate::ProviderKind) -> Self {
        self.inference_provider = Some(provider);
        self
    }
    pub(crate) async fn inference_resolution(
        &self,
    ) -> Option<voyage_protocol::inference::InferenceResolution> {
        self.resolved_inference
            .try_lock()
            .ok()?
            .as_ref()
            .map(|(_, r)| r.clone())
    }

    pub fn with_inference_settings(
        mut self,
        reasoning_effort: Option<String>,
        service_tier: Option<String>,
    ) -> Self {
        self.reasoning_effort = reasoning_effort;
        self.service_tier = service_tier;
        self
    }

    pub fn with_inference_accounting(
        mut self,
        accounting: crate::inference::runtime::Accounting,
    ) -> Self {
        self.inference = Some(accounting);
        self
    }
    pub async fn inference_status(
        &self,
        session: uuid::Uuid,
    ) -> Result<Vec<crate::inference::Status>, AgentError> {
        let Some(accounting) = &self.inference else {
            return Ok(Vec::new());
        };
        accounting
            .bind(session)
            .await
            .map_err(|error| AgentError::Inference(error.to_string()))?;
        accounting
            .status(session)
            .await
            .map_err(|error| AgentError::Inference(error.to_string()))
    }
    pub async fn inference_history(
        &self,
        session: uuid::Uuid,
        project_scope: bool,
        query: crate::inference::history::Query,
        cancel: CancellationToken,
    ) -> Result<crate::inference::history::History, AgentError> {
        self.check_current_policy()?;
        let accounting = self
            .inference
            .as_ref()
            .ok_or_else(|| AgentError::Inference("Inference accounting is unavailable".into()))?;
        tokio::select! {biased; _=cancel.cancelled()=>return Err(AgentError::Cancelled), result=accounting.bind(session)=>result.map_err(|error|AgentError::Inference(error.to_string()))?};
        self.check_current_policy()?;
        let result = accounting
            .history(session, project_scope, query, cancel.clone())
            .await
            .map_err(|error| AgentError::Inference(error.to_string()))?;
        self.check_current_policy()?;
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        crate::inference::history::ensure_display_safe(&result, &self.context.redactor)
            .map_err(|error| AgentError::Inference(error.to_string()))?;
        Ok(result)
    }
    async fn inference_admit(
        &self,
        reference: Option<&crate::completion::runtime::RunReference>,
        model: &str,
        purpose: crate::inference::Purpose,
    ) -> Result<Option<crate::inference::Permit>, AgentError> {
        let Some(accounting) = &self.inference else {
            return Ok(None);
        };
        let reference = reference.ok_or_else(|| {
            AgentError::Inference("execution has no durable session/run attribution".into())
        })?;
        let permit = accounting
            .admit(reference, &self.context.redactor.redact(model), purpose)
            .await
            .map_err(|error| AgentError::Inference(error.to_string()))?;
        for warning in &permit.warnings {
            self.sink
                .emit(AgentEvent::InferenceWarning(warning.clone()))
                .await;
        }
        Ok(Some(permit))
    }
    async fn inference_report(
        &self,
        permit: Option<&crate::inference::Permit>,
        report: crate::provider::ReportedUsage,
    ) -> Result<(), AgentError> {
        if let (Some(accounting), Some(permit)) = (&self.inference, permit) {
            accounting
                .report(permit, report)
                .await
                .map_err(|error| AgentError::Inference(error.to_string()))?;
        }
        Ok(())
    }
    async fn inference_finish(
        &self,
        permit: Option<&crate::inference::Permit>,
        outcome: crate::inference::AttemptOutcome,
    ) -> Result<(), AgentError> {
        if let (Some(accounting), Some(permit)) = (&self.inference, permit) {
            accounting
                .finish(permit, outcome)
                .await
                .map_err(|error| AgentError::Inference(error.to_string()))?;
        }
        Ok(())
    }

    pub fn with_completion_gate(
        mut self,
        todos: Arc<crate::todo::TodoStore>,
        agents: crate::subagent::AgentTreeStore,
        runtime: Arc<crate::subagent::SubagentRuntime>,
    ) -> Self {
        self.completion_gate = Some(gate::GateResources {
            todos,
            agents,
            runtime,
            shutdown_timeout: Duration::from_secs(5),
        });
        self
    }

    pub fn with_completion_shutdown_timeout(mut self, shutdown: Duration) -> Self {
        if let Some(gate) = &mut self.completion_gate {
            gate.shutdown_timeout = shutdown;
        }
        self
    }

    pub fn with_completion_coordinator(
        mut self,
        coordinator: crate::completion::runtime::Coordinator,
    ) -> Self {
        self.completion_coordinator = Some(coordinator);
        self
    }

    /// Prepare and verify durable ownership before the frontend publishes work.
    /// Historical runs are checked, never adopted into the new run.
    pub async fn prepare_run(
        &self,
        session: &crate::session::Session,
    ) -> Result<Option<crate::completion::runtime::RunHandle>, AgentError> {
        self.prepare_run_with_id(session, uuid::Uuid::new_v4())
            .await
    }

    pub async fn prepare_run_with_id(
        &self,
        session: &crate::session::Session,
        run_id: uuid::Uuid,
    ) -> Result<Option<crate::completion::runtime::RunHandle>, AgentError> {
        self.check_current_policy()?;
        if let Some(accounting) = &self.inference {
            accounting
                .bind(session.id)
                .await
                .map_err(|error| AgentError::Inference(error.to_string()))?;
        }
        let Some(coordinator) = &self.completion_coordinator else {
            return Ok(None);
        };
        for reference in &session.completion_runs {
            if reference.session_id != session.id {
                return Err(AgentError::Completion(
                    "run reference belongs to a different session".into(),
                ));
            }
            crate::completion::runtime::RunHandle::resume(
                coordinator.clone(),
                session.id,
                reference.run_id,
            )
            .await
            .map_err(|error| AgentError::Completion(error.to_string()))?;
        }
        crate::completion::runtime::RunHandle::create(coordinator.clone(), session.id, run_id)
            .await
            .map(Some)
            .map_err(|error| AgentError::Completion(error.to_string()))
    }

    pub async fn run_scoped(
        &self,
        history: Vec<Message>,
        prompt: String,
        cancel: CancellationToken,
        input: Option<SteeringReceiver>,
        scope: Option<crate::completion::runtime::RunHandle>,
    ) -> Result<AgentOutcome, AgentError> {
        self.run_inner(
            history,
            Message::new(crate::model::Role::User, prompt),
            cancel,
            input,
            None,
            None,
            scope,
            None,
        )
        .await
    }

    pub fn with_context_window(mut self, limit: usize) -> Self {
        self.context_window = limit;
        self
    }

    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_retry_jitter(mut self, jitter: Arc<dyn RetryJitter>) -> Self {
        self.retry_jitter = jitter;
        self
    }

    pub fn with_model_mirror(mut self, mirror: Arc<RwLock<String>>) -> Self {
        *mirror.write().expect("model mirror lock poisoned") = self.model();
        self.model_mirror = Some(mirror);
        self
    }

    /// Redact diagnostics before projecting them into durable frontend metadata.
    pub fn redact_diagnostic(&self, text: impl AsRef<str>) -> String {
        self.context.redactor.redact(text.as_ref())
    }

    pub fn supports_steering(&self) -> bool {
        self.provider.supports_steering()
    }

    pub fn model(&self) -> String {
        self.model
            .read()
            .expect("agent model lock poisoned")
            .clone()
    }

    pub fn set_model(&self, model: impl Into<String>) -> Result<String, AgentError> {
        let model = model.into();
        if model.trim().is_empty() {
            return Err(ProviderError::InvalidResponse("model cannot be empty".into()).into());
        }
        *self.model.write().expect("agent model lock poisoned") = model.trim().to_owned();
        if let Some(mirror) = &self.model_mirror {
            *mirror.write().expect("model mirror lock poisoned") = model.trim().to_owned();
        }
        Ok(model.trim().to_owned())
    }

    /// Frontends call before recording a new turn; the run repeats this check at dispatch.
    pub fn check_current_policy(&self) -> Result<(), AgentError> {
        self.context
            .policy
            .check_current()
            .map_err(|error| AgentError::Policy(error.to_string()))
    }

    pub async fn models(&self, refresh: bool) -> Result<Vec<ModelInfo>, AgentError> {
        let mut cache = self.model_cache.lock().await;
        self.check_current_policy()?;
        if !refresh
            && let Some((created, models)) = cache.as_ref()
            && created.elapsed() < Duration::from_secs(300)
        {
            return Ok(models.clone());
        }
        let mut models = tokio::time::timeout(self.context.timeout, self.provider.models())
            .await
            .map_err(|_| ProviderError::Timeout("model discovery timed out".into()))??;
        let current = self.model();
        if !models.iter().any(|model| model.id == current) {
            models.push(ModelInfo::minimal(current));
        }
        crate::provider::validate_models_for_display(&models, |value| {
            self.context.redactor.contains_secret(value)
        })?;
        normalize_models(&mut models);
        *cache = Some((std::time::Instant::now(), models.clone()));
        Ok(models)
    }

    /// Best-effort utility request using this agent's existing provider and authority.
    pub async fn generate_title(
        &self,
        messages: &[Message],
        cancel: CancellationToken,
    ) -> Option<crate::titles::TitleResult> {
        self.generate_title_inner(messages, cancel, None).await
    }
    pub async fn generate_title_for_session(
        &self,
        session: &crate::session::Session,
        cancel: CancellationToken,
    ) -> Option<crate::titles::TitleResult> {
        self.generate_title_inner(&session.messages, cancel, session.completion_runs.last())
            .await
    }
    async fn generate_title_inner(
        &self,
        messages: &[Message],
        cancel: CancellationToken,
        reference: Option<&crate::completion::runtime::RunReference>,
    ) -> Option<crate::titles::TitleResult> {
        use futures_util::StreamExt;
        let work = async {
            let model = crate::titles::title_model()?;
            if !self
                .models(false)
                .await
                .ok()?
                .iter()
                .any(|item| item.id == model)
            {
                return None;
            }
            let mut request = crate::titles::request(messages, model, &self.context.redactor)?;
            request.max_tokens = (self.max_tokens > 0).then_some(self.max_tokens);
            let limit = self.context_limit(&request.model);
            if limit > 0 {
                crate::context::preflight(&mut request, limit).ok()?;
            }
            self.check_current_policy().ok()?;
            let permit = self
                .inference_admit(reference, &request.model, crate::inference::Purpose::Title)
                .await
                .ok()?;
            self.check_current_policy().ok()?;
            if cancel.is_cancelled() {
                return None;
            }
            let mut stream = match self.provider.stream(request).await {
                Ok(stream) => stream,
                Err(_) => {
                    self.inference_finish(
                        permit.as_ref(),
                        crate::inference::AttemptOutcome::Failed,
                    )
                    .await
                    .ok()?;
                    return None;
                }
            };
            let mut text_bytes = 0_usize;
            while let Some(event) = stream.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(_) => {
                        self.inference_finish(
                            permit.as_ref(),
                            crate::inference::AttemptOutcome::Failed,
                        )
                        .await
                        .ok()?;
                        return None;
                    }
                };
                match event {
                    crate::provider::ProviderStreamEvent::ResponseMetadata { .. } => {}
                    crate::provider::ProviderStreamEvent::UsageReported(report) => {
                        self.inference_report(permit.as_ref(), report).await.ok()?
                    }
                    crate::provider::ProviderStreamEvent::Completed(response) => {
                        self.inference_finish(
                            permit.as_ref(),
                            crate::inference::AttemptOutcome::Completed,
                        )
                        .await
                        .ok()?;
                        return Some(crate::titles::TitleResult {
                            title: crate::titles::sanitize(
                                &response.message,
                                &self.context.redactor,
                            ),
                            usage: response.usage,
                        });
                    }
                    crate::provider::ProviderStreamEvent::Delta(
                        crate::provider::ProviderDelta::Text(text),
                    ) => {
                        text_bytes = text_bytes.checked_add(text.len())?;
                        if text_bytes > 4096 {
                            return None;
                        }
                    }
                    crate::provider::ProviderStreamEvent::Delta(
                        crate::provider::ProviderDelta::ToolCall { .. },
                    ) => return None,
                }
            }
            None
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            _ = self.context.cancellation.cancelled() => None,
            result = tokio::time::timeout(self.context.timeout.min(Duration::from_secs(10)), work) => {
                result.ok().flatten()
            }
        }
    }

    pub async fn run(
        &self,
        history: Vec<Message>,
        prompt: String,
    ) -> Result<AgentOutcome, AgentError> {
        self.run_with_cancel(history, prompt, CancellationToken::new())
            .await
    }

    pub async fn run_with_cancel(
        &self,
        history: Vec<Message>,
        prompt: String,
        cancel: CancellationToken,
    ) -> Result<AgentOutcome, AgentError> {
        self.run_with_cancel_and_input(history, prompt, cancel, None)
            .await
    }

    pub async fn run_with_cancel_and_input(
        &self,
        history: Vec<Message>,
        prompt: String,
        cancel: CancellationToken,
        input: Option<SteeringReceiver>,
    ) -> Result<AgentOutcome, AgentError> {
        self.run_inner(
            history,
            Message::new(crate::model::Role::User, prompt),
            cancel,
            input,
            None,
            None,
            None,
            None,
        )
        .await
    }

    pub async fn run_checkpointed(
        &self,
        history: Vec<Message>,
        prompt: String,
        cancel: CancellationToken,
        input: Option<SteeringReceiver>,
        checkpoint: &dyn RunCheckpoint,
        model: String,
    ) -> Result<AgentOutcome, AgentError> {
        self.run_inner(
            history,
            Message::new(crate::model::Role::User, prompt),
            cancel,
            input,
            Some(checkpoint),
            Some(model),
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_checkpointed_scoped(
        &self,
        history: Vec<Message>,
        prompt: String,
        cancel: CancellationToken,
        input: Option<SteeringReceiver>,
        checkpoint: &dyn RunCheckpoint,
        model: String,
        scope: Option<crate::completion::runtime::RunHandle>,
    ) -> Result<AgentOutcome, AgentError> {
        self.run_checkpointed_scoped_with_workflow_secrets(
            history,
            Message::new(crate::model::Role::User, prompt),
            cancel,
            input,
            checkpoint,
            model,
            scope,
            None,
        )
        .await
    }

    /// Transient bindings belong to this accepted checkpoint run only. They are
    /// deliberately absent from the shared ToolContext and child-agent runtime.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_checkpointed_scoped_with_workflow_secrets(
        &self,
        history: Vec<Message>,
        prompt: Message,
        cancel: CancellationToken,
        input: Option<SteeringReceiver>,
        checkpoint: &dyn RunCheckpoint,
        model: String,
        scope: Option<crate::completion::runtime::RunHandle>,
        bindings: Option<crate::workflow::secrets::RunBindings>,
    ) -> Result<AgentOutcome, AgentError> {
        self.run_inner(
            history,
            prompt,
            cancel,
            input,
            Some(checkpoint),
            Some(model),
            scope,
            bindings,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_inner(
        &self,
        mut history: Vec<Message>,
        prompt: Message,
        cancel: CancellationToken,
        mut input: Option<SteeringReceiver>,
        checkpoint: Option<&dyn RunCheckpoint>,
        selected_model: Option<String>,
        scope: Option<crate::completion::runtime::RunHandle>,
        bindings: Option<crate::workflow::secrets::RunBindings>,
    ) -> Result<AgentOutcome, AgentError> {
        self.check_current_policy()?;
        // Reset inherited defaults at each turn when an agent is reused.
        *self.resolved_inference.lock().await = None;
        let root_scope = scope.clone();
        if let (Some(scope), Some(checkpoint)) = (&root_scope, checkpoint)
            && scope.run_id() != checkpoint.run_id()
        {
            return Err(AgentError::Completion(
                "checkpoint run does not match completion scope".into(),
            ));
        }
        let root_gate = if root_scope.is_some() {
            Some(self.completion_gate.as_ref().ok_or_else(|| {
                AgentError::Completion("root completion stores are not configured".into())
            })?)
        } else {
            None
        };
        let mut context = self.context.clone();
        context.cancellation = cancel.child_token();
        if let Some(scope) = scope {
            if !self
                .completion_coordinator
                .as_ref()
                .is_some_and(|c| c.same(scope.coordinator()))
            {
                return Err(AgentError::Completion(
                    "run coordinator does not match this agent".into(),
                ));
            }
            context.execution_id = scope.run_id();
            context.completion = Some(scope);
        } else {
            context.execution_id =
                checkpoint.map_or_else(uuid::Uuid::new_v4, RunCheckpoint::run_id);
        }
        if self.completion_coordinator.is_some() && context.completion.is_none() {
            return Err(AgentError::Completion(
                "prepare a session run before execution".into(),
            ));
        }
        if context.execution_id.is_nil() {
            return Err(CheckpointError.into());
        }
        if bindings
            .as_ref()
            .is_some_and(|bindings| !bindings.matches_run(context.execution_id))
        {
            return Err(AgentError::Completion(
                "workflow secret bindings do not match this run".into(),
            ));
        }
        history.retain(|message| message.role != crate::model::Role::System);
        history.push(prompt);
        let mut usage = Usage::default();
        let mut turn = 0usize;
        let mut sealed = false;
        let mut last_readiness = None;
        let mut completion_continuation = gate::CompletionContinuation::default();
        let mut partial_output = String::new();
        let result = async {
        if root_gate.is_some() {
            self.sink.emit(AgentEvent::CompletionState { phase: CompletionPhase::Provisional, readiness: None, detail: Some("Streamed output is provisional until the run is accepted".into()) }).await;
        }
        let active_model = selected_model.unwrap_or_else(|| self.model());
        tracing::info!(execution_id = %context.execution_id, model = %active_model, "agent execution started");
        if cancel.is_cancelled() {
            self.sink.emit(AgentEvent::Cancelled).await;
            return Err(AgentError::Cancelled);
        }
        let policy = context.policy.clone();
        let workspace = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                self.sink.emit(AgentEvent::Cancelled).await;
                return Err(AgentError::Cancelled);
            }
            result = tokio::task::spawn_blocking(move || crate::workspace_instructions::load(&policy)) => {
                result.map_err(|error| AgentError::WorkspaceInstructions(error.to_string()))?
                    .map_err(|error| AgentError::WorkspaceInstructions(context.redactor.redact(format!("{error:#}"))))?
            }
        };
        let extension_policy = context.policy.clone();
        let extension_guidance = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                self.sink.emit(AgentEvent::Cancelled).await;
                return Err(AgentError::Cancelled);
            }
            result = tokio::task::spawn_blocking(move || crate::extensions::guidance(extension_policy.workspace())) => result.unwrap_or_default(),
        };
        // Initial input is canonical before admitting optional package hooks.
        gate::guarded(self.checkpoint(checkpoint, &mut history, &usage), &cancel).await??;
        self.tools.run_extension_lifecycle("run_start", &context).await;
        let mut working_context = if let Some(checkpoint) = checkpoint {
            gate::guarded(tokio::time::timeout(context.timeout, checkpoint.working_context()), &cancel).await?
                .map_err(|_| CheckpointError)??
        } else { crate::context::WorkingContext::default() };
        'execution: loop {
            // Diagnostic accounting only; progress never imposes an execution cutoff.
            turn = turn.saturating_add(1);
            let steering_start = history.len();
            if let Some(receiver) = &mut input {
                for mut message in receiver.drain() {
                    if !self.supports_steering() {
                        return Err(ProviderError::Request("active steering is unavailable with this compatibility provider; send a new turn after completion".into()).into());
                    }
                    if let Some(receipt) = &mut message.steering {
                        receipt.status = crate::model::SteeringStatus::Applied;
                    }
                    history.push(message);
                }
            }
            gate::guarded(self.checkpoint(checkpoint, &mut history, &usage), &cancel).await??;
            if history.len() > steering_start {
                self.sink
                    .emit(AgentEvent::SteeringApplied {
                        history: history.clone(),
                    })
                    .await;
            }
            if cancel.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            self.sink.emit(AgentEvent::Thinking { turn }).await;
            let prepared = working_context.prepare(&history).map_err(|_| CheckpointError)?;
            if prepared > 0 && let Some(checkpoint) = checkpoint {
                gate::guarded(tokio::time::timeout(context.timeout, checkpoint.save_working_context(&working_context)), &cancel).await?
                    .map_err(|_| CheckpointError)??;
            }
            let mut recovery_attempt = 0;
            let mut rejected_size = None;
            let (response, permit) = loop {
                let mut messages = working_context.project(&history).map_err(|_| CheckpointError)?;
                completion_continuation.project(&mut messages);
                crate::model::visual::project(&mut messages, context.artifact_scope.as_ref())?;
                let mut instructions = self.effective_system_prompt(workspace.as_deref(), &extension_guidance);
                completion_continuation.append_to(&mut instructions);
                messages.insert(0, Message::new(crate::model::Role::System, instructions));
                let request = ModelRequest {
                    model: active_model.clone(), messages, tools: self.tools.definitions(),
                    temperature: self.temperature, reasoning_effort: self.reasoning_effort.clone(),
                    service_tier: self.service_tier.clone(), max_tokens: (self.max_tokens > 0).then_some(self.max_tokens),
                };
                let result = self.stream_with_retry(request, &cancel, checkpoint, &mut partial_output, context.completion.as_ref().map(|run| run.reference()).as_ref(), &mut rejected_size).await;
                match result {
                    Ok(response) => break response,
                    Err(AgentError::Provider(error)) if error.is_context_length() => {
                        // Stay inside this provider boundary: tools already checkpointed above
                        // are never replayed, and each changed request receives new admission.
                        let mut changed = 0;
                        while recovery_attempt < 4 && changed == 0 {
                            changed = working_context.recover(&history, recovery_attempt).map_err(|_| CheckpointError)?;
                            recovery_attempt += 1;
                        }
                        if changed == 0 { return Err(AgentError::ContextExhausted(None).with_recovery(&history, &usage)); }
                        if let Some(checkpoint) = checkpoint {
                            gate::guarded(tokio::time::timeout(context.timeout, checkpoint.save_working_context(&working_context)), &cancel).await?
                                .map_err(|_| CheckpointError)??;
                        }
                        tracing::info!(recovery_attempt, compacted_messages=changed, "provider context rejection: durable working context reduced");
                    }
                    Err(error) => return Err(error.with_recovery(&history, &usage)),
                }
            };
            let had_streamed_text = !partial_output.is_empty();
            partial_output.clear();
            usage.input_tokens = usage
                .input_tokens
                .checked_add(response.usage.input_tokens)
                .ok_or(AgentError::UsageOverflow)?;
            usage.output_tokens = usage
                .output_tokens
                .checked_add(response.usage.output_tokens)
                .ok_or(AgentError::UsageOverflow)?;
            self.inference_finish(permit.as_ref(), crate::inference::AttemptOutcome::Completed).await?;
            let mut assistant = response.message;
            if let Err(error) = tool_replay::normalize(&mut assistant.tool_calls) {
                // Account for the completed provider response without accepting
                // ambiguous calls into canonical history or dispatching effects.
                gate::guarded(self.checkpoint(checkpoint, &mut history, &usage), &cancel).await??;
                return Err(error.into());
            }
            let calls = assistant.tool_calls.clone();
            let answer = assistant.content.clone();
            history.push(assistant);
            gate::guarded(self.checkpoint(checkpoint, &mut history, &usage), &cancel).await??;
            if cancel.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            // A completed-only response is still provisional text. Durable
            // journals may need its text independently of the local event sink.
            // Do not duplicate content already delivered as streaming deltas.
            if !had_streamed_text && !answer.is_empty() && let Some(checkpoint) = checkpoint {
                gate::guarded(async {
                    tokio::time::timeout(self.context.timeout, checkpoint.unstreamed(&answer))
                        .await.map_err(|_| CheckpointError)?
                }, &cancel).await??;
            }
            if !answer.is_empty() {
                self.sink
                    .emit(AgentEvent::AssistantText(answer.clone()))
                    .await;
            }
            if calls.is_empty() {
                loop {
                    let steering_start = history.len();
                    if let Some(receiver) = &mut input {
                        let pending = if root_gate.is_some() { receiver.drain() } else { receiver.drain_or_close() };
                        if pending.is_empty() { break; }
                        for mut message in pending {
                            if !self.supports_steering() {
                                return Err(ProviderError::Request("active steering is unavailable with this compatibility provider; send a new turn after completion".into()).into());
                            }
                            if let Some(receipt) = &mut message.steering {
                                receipt.status = crate::model::SteeringStatus::Applied;
                            }
                            history.push(message);
                        }
                    } else { break; }
                    if history.len() > steering_start {
                        gate::guarded(self.checkpoint(checkpoint, &mut history, &usage), &cancel).await??;
                    }
                    if history.len() > steering_start {
                        self.sink
                            .emit(AgentEvent::SteeringApplied {
                                history: history.clone(),
                            })
                            .await;
                        continue 'execution;
                    }
                    // Expiry alone must not trigger inference; drain again to close input.
                }
                if let (Some(gate), Some(scope)) = (root_gate, &root_scope) {
                    loop {
                        let mut lease = gate::guarded(gate.lease(scope), &cancel).await??;
                        last_readiness = Some(lease.readiness.clone());
                        let clean = lease.readiness.ready() && lease.readiness.incomplete == 0;
                        if !clean && completion_continuation.offer(&lease.readiness, history.len() - 1) {
                            // Release the writer lease before inference or tools so the model
                            // and active descendants can finish the work just observed.
                            let readiness = lease.readiness.clone();
                            drop(lease);
                            tracing::info!(execution_id = %context.execution_id, total = readiness.total,
                                incomplete = readiness.incomplete, accounted = readiness.accounted,
                                "runtime completion continuation requested");
                            self.sink.emit(AgentEvent::CompletionState {
                                phase: CompletionPhase::Reconciling,
                                readiness: Some(readiness),
                                detail: Some("Voyage runtime asked the model to finish remaining work; no user message was added".into()),
                            }).await;
                            continue 'execution;
                        }
                        if !clean {
                            drop(lease);
                            gate::guarded(gate.shutdown_owned(scope), &cancel).await?;
                            lease = gate::guarded(gate.lease(scope), &cancel).await??;
                            last_readiness = Some(lease.readiness.clone());
                        }
                        // After the optional continuation, finalization is local; no model sign-off.
                        // Close steering atomically while record writers are excluded.
                        if let Some(receiver) = &mut input {
                            let pending = receiver.drain_or_close();
                            if !pending.is_empty() {
                                let steering_start = history.len();
                                for mut message in pending {
                                    if let Some(receipt) = &mut message.steering { receipt.status = crate::model::SteeringStatus::Applied; }
                                    history.push(message);
                                }
                                drop(lease);
                                gate::guarded(self.checkpoint(checkpoint, &mut history, &usage), &cancel).await??;
                                if history.len() > steering_start {
                                    self.sink.emit(AgentEvent::SteeringApplied { history: history.clone() }).await;
                                    continue 'execution;
                                }
                                // Expiry alone does not justify another provider call.
                                // Reacquire the completion lease and close input atomically.
                                continue;
                            }
                        }
                        self.tools.run_extension_lifecycle("run_finish", &context).await;
                        if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
                        gate::guarded(self.checkpoint(checkpoint, &mut history, &usage), &cancel).await??;
                        let readiness = lease.readiness.clone();
                        let clean = readiness.ready() && readiness.incomplete == 0;
                        let reason = (!clean).then(|| gate::incomplete_reason(&readiness));
                        let final_outcome = if clean { crate::completion::FinalOutcome::Completed } else { crate::completion::FinalOutcome::Incomplete };
                        gate::guarded(lease.seal(final_outcome, reason.clone()), &cancel).await?
                            .map_err(|error| AgentError::Completion(error.to_string()))?;
                        sealed = true;
                        if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
                        let stop_reason = if clean { StopReason::Completed } else { StopReason::Incomplete { reason: reason.clone().unwrap(), readiness: Some(readiness.clone()) } };
                        if let Some(checkpoint) = checkpoint {
                            tokio::time::timeout(gate.shutdown_timeout, checkpoint.accepted(&history, &usage, &stop_reason)).await.map_err(|_| CheckpointError)??;
                        }
                        if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
                        self.sink.emit(AgentEvent::CompletionState { phase: if clean { CompletionPhase::Completed } else { CompletionPhase::Incomplete }, readiness: Some(readiness), detail: reason }).await;
                        return Ok(AgentOutcome { messages: history.clone(), answer, usage: usage.clone(), turns: turn, stop_reason });
                    }
                }
                self.tools.run_extension_lifecycle("run_finish", &context).await;
                if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
                if let Some(checkpoint) = checkpoint {
                    tokio::time::timeout(context.timeout, checkpoint.accepted(&history, &usage, &StopReason::Completed)).await.map_err(|_| CheckpointError)??;
                }
                return Ok(AgentOutcome {
                    messages: history.clone(), answer, usage: usage.clone(), turns: turn, stop_reason: StopReason::Completed,
                });
            }
            for call in calls {
                context.tool_call_id = Some(call.id.clone());
                self.sink
                    .emit(AgentEvent::ToolStarted {
                        name: call.name.clone(),
                        arguments: serde_json::from_str(
                            &context.redactor.redact(call.arguments.to_string()),
                        )
                        .unwrap_or_else(|_| serde_json::Value::String("[REDACTED]".into())),
                    })
                    .await;
                let tool_started = std::time::Instant::now();
                let result = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => { self.sink.emit(AgentEvent::Cancelled).await; return Err(AgentError::Cancelled); }
                    value = gate::guarded(self.tools.execute_report_with_workflow_secrets(&call.name, call.arguments, &context, bindings.as_ref()), &cancel) => value?,
                };
                let mut report = match result {
                    Ok(report) => report,
                    Err(error) => crate::tools::ToolReport::error(error),
                };
                report.outcome.elapsed_ms =
                    Some(tool_started.elapsed().as_millis().min(u64::MAX as u128) as u64);
                report.synchronize();
                let success = report.outcome.success();
                tracing::info!(execution_id = %context.execution_id, tool = %call.name,
                    success, outcome = ?report.outcome, "tool execution finished");
                let content = report.output.text_fallback();
                let mut message = Message::tool_result(&call.id, &content, success);
                message.tool_output = Some(Box::new(report.output));
                message.tool_outcome = Some(report.outcome);
                history.push(message);
                gate::guarded(self.checkpoint(checkpoint, &mut history, &usage), &cancel).await??;
                self.sink
                    .emit(AgentEvent::ToolFinished {
                        name: call.name,
                        result: content.clone(),
                        success,
                    })
                    .await;
            }
        }
        }.await;
        match result {
            Ok(outcome) => Ok(outcome),
            Err(source) => {
                if let (Some(gate), Some(scope)) = (root_gate, &root_scope) {
                    let mut shutdown = OwnedShutdown::default();
                    let cleanup = async {
                        shutdown = gate.shutdown_owned(scope).await;
                        if !sealed && let Ok(lease) = gate.lease(scope).await {
                            last_readiness = Some(lease.readiness.clone());
                            let _ = lease
                                .seal(
                                    if source.is_incomplete() {
                                        crate::completion::FinalOutcome::Incomplete
                                    } else {
                                        crate::completion::FinalOutcome::Interrupted
                                    },
                                    Some(
                                        if source.is_incomplete() {
                                            "provider output incomplete"
                                        } else {
                                            "root run interrupted before acceptance"
                                        }
                                        .into(),
                                    ),
                                )
                                .await;
                        }
                    };
                    let _ = tokio::time::timeout(gate.shutdown_timeout, cleanup).await;
                    // Durable checkpoints retain unfinished text separately; do
                    // not duplicate it as an accepted canonical assistant message.
                    if checkpoint.is_none() && !partial_output.is_empty() {
                        history.push(Message::new(crate::model::Role::Assistant, partial_output));
                    }
                    self.sink.emit(AgentEvent::CompletionState { phase: if source.is_incomplete() { CompletionPhase::Incomplete } else { CompletionPhase::Interrupted }, readiness: last_readiness.clone(), detail: Some(format!("Run not completed; owned child shutdown observed: {}; remaining IDs: {:?}", shutdown.observation_complete, shutdown.remaining)) }).await;
                    Err(AgentError::Finalization(Box::new(FinalizationFailure {
                        source: Box::new(source),
                        recovery: CanonicalRecovery {
                            messages: history,
                            usage,
                        },
                        readiness: last_readiness,
                        shutdown,
                    })))
                } else {
                    Err(source)
                }
            }
        }
    }

    async fn checkpoint(
        &self,
        checkpoint: Option<&dyn RunCheckpoint>,
        messages: &mut Vec<Message>,
        usage: &Usage,
    ) -> Result<(), AgentError> {
        if let Some(checkpoint) = checkpoint {
            *messages =
                tokio::time::timeout(self.context.timeout, checkpoint.reconciled(messages, usage))
                    .await
                    .map_err(|_| CheckpointError)??;
        }
        Ok(())
    }

    fn context_limit(&self, model: &str) -> usize {
        if self.context_window == 0 {
            return 0;
        }
        self.provider
            .context_window(model)
            .map_or(self.context_window, |limit| limit.min(self.context_window))
    }

    async fn project_provider_text(
        &self,
        text: String,
        checkpoint: Option<&dyn RunCheckpoint>,
        partial_output: &mut String,
    ) -> Result<(), AgentError> {
        if text.is_empty() {
            return Ok(());
        }
        partial_output.push_str(&text);
        if let Some(checkpoint) = checkpoint {
            tokio::time::timeout(self.context.timeout, checkpoint.partial(&text))
                .await
                .map_err(|_| CheckpointError)??;
        }
        self.sink.emit(AgentEvent::AssistantTextDelta(text)).await;
        Ok(())
    }

    async fn stream_with_retry(
        &self,
        mut request: ModelRequest,
        cancel: &CancellationToken,
        checkpoint: Option<&dyn RunCheckpoint>,
        partial_output: &mut String,
        reference: Option<&crate::completion::runtime::RunReference>,
        previous_request_size: &mut Option<usize>,
    ) -> Result<
        (
            crate::model::ModelResponse,
            Option<crate::inference::Permit>,
        ),
        AgentError,
    > {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        if let Some(provider) = &self.inference_provider {
            let mut frozen = self.resolved_inference.lock().await;
            if frozen
                .as_ref()
                .is_none_or(|(model, _)| model != &request.model)
            {
                // Resolve once for this admitted executor/model, then freeze the
                // wire settings across retries and tool continuations. A later
                // catalog refresh or next-turn setting cannot alter this turn.
                tokio::select! {
                    _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                    _ = tokio::time::timeout(Duration::from_secs(20), self.models(false)) => {}
                }
                let cache = self.model_cache.lock().await;
                let known = cache
                    .as_ref()
                    .and_then(|(_, models)| models.iter().find(|m| m.id == request.model));
                let resolution = crate::provider::resolve_inference_values(
                    provider,
                    &request.model,
                    request.reasoning_effort.as_deref(),
                    request.service_tier.as_deref(),
                    known,
                );
                crate::provider::validate_resolution(provider, &resolution)?;
                *frozen = Some((request.model.clone(), resolution));
            }
            if let Some((_, resolution)) = frozen.as_ref() {
                request.reasoning_effort = resolution.thinking.wire_value.clone();
                request.service_tier = resolution.service.wire_value.clone();
            }
        }
        // Project outgoing copies; previously stored canonical history is not
        // rewritten when the operator configures a new secret.
        for message in &mut request.messages {
            crate::provider::redact_message(message, &self.context.redactor)?;
        }
        for definition in &mut request.tools {
            crate::provider::redact_tool_definition(definition, &self.context.redactor)?;
        }
        tool_replay::project_interrupted_calls(&mut request.messages);
        let limit = self.context_limit(&request.model);
        if limit > 0 {
            let report = crate::context::preflight(&mut request, limit)?;
            tracing::info!(
                estimated_tokens = report.estimated,
                context_window = report.limit,
                omitted_messages = report.omitted_messages,
                "explicit request context preflight"
            );
            self.sink.emit(AgentEvent::ContextBudget(report)).await;
        }
        // Compare the actual post-redaction, tool-replay and explicit-limit projection.
        // A raw-history decrease is insufficient if preflight had already omitted it.
        let request_size = crate::context::estimate(&request);
        if previous_request_size
            .is_some_and(|previous| request_size.saturating_add(128) >= previous)
        {
            return Err(AgentError::ContextExhausted(None));
        }
        *previous_request_size = Some(request_size);
        self.provider_request_attempts(request, cancel, checkpoint, partial_output, reference)
            .await
    }

    pub fn workspace(&self) -> &std::path::Path {
        self.context.policy.workspace()
    }
}

fn runtime_guidance(base: &str, tools: &[ToolDefinition], access: AccessMode) -> String {
    let inventory = if tools.is_empty() {
        "- (none)".to_owned()
    } else {
        tools
            .iter()
            .map(|tool| format!("- `{}`: {}", tool.name, tool.description))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let authority = match access {
        AccessMode::ReadOnly => {
            "This execution is read-only. Use inspection tools only; do not attempt shell commands, file writes, terminal control, todo mutations, worktree mutations, or MCP calls."
        }
        AccessMode::Approval => {
            "This execution uses approval mode. Ordinary inspection may proceed directly; consequential commands, file writes, and MCP calls may pause for operator approval."
        }
        AccessMode::Unrestricted => {
            "This execution is unrestricted within the runtime's configured roots and resource limits. Tool actions do not require interactive approval."
        }
    };
    // Bundled guidance is ephemeral and follows the actual registry (including
    // delegated tool restrictions); it is never saved as a conversation message.
    let coordination = if tools.iter().any(|tool| tool.name == "vessel") {
        concat!(
            "\n\n## Bundled skill: ",
            include_str!("../skills/vessel-coordination.md")
        )
    } else {
        ""
    };
    format!(
        "{base}{coordination}\n\n## Authoritative Helm runtime\n\n\
         Access mode: `{access}`. {authority}\n\n\
         The tool calls available in this execution are exactly the ones below. This generated \
         list and runtime policy override any workspace instructions, provider-host, prior-session, plugin, skill, app, MCP, or built-in \
         capability guidance. Never claim access to a tool that is absent from this list. If the \
         user asks what tools are available, report these exact call names and describe them from \
         this list. A capability is not a callable tool unless its exact name appears below. In \
         particular, do not invent orchestration wrappers such as `multi_tool_use.parallel`, web \
         search, browser, or provider-host functions. Before answering a tool-inventory question, \
         check every name in the answer against this list and omit any unmatched name. Do not \
         translate tools into names from another harness. The local `/tools` command is the \
         operator's authoritative inventory.\n\n{inventory}"
    )
}

mod tool_preview;
