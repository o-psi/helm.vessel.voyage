mod anthropic;
mod codex_subscription;
mod openai;

use async_trait::async_trait;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub reasoning_efforts: Vec<String>,
    #[serde(default)]
    pub input_modalities: Vec<String>,
}

impl ModelInfo {
    pub fn minimal(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            display_name: id.clone(),
            id,
            description: String::new(),
            is_default: false,
            reasoning_efforts: Vec::new(),
            input_modalities: vec!["text".into()],
        }
    }
}

pub fn normalize_models(models: &mut Vec<ModelInfo>) {
    models.retain(|model| !model.id.trim().is_empty());
    let mut unique = std::collections::BTreeMap::new();
    for model in std::mem::take(models) {
        unique
            .entry(model.id.clone())
            .and_modify(|current: &mut ModelInfo| {
                if model.is_default && !current.is_default {
                    *current = model.clone();
                }
            })
            .or_insert(model);
    }
    models.extend(unique.into_values());
    models.sort_by(|left, right| {
        right
            .is_default
            .cmp(&left.is_default)
            .then_with(|| {
                left.display_name
                    .to_ascii_lowercase()
                    .cmp(&right.display_name.to_ascii_lowercase())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
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
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Err(ProviderError::Unavailable(
            "this provider does not expose model discovery".into(),
        ))
    }
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

#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    fn catalog_is_deduplicated_and_default_first() {
        let mut models = vec![
            ModelInfo::minimal("z"),
            ModelInfo {
                display_name: "Preferred".into(),
                is_default: true,
                ..ModelInfo::minimal("z")
            },
            ModelInfo::minimal("a"),
            ModelInfo::minimal(""),
        ];
        normalize_models(&mut models);
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["z", "a"]
        );
        assert_eq!(models[0].display_name, "Preferred");
    }
}
