mod anthropic;
mod codex_subscription;
mod openai;

use async_trait::async_trait;
use futures_util::Stream;
use std::pin::Pin;
use thiserror::Error;

use crate::{
    config::{Config, ProviderKind},
    model::{ModelRequest, ModelResponse},
};

pub use anthropic::AnthropicProvider;
pub use codex_subscription::CodexSubscriptionProvider;
pub use openai::OpenAiProvider;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("authentication failed: {0}")]
    Authentication(String),
    #[error("provider rate limit: {message}")]
    RateLimit {
        message: String,
        retry_after: Option<std::time::Duration>,
    },
    #[error("provider temporarily unavailable: {0}")]
    Unavailable(String),
    #[error("provider request timed out: {0}")]
    Timeout(String),
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("invalid provider response: {0}")]
    InvalidResponse(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderDelta {
    Text(String),
    ToolCall {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: String,
    },
}

#[derive(Debug)]
pub enum ProviderStreamEvent {
    Delta(ProviderDelta),
    Completed(ModelResponse),
}

pub type ProviderStream =
    Pin<Box<dyn Stream<Item = Result<ProviderStreamEvent, ProviderError>> + Send>>;

impl ProviderError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimit { .. } | Self::Unavailable(_) | Self::Timeout(_)
        )
    }
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::RateLimit { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError>;
    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        let response = self.complete(request).await?;
        Ok(Box::pin(futures_util::stream::once(async move {
            Ok(ProviderStreamEvent::Completed(response))
        })))
    }
}

pub fn from_config(
    config: &Config,
    workspace: std::path::PathBuf,
) -> Result<Box<dyn Provider>, ProviderError> {
    match config.provider {
        ProviderKind::Openai => Ok(Box::new(OpenAiProvider::new(
            config
                .api_key()
                .map_err(|e| ProviderError::Authentication(e.to_string()))?,
            config.base_url.clone(),
        ))),
        ProviderKind::Anthropic => Ok(Box::new(AnthropicProvider::new(
            config
                .api_key()
                .map_err(|e| ProviderError::Authentication(e.to_string()))?,
            config.base_url.clone(),
        ))),
        ProviderKind::CodexSubscription => Ok(Box::new(CodexSubscriptionProvider::new(
            config.codex_command.clone(),
            workspace,
        ))),
    }
}

pub(crate) async fn checked_json(
    response: reqwest::Response,
) -> Result<serde_json::Value, ProviderError> {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(std::time::Duration::from_secs);
    let body = response
        .text()
        .await
        .map_err(|e| ProviderError::Request(e.to_string()))?;
    if status.is_success() {
        serde_json::from_str(&body)
            .map_err(|e| ProviderError::InvalidResponse(format!("{e}: {body}")))
    } else if status.as_u16() == 401 || status.as_u16() == 403 {
        Err(ProviderError::Authentication(body))
    } else if status.as_u16() == 429 {
        Err(ProviderError::RateLimit {
            message: body,
            retry_after,
        })
    } else if status.as_u16() == 408 || status.as_u16() == 409 || status.as_u16() >= 500 {
        Err(ProviderError::Unavailable(format!("HTTP {status}: {body}")))
    } else {
        Err(ProviderError::Request(format!("HTTP {status}: {body}")))
    }
}

pub(crate) async fn checked_stream_response(
    response: reqwest::Response,
) -> Result<reqwest::Response, ProviderError> {
    if response.status().is_success() {
        Ok(response)
    } else {
        match checked_json(response).await {
            Err(error) => Err(error),
            Ok(_) => unreachable!("non-success response cannot produce successful JSON"),
        }
    }
}
