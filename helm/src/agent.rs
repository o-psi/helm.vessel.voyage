use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{
    model::{Message, ModelRequest, Usage},
    provider::{Provider, ProviderError},
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
    model: String,
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
            model,
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
        mut history: Vec<Message>,
        prompt: String,
        cancel: CancellationToken,
    ) -> Result<AgentOutcome, AgentError> {
        let mut context = self.context.clone();
        context.cancellation = cancel.child_token();
        if history
            .first()
            .is_none_or(|m| m.role != crate::model::Role::System)
        {
            history.insert(
                0,
                Message::new(crate::model::Role::System, &self.system_prompt),
            );
        }
        history.push(Message::new(crate::model::Role::User, prompt));
        let mut usage = Usage::default();
        for turn in 1..=self.max_turns {
            self.sink.emit(AgentEvent::Thinking { turn }).await;
            let request = ModelRequest {
                model: self.model.clone(),
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
                        arguments: call.arguments.clone(),
                    })
                    .await;
                let result = tokio::select! {
                    _ = cancel.cancelled() => { self.sink.emit(AgentEvent::Cancelled).await; return Err(AgentError::Cancelled); }
                    value = self.tools.execute(&call.name, call.arguments, &context) => value,
                };
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
                history.push(Message::tool(call.id, content));
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
        async fn approve(&self, _: &str) -> bool {
            true
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
