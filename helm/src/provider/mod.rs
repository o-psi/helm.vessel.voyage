mod anthropic;
mod catalog;
pub use catalog::{validate_model, validate_models, validate_models_for_display};
mod chatgpt_oauth;
mod codex_subscription;
pub(crate) mod discovery;
mod openai;
mod openai_responses;
mod redaction;
pub(crate) use redaction::definition as redact_tool_definition;
pub(crate) use redaction::message as redact_message;

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
pub use chatgpt_oauth::{
    ChatGptOauthProvider, ChatGptTokenStore, DeviceAuthorization, OAuthEndpoints, TokenStatus,
};
pub use codex_subscription::CodexSubscriptionProvider;
pub use openai::OpenAiProvider;
pub use openai_responses::OpenAiResponsesProvider;

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
    /// Internal native accounting metadata; not a public Vessel event.
    UsageReported(ReportedUsage),
    Delta(ProviderDelta),
    Completed(ModelResponse),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReportedUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
pub(crate) fn reported_usage(
    value: &serde_json::Value,
    input: &str,
    output: &str,
) -> Result<ReportedUsage, ProviderError> {
    fn field(value: &serde_json::Value, name: &str) -> Result<Option<u64>, ProviderError> {
        match value.get(name) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(value) => value.as_u64().map(Some).ok_or_else(|| {
                ProviderError::InvalidResponse("invalid provider usage counter".into())
            }),
        }
    }
    if value.is_null() {
        return Ok(ReportedUsage::default());
    }
    if !value.is_object() {
        return Err(ProviderError::InvalidResponse(
            "invalid provider usage object".into(),
        ));
    }
    Ok(ReportedUsage {
        input_tokens: field(value, input)?,
        output_tokens: field(value, output)?,
    })
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
    /// A known effective model limit, when supplied by the provider adapter.
    /// Can narrow an operator-enabled local ceiling; it does not enable a gate
    /// when the operator has left local token limits disabled.
    fn context_window(&self, _model: &str) -> Option<usize> {
        None
    }
    /// Whether new canonical user input is honored at every request boundary.
    fn supports_steering(&self) -> bool {
        true
    }

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
        ProviderKind::OpenaiResponses => Ok(Box::new(OpenAiResponsesProvider::new(
            config
                .api_key()
                .map_err(|e| ProviderError::Authentication(e.to_string()))?,
            config.base_url.clone(),
        ))),
        ProviderKind::OpenaiChat => Ok(Box::new(
            OpenAiProvider::new(
                config
                    .api_key()
                    .map_err(|e| ProviderError::Authentication(e.to_string()))?,
                config.base_url.clone(),
            )
            .with_max_tokens_parameter(config.chat_use_max_tokens),
        )),
        ProviderKind::ChatGptOauth => {
            let store = ChatGptTokenStore::new(ChatGptTokenStore::default_path()?);
            let mut endpoints = OAuthEndpoints::default();
            if let Some(base) = config.chatgpt_base_url.as_deref() {
                let base = base.trim_end_matches('/');
                endpoints.responses = format!("{base}/responses");
                endpoints.models = format!("{base}/models");
            }
            Ok(Box::new(ChatGptOauthProvider::from_store(store, endpoints)))
        }
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

/// Credentials and request bodies belong only to the explicitly configured endpoint.
/// Even same-origin redirects are rejected so no redirect can replay a POST.
pub(crate) fn native_http_client() -> reqwest::Client {
    native_http_client_builder()
        .build()
        .expect("native provider HTTP client")
}

fn native_http_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
}

pub(crate) fn reject_redirect(response: &reqwest::Response) -> Result<(), ProviderError> {
    if response.status().is_redirection() {
        return Err(ProviderError::Request(format!(
            "HTTP {} redirect refused; configure the provider's final endpoint explicitly",
            response.status().as_u16()
        )));
    }
    Ok(())
}

pub(crate) async fn checked_json(
    response: reqwest::Response,
) -> Result<serde_json::Value, ProviderError> {
    reject_redirect(&response)?;
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

trait CompatibleAuthentication {
    fn apply_key(self, key: &str) -> Self;
}
impl CompatibleAuthentication for reqwest::RequestBuilder {
    fn apply_key(self, key: &str) -> Self {
        if key.is_empty() {
            self
        } else {
            self.bearer_auth(key)
        }
    }
}
