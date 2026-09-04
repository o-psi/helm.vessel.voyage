use async_trait::async_trait;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{
    model::{Message, ModelRequest, ToolDefinition, Usage},
    provider::{ModelInfo, Provider, ProviderError, normalize_models},
    tools::{ToolContext, ToolRegistry},
};

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
    Cancelled,
}

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent);
}

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
    MaxTurns,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("agent exceeded the maximum of {0} model turns")]
    MaxTurns(usize),
    #[error("agent run was cancelled")]
    Cancelled,
}

pub struct Agent {
    provider: Box<dyn Provider>,
    tools: ToolRegistry,
    context: ToolContext,
    sink: Arc<dyn EventSink>,
    model: RwLock<String>,
    model_mirror: Option<Arc<RwLock<String>>>,
    model_cache: tokio::sync::Mutex<Option<(std::time::Instant, Vec<ModelInfo>)>>,
    system_prompt: String,
    max_turns: usize,
    max_tokens: u32,
    temperature: Option<f32>,
    retry: RetryPolicy,
}

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
        }
    }
}

impl Agent {
    fn effective_system_prompt(&self) -> String {
        runtime_guidance(&self.system_prompt, &self.tools.definitions())
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
        max_turns: usize,
        max_tokens: u32,
        temperature: Option<f32>,
    ) -> Self {
        Self {
            provider,
            tools,
            context,
            sink,
            model: RwLock::new(model),
            model_mirror: None,
            model_cache: tokio::sync::Mutex::new(None),
            system_prompt,
            max_turns,
            max_tokens,
            temperature,
            retry: RetryPolicy::default(),
        }
    }

    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_model_mirror(mut self, mirror: Arc<RwLock<String>>) -> Self {
        *mirror.write().expect("model mirror lock poisoned") = self.model();
        self.model_mirror = Some(mirror);
        self
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

    pub async fn models(&self, refresh: bool) -> Result<Vec<ModelInfo>, AgentError> {
        let mut cache = self.model_cache.lock().await;
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
        normalize_models(&mut models);
        *cache = Some((std::time::Instant::now(), models.clone()));
        Ok(models)
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
        mut history: Vec<Message>,
        prompt: String,
        cancel: CancellationToken,
        mut input: Option<tokio::sync::mpsc::Receiver<String>>,
    ) -> Result<AgentOutcome, AgentError> {
        let mut context = self.context.clone();
        context.cancellation = cancel.child_token();
        context.execution_id = uuid::Uuid::new_v4();
        let active_model = self.model();
        tracing::info!(execution_id = %context.execution_id, model = %active_model, "agent execution started");
        let system_prompt = self.effective_system_prompt();
        if let Some(message) = history
            .first_mut()
            .filter(|message| message.role == crate::model::Role::System)
        {
            // Refresh this on every execution. Saved sessions may predate a runtime/tool
            // upgrade and must not keep stale capability guidance forever.
            message.content = system_prompt;
        } else {
            history.insert(0, Message::new(crate::model::Role::System, system_prompt));
        }
        history.push(Message::new(crate::model::Role::User, prompt));
        let mut usage = Usage::default();
        for turn in 1..=self.max_turns {
            if let Some(receiver) = &mut input {
                while let Ok(message) = receiver.try_recv() {
                    history.push(Message::new(crate::model::Role::User, message));
                }
            }
            self.sink.emit(AgentEvent::Thinking { turn }).await;
            let request = ModelRequest {
                model: active_model.clone(),
                messages: history.clone(),
                tools: self.tools.definitions(),
                temperature: self.temperature,
                max_tokens: Some(self.max_tokens),
            };
            let response = self.stream_with_retry(request, &cancel).await?;
            usage.input_tokens += response.usage.input_tokens;
            usage.output_tokens += response.usage.output_tokens;
            let assistant = response.message;
            // AssistantText remains a completion notification for non-stream-aware sinks.
            if !assistant.content.is_empty() {
                self.sink
                    .emit(AgentEvent::AssistantText(assistant.content.clone()))
                    .await;
            }
            let calls = assistant.tool_calls.clone();
            let answer = assistant.content.clone();
            history.push(assistant);
            if calls.is_empty() {
                let mut received_supervisor_input = false;
                if let Some(receiver) = &mut input {
                    while let Ok(message) = receiver.try_recv() {
                        history.push(Message::new(crate::model::Role::User, message));
                        received_supervisor_input = true;
                    }
                }
                if received_supervisor_input {
                    continue;
                }
                return Ok(AgentOutcome {
                    messages: history,
                    answer,
                    usage,
                    turns: turn,
                    stop_reason: StopReason::Completed,
                });
            }
            for call in calls {
                self.sink
                    .emit(AgentEvent::ToolStarted {
                        name: call.name.clone(),
                        arguments: serde_json::from_str(
                            &context.redactor.redact(call.arguments.to_string()),
                        )
                        .unwrap_or_else(|_| serde_json::Value::String("[REDACTED]".into())),
                    })
                    .await;
                let result = tokio::select! {
                    _ = cancel.cancelled() => { self.sink.emit(AgentEvent::Cancelled).await; return Err(AgentError::Cancelled); }
                    value = self.tools.execute(&call.name, call.arguments, &context) => value,
                };
                tracing::info!(execution_id = %context.execution_id, tool = %call.name,
                    success = result.is_ok(), "tool execution finished");
                let (content, success) = match result {
                    Ok(value) => (value, true),
                    Err(error) => (error.to_string(), false),
                };
                self.sink
                    .emit(AgentEvent::ToolFinished {
                        name: call.name,
                        result: content.clone(),
                        success,
                    })
                    .await;
                history.push(Message::tool_result(call.id, content, success));
            }
        }
        Err(AgentError::MaxTurns(self.max_turns))
    }

    async fn stream_with_retry(
        &self,
        request: ModelRequest,
        cancel: &CancellationToken,
    ) -> Result<crate::model::ModelResponse, AgentError> {
        use crate::provider::{ProviderDelta, ProviderStreamEvent};
        use futures_util::StreamExt;
        let mut delay = self.retry.initial_delay;
        for attempt in 1..=self.retry.max_attempts.max(1) {
            let stream_result = tokio::select! {_ = cancel.cancelled()=>{self.sink.emit(AgentEvent::Cancelled).await;return Err(AgentError::Cancelled);}, value=self.provider.stream(request.clone())=>value};
            let mut stream = match stream_result {
                Ok(value) => value,
                Err(error) if error.is_retryable() && attempt < self.retry.max_attempts => {
                    self.retry_wait(attempt, &error, delay, cancel).await?;
                    delay = delay.saturating_mul(2).min(self.retry.max_delay);
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let mut partial = false;
            loop {
                let event = tokio::select! {_ = cancel.cancelled()=>{self.sink.emit(AgentEvent::Cancelled).await;return Err(AgentError::Cancelled);}, value=stream.next()=>value};
                match event {
                    Some(Ok(ProviderStreamEvent::Delta(delta))) => {
                        partial = true;
                        if let ProviderDelta::Text(text) = delta {
                            self.sink.emit(AgentEvent::AssistantTextDelta(text)).await;
                        }
                    }
                    Some(Ok(ProviderStreamEvent::Completed(response))) => return Ok(response),
                    Some(Err(error))
                        if !partial
                            && error.is_retryable()
                            && attempt < self.retry.max_attempts =>
                    {
                        self.retry_wait(attempt, &error, delay, cancel).await?;
                        delay = delay.saturating_mul(2).min(self.retry.max_delay);
                        break;
                    }
                    Some(Err(error)) => return Err(error.into()),
                    None => {
                        return Err(ProviderError::InvalidResponse(
                            "provider stream ended without completion".into(),
                        )
                        .into());
                    }
                }
            }
        }
        unreachable!("retry loop always returns")
    }

    async fn retry_wait(
        &self,
        attempt: usize,
        error: &ProviderError,
        delay: Duration,
        cancel: &CancellationToken,
    ) -> Result<(), AgentError> {
        let wait = error
            .retry_after()
            .unwrap_or(delay)
            .min(self.retry.max_delay);
        self.sink
            .emit(AgentEvent::ProviderRetry {
                attempt,
                delay: wait,
                error: error.to_string(),
            })
            .await;
        tokio::select! {_ = cancel.cancelled()=>Err(AgentError::Cancelled),_ = tokio::time::sleep(wait)=>Ok(())}
    }

    pub fn workspace(&self) -> &std::path::Path {
        self.context.policy.workspace()
    }
}

fn runtime_guidance(base: &str, tools: &[ToolDefinition]) -> String {
    let inventory = if tools.is_empty() {
        "- (none)".to_owned()
    } else {
        tools
            .iter()
            .map(|tool| format!("- `{}`: {}", tool.name, tool.description))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "{base}\n\n## Authoritative Helm runtime\n\n\
         The tool calls available in this execution are exactly the ones below. This generated \
         list overrides any provider-host, prior-session, plugin, skill, app, MCP, or built-in \
         capability guidance. Never claim access to a tool that is absent from this list. If the \
         user asks what tools are available, report these exact call names and describe them from \
         this list. Do not translate them into names from another harness.\n\n{inventory}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        model::{ModelResponse, Role},
        policy::Policy,
        tools::Approver,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Yes;
    #[async_trait]
    impl Approver for Yes {
        async fn approve(
            &self,
            _: &crate::tools::ApprovalRequest,
        ) -> crate::tools::ApprovalOutcome {
            crate::tools::ApprovalOutcome::Approved
        }
    }
    struct Flaky {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl Provider for Flaky {
        async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) < 2 {
                Err(ProviderError::RateLimit {
                    message: "slow down".into(),
                    retry_after: None,
                })
            } else {
                Ok(ModelResponse {
                    message: Message::new(Role::Assistant, "done"),
                    usage: Usage::default(),
                })
            }
        }
    }
    struct Hanging;
    #[async_trait]
    impl Provider for Hanging {
        async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
            std::future::pending().await
        }
    }
    struct EchoLatestUser;
    #[async_trait]
    impl Provider for EchoLatestUser {
        async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
            let content = request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::User)
                .map(|message| message.content.clone())
                .unwrap_or_default();
            Ok(ModelResponse {
                message: Message::new(Role::Assistant, content),
                usage: Usage::default(),
            })
        }
    }
    struct AssertCurrentGuidance;
    #[async_trait]
    impl Provider for AssertCurrentGuidance {
        async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
            let system = request.messages.first().expect("system message");
            assert_eq!(system.role, Role::System);
            assert!(system.content.starts_with("current base"));
            assert!(system.content.contains("Authoritative Helm runtime"));
            assert!(system.content.contains("- (none)"));
            assert!(!system.content.contains("obsolete guidance"));
            Ok(ModelResponse {
                message: Message::new(Role::Assistant, "done"),
                usage: Usage::default(),
            })
        }
    }
    struct PartialFailure {
        calls: Arc<AtomicUsize>,
    }
    struct StreamingOk;
    #[async_trait]
    impl Provider for StreamingOk {
        async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
            unreachable!()
        }
        async fn stream(
            &self,
            _: ModelRequest,
        ) -> Result<crate::provider::ProviderStream, ProviderError> {
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(crate::provider::ProviderStreamEvent::Delta(
                    crate::provider::ProviderDelta::Text("hel".into()),
                )),
                Ok(crate::provider::ProviderStreamEvent::Delta(
                    crate::provider::ProviderDelta::Text("lo".into()),
                )),
                Ok(crate::provider::ProviderStreamEvent::Completed(
                    ModelResponse {
                        message: Message::new(Role::Assistant, "hello"),
                        usage: Usage::default(),
                    },
                )),
            ])))
        }
    }
    #[derive(Default)]
    struct Recording {
        events: std::sync::Mutex<Vec<AgentEvent>>,
    }
    #[async_trait]
    impl EventSink for Recording {
        async fn emit(&self, event: AgentEvent) {
            self.events.lock().unwrap().push(event);
        }
    }
    #[async_trait]
    impl Provider for PartialFailure {
        async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
            unreachable!()
        }
        async fn stream(
            &self,
            _: ModelRequest,
        ) -> Result<crate::provider::ProviderStream, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(crate::provider::ProviderStreamEvent::Delta(
                    crate::provider::ProviderDelta::Text("partial".into()),
                )),
                Err(ProviderError::Unavailable("connection lost".into())),
            ])))
        }
    }

    fn agent(provider: Box<dyn Provider>, directory: &tempfile::TempDir) -> Agent {
        let policy =
            Arc::new(Policy::new(&Config::default(), directory.path().to_path_buf()).unwrap());
        Agent::new(
            provider,
            ToolRegistry::default(),
            ToolContext {
                policy,
                approver: Arc::new(Yes),
                timeout: Duration::from_secs(1),
                max_output_bytes: 4096,
                environment: Default::default(),
                cancellation: CancellationToken::new(),
                execution_id: uuid::Uuid::new_v4(),
                interaction: crate::tools::InteractionMode::Attended,
                redactor: Arc::new(crate::tools::Redactor::default()),
            },
            Arc::new(SilentSink),
            "test".into(),
            "system".into(),
            2,
            100,
            None,
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 3,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        })
    }

    #[tokio::test]
    async fn refreshes_saved_system_guidance_from_the_current_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let mut agent = agent(Box::new(AssertCurrentGuidance), &directory);
        agent.system_prompt = "current base".into();
        let history = vec![Message::new(Role::System, "obsolete guidance")];
        agent.run(history, "list tools".into()).await.unwrap();
    }

    #[test]
    fn generated_guidance_names_only_the_registered_tools() {
        let tools = vec![ToolDefinition {
            name: "real_tool".into(),
            description: "Does real work.".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }];
        let guidance = runtime_guidance("base", &tools);
        assert!(guidance.contains("`real_tool`: Does real work."));
        assert!(!guidance.contains("functions.exec"));
        assert!(!guidance.contains("collaboration.spawn_agent"));
    }

    #[tokio::test]
    async fn retries_only_transient_provider_failures() {
        let directory = tempfile::tempdir().unwrap();
        let result = agent(
            Box::new(Flaky {
                calls: AtomicUsize::new(0),
            }),
            &directory,
        )
        .run(vec![], "hello".into())
        .await
        .unwrap();
        assert_eq!(result.answer, "done");
        assert_eq!(result.stop_reason, StopReason::Completed);
    }

    #[tokio::test]
    async fn cancellation_interrupts_provider_wait() {
        let directory = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let error = agent(Box::new(Hanging), &directory)
            .run_with_cancel(vec![], "hello".into(), cancel)
            .await
            .unwrap_err();
        assert!(matches!(error, AgentError::Cancelled));
    }

    #[tokio::test]
    async fn injected_supervisor_input_changes_the_model_result() {
        let directory = tempfile::tempdir().unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        sender
            .send("new supervisor direction".into())
            .await
            .unwrap();
        let outcome = agent(Box::new(EchoLatestUser), &directory)
            .run_with_cancel_and_input(
                vec![],
                "original task".into(),
                CancellationToken::new(),
                Some(receiver),
            )
            .await
            .unwrap();
        assert_eq!(outcome.answer, "new supervisor direction");
    }

    #[tokio::test]
    async fn never_retries_after_visible_partial_output() {
        let directory = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let error = agent(
            Box::new(PartialFailure {
                calls: calls.clone(),
            }),
            &directory,
        )
        .run(vec![], "hello".into())
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            AgentError::Provider(ProviderError::Unavailable(_))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn forwards_provider_text_as_incremental_agent_events() {
        let directory = tempfile::tempdir().unwrap();
        let sink = Arc::new(Recording::default());
        let policy =
            Arc::new(Policy::new(&Config::default(), directory.path().to_path_buf()).unwrap());
        let agent = Agent::new(
            Box::new(StreamingOk),
            ToolRegistry::default(),
            ToolContext {
                policy,
                approver: Arc::new(Yes),
                timeout: Duration::from_secs(1),
                max_output_bytes: 4096,
                environment: Default::default(),
                cancellation: CancellationToken::new(),
                execution_id: uuid::Uuid::new_v4(),
                interaction: crate::tools::InteractionMode::Attended,
                redactor: Arc::new(crate::tools::Redactor::default()),
            },
            sink.clone(),
            "test".into(),
            "system".into(),
            2,
            100,
            None,
        );
        let outcome = agent.run(vec![], "hi".into()).await.unwrap();
        assert_eq!(outcome.answer, "hello");
        let deltas = sink
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                AgentEvent::AssistantTextDelta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(deltas, "hello");
    }
}
