mod anthropic;
mod openai;

use async_trait::async_trait;
use thiserror::Error;

use crate::{
    config::{Config, ProviderKind},
    model::{ModelRequest, ModelResponse},
};

pub use anthropic::AnthropicProvider;
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
}

pub fn from_config(config: &Config) -> Result<Box<dyn Provider>, ProviderError> {
    let key = config
        .api_key()
        .map_err(|e| ProviderError::Authentication(e.to_string()))?;
    match config.provider {
        ProviderKind::Openai => Ok(Box::new(OpenAiProvider::new(key, config.base_url.clone()))),
        ProviderKind::Anthropic => Ok(Box::new(AnthropicProvider::new(
            key,
            config.base_url.clone(),
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
