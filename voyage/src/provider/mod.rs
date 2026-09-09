pub(crate) mod multimodal;
pub use multimodal::validate_image_capability;
mod anthropic;
mod catalog;
mod inference;
pub use catalog::{validate_model, validate_models, validate_models_for_display};
pub(crate) use inference::validate_resolution;
pub use inference::{
    inference_capabilities, inference_capabilities_with_model, inference_context,
    resolve_inference, resolve_inference_values, validate_inference_settings,
    validate_inference_settings_with_model,
};
mod chatgpt_oauth;
mod codex_subscription;
pub(crate) use codex_subscription::shutdown_owned as shutdown_compatibility;
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

#[cfg(test)]
mod failure_tests;

pub(crate) const USAGE_LIMIT_MESSAGE: &str = "Provider account usage limit reached. Wait for the account allowance to reset before sending another message.";

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("authentication failed: {0}")]
    Authentication(String),
    #[error("{USAGE_LIMIT_MESSAGE}")]
    UsageLimit,
    #[error("provider rate limit: {message}")]
    RateLimit {
        message: String,
        retry_after: Option<std::time::Duration>,
    },
    #[error("provider temporarily unavailable: {0}")]
    Unavailable(String),
    #[error("{source}")]
    RetryAfter {
        source: Box<ProviderError>,
        delay: std::time::Duration,
    },
    #[error("provider request timed out: {0}")]
    Timeout(String),
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("invalid provider response: {0}")]
    InvalidResponse(String),
    #[error("Provider stopped before completing its response.")]
    Incomplete,
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
    pub reasoning_support_known: bool,
    #[serde(default)]
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub service_tiers: Vec<String>,
    #[serde(default)]
    pub service_support_known: bool,
    #[serde(default)]
    pub default_service_tier: Option<String>,
    #[serde(default)]
    pub observed_at_ms: Option<u64>,
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
            reasoning_support_known: false,
            default_reasoning_effort: None,
            service_tiers: Vec::new(),
            service_support_known: false,
            default_service_tier: None,
            observed_at_ms: None,
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
        if let Self::RetryAfter { source, .. } = self {
            return source.is_retryable();
        }
        matches!(
            self,
            Self::RateLimit { .. } | Self::Unavailable(_) | Self::Timeout(_)
        )
    }
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::RateLimit { retry_after, .. } => *retry_after,
            Self::RetryAfter { delay, .. } => Some(*delay),
            _ => None,
        }
    }

    pub(crate) fn public_failure_reason(&self) -> &'static str {
        match self {
            Self::RetryAfter { source, .. } => source.public_failure_reason(),
            Self::Authentication(_) => {
                "Provider authentication failed. Check credentials on the executing machine."
            }
            Self::UsageLimit => USAGE_LIMIT_MESSAGE,
            Self::RateLimit { .. } => "Provider rate limit prevented completion.",
            Self::Unavailable(_) => "Provider temporarily unavailable.",
            Self::Timeout(_) => "Provider request timed out.",
            Self::Request(_) => "Provider rejected the request.",
            Self::InvalidResponse(_) => "Provider returned an invalid or incomplete response.",
            Self::Incomplete => "Provider stopped before completing its response.",
        }
    }
}

/// RFC 9110 section 10.2.3 permits either delay-seconds or an HTTP-date.
/// Overflowing valid seconds must not turn a long server wait into an early retry.
pub(crate) fn parse_retry_after(
    value: &str,
    now: std::time::SystemTime,
) -> Option<std::time::Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(std::time::Duration::from_secs(
            value.parse().unwrap_or(u64::MAX),
        ));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(now).unwrap_or_default())
}

pub(crate) fn response_retry_after(response: &reqwest::Response) -> Option<std::time::Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_retry_after(value, std::time::SystemTime::now()))
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
    validate_config_endpoints(config)?;
    validate_inference_settings(config)
        .map_err(|error| ProviderError::Request(error.to_string()))?;
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
        ProviderKind::CodexSubscription => Ok(Box::new(
            CodexSubscriptionProvider::new(config.codex_command.clone(), workspace)
                .with_sandbox(&config.sandbox)?,
        )),
    }
}

/// Validate before credentials are loaded and again at transport dispatch, since
/// public provider constructors do not require a validated Config.
pub(crate) fn validate_config_endpoints(config: &Config) -> Result<(), ProviderError> {
    let endpoint = match config.provider {
        ProviderKind::OpenaiChat | ProviderKind::OpenaiResponses | ProviderKind::Anthropic => {
            config.base_url.as_deref()
        }
        ProviderKind::ChatGptOauth => config.chatgpt_base_url.as_deref(),
        ProviderKind::CodexSubscription => None,
    };
    if let Some(endpoint) = endpoint {
        validate_native_endpoint(endpoint)?;
    }
    Ok(())
}

pub(crate) fn validate_native_endpoint(raw: &str) -> Result<(), ProviderError> {
    let denied = || {
        ProviderError::Request(
            "provider endpoint requires HTTPS or literal loopback HTTP, without userinfo or fragment"
                .into(),
        )
    };
    let url = reqwest::Url::parse(raw).map_err(|_| denied())?;
    let loopback = url.host_str().is_some_and(|host| {
        host.trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
    });
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || raw.trim() != raw
        || raw.chars().any(char::is_control)
    {
        return Err(denied());
    }
    Ok(())
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

/// Literal loopback HTTP must remain local even when HTTP_PROXY is configured.
/// Remote HTTPS keeps the caller's normal proxy configuration and connection pool.
pub(crate) fn endpoint_http_client(client: &reqwest::Client, endpoint: &str) -> reqwest::Client {
    if reqwest::Url::parse(endpoint).is_ok_and(|url| url.scheme() == "http") {
        static LOCAL: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        LOCAL
            .get_or_init(|| {
                native_http_client_builder()
                    .no_proxy()
                    .build()
                    .expect("local provider HTTP client")
            })
            .clone()
    } else {
        client.clone()
    }
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
    let retry_after = response_retry_after(&response);
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
        // Account exhaustion is not transient throttling. Do not retry the same
        // exhausted allowance, or retain arbitrary provider error text.
        let exhausted = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .is_some_and(|value| {
                [value.pointer("/error/type"), value.pointer("/error/code")]
                    .into_iter()
                    .flatten()
                    .any(|code| {
                        matches!(
                            code.as_str(),
                            Some("usage_limit_reached" | "insufficient_quota")
                        )
                    })
            });
        if exhausted {
            return Err(ProviderError::UsageLimit);
        }
        Err(ProviderError::RateLimit {
            message: body,
            retry_after,
        })
    } else if status.as_u16() == 408 || status.as_u16() == 409 || status.as_u16() >= 500 {
        let error = ProviderError::Unavailable(format!("HTTP {status}: {body}"));
        Err(match retry_after {
            Some(delay) => ProviderError::RetryAfter {
                source: Box::new(error),
                delay,
            },
            None => error,
        })
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

/// Unknown response tier IDs are metadata, never instructions. Invalid metadata
/// is discarded rather than failing/retrying an already completed inference.
pub(crate) fn reported_service_tier(value: Option<&serde_json::Value>) -> Option<String> {
    let text = value?.as_str()?;
    (!text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    .then(|| text.to_owned())
}
