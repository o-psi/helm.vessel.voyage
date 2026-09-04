use async_trait::async_trait;
use std::sync::Arc;
use thiserror::Error;

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
    ToolStarted {
        name: String,
        arguments: serde_json::Value,
    },
    ToolFinished {
        name: String,
        result: String,
        success: bool,
    },
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
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("agent exceeded the maximum of {0} model turns")]
    MaxTurns(usize),
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
        }
    }

    pub async fn run(
        &self,
        mut history: Vec<Message>,
        prompt: String,
    ) -> Result<AgentOutcome, AgentError> {
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
            let response = self
                .provider
                .complete(ModelRequest {
                    model: self.model.clone(),
                    messages: history.clone(),
                    tools: self.tools.definitions(),
                    temperature: self.temperature,
                    max_tokens: Some(self.max_tokens),
                })
                .await?;
            usage.input_tokens += response.usage.input_tokens;
            usage.output_tokens += response.usage.output_tokens;
            let assistant = response.message;
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
                });
            }
            for call in calls {
                self.sink
                    .emit(AgentEvent::ToolStarted {
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    })
                    .await;
                let result = self
                    .tools
                    .execute(&call.name, call.arguments, &self.context)
                    .await;
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

    pub fn workspace(&self) -> &std::path::Path {
        self.context.policy.workspace()
    }
}
