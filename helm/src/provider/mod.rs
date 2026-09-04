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
    #[error("provider rate limit: {0}")]
    RateLimit(String),
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("invalid provider response: {0}")]
    InvalidResponse(String),
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
        Err(ProviderError::RateLimit(body))
    } else {
        Err(ProviderError::Request(format!("HTTP {status}: {body}")))
    }
}
