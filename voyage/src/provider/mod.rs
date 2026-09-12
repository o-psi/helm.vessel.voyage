pub(crate) mod multimodal;
pub use multimodal::validate_image_capability;
mod anthropic;
mod api_credential;
mod catalog;
mod inference;
pub use catalog::{validate_model, validate_models, validate_models_for_display};
pub(crate) use inference::validate_resolution;
pub use inference::{
    inference_capabilities, inference_capabilities_with_model, inference_context,
    resolve_inference, resolve_inference_values, validate_inference_settings,
    validate_inference_settings_with_model,
};
pub(crate) mod chatgpt_oauth;
pub(crate) mod discovery;
mod openai;
mod openai_responses;
mod redaction;
mod rejection;
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
pub use openai::OpenAiProvider;
pub use openai_responses::OpenAiResponsesProvider;

#[cfg(test)]
mod account_identity_tests;
#[cfg(test)]
mod failure_tests;

pub(crate) const USAGE_LIMIT_MESSAGE: &str = "Provider account usage limit reached. Wait for the account allowance to reset before sending another message.";

pub(crate) const CONTEXT_LENGTH_MESSAGE: &str =
    "Provider rejected the request because it exceeds the model context length.";

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("authentication failed: {0}")]
    Authentication(String),
    #[error("{USAGE_LIMIT_MESSAGE}")]
    UsageLimit,
    #[error("{CONTEXT_LENGTH_MESSAGE}")]
    ContextLength,
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
    /// A transport failure does not establish whether the remote effect occurred.
    #[error("provider transport failed: {0}")]
    Transport(String),
    #[error("provider connection could not be established")]
    Connection,
    /// Observed HTTP metadata; classification and retry policy belong to the source.
    #[error("{source}")]
    HttpStatus {
        source: Box<ProviderError>,
        status: u16,
        request_id: Option<String>,
    },
    #[error("{source}")]
    Code {
        source: Box<ProviderError>,
        code: &'static str,
    },
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
    ResponseMetadata {
        status: u16,
        request_id: Option<String>,
    },
    Delta(ProviderDelta),
    Completed(Box<ModelResponse>),
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

/// Only opaque bounded correlation identifiers, never arbitrary header text.
pub(crate) fn upstream_request_id(response: &reqwest::Response) -> Option<String> {
    ["x-request-id", "request-id", "x-oai-request-id"]
        .into_iter()
        .find_map(|name| {
            let value = response.headers().get(name)?.to_str().ok()?;
            (!value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b)))
            .then(|| value.to_owned())
        })
}

pub(crate) fn observed_stream(
    response: reqwest::Response,
    decode: impl FnOnce(reqwest::Response) -> ProviderStream,
) -> ProviderStream {
    use futures_util::StreamExt;
    let status = response.status().as_u16();
    let request_id = upstream_request_id(&response);
    let first = ProviderStreamEvent::ResponseMetadata { status, request_id };
    Box::pin(futures_util::stream::once(async move { Ok(first) }).chain(decode(response)))
}

impl ProviderError {
    /// Semantic consumers such as device enrollment must not match metadata wrappers.
    pub(crate) fn into_semantic(self) -> Self {
        match self {
            Self::Code { source, .. }
            | Self::HttpStatus { source, .. }
            | Self::RetryAfter { source, .. } => source.into_semantic(),
            other => other,
        }
    }

    pub fn upstream_request_id(&self) -> Option<&str> {
        match self {
            Self::HttpStatus {
                request_id, source, ..
            } => request_id
                .as_deref()
                .or_else(|| source.upstream_request_id()),
            Self::Code { source, .. } | Self::RetryAfter { source, .. } => {
                source.upstream_request_id()
            }
            _ => None,
        }
    }
    fn with_upstream_id(self, request_id: Option<String>) -> Self {
        match self {
            Self::HttpStatus { source, status, .. } => Self::HttpStatus {
                source,
                status,
                request_id,
            },
            other => other,
        }
    }

    pub fn safe_code(&self) -> Option<&'static str> {
        match self {
            Self::Code { code, .. } => Some(code),
            Self::HttpStatus { source, .. } | Self::RetryAfter { source, .. } => source.safe_code(),
            _ => None,
        }
    }

    /// Stable, content-free classification suitable for public failure metadata.
    pub fn category(&self) -> &'static str {
        match self {
            Self::Code { source, .. }
            | Self::HttpStatus { source, .. }
            | Self::RetryAfter { source, .. } => source.category(),
            Self::Authentication(_) => "authentication",
            Self::UsageLimit => "usage_limit",
            Self::ContextLength => "context_length",
            Self::RateLimit { .. } => "rate_limit",
            Self::Unavailable(_) => "unavailable",
            Self::Timeout(_) => "timeout",
            Self::Transport(_) => "transport",
            Self::Connection => "connection",
            Self::Request(_) => "request",
            Self::InvalidResponse(_) => "invalid_response",
            Self::Incomplete => "incomplete",
        }
    }

    pub fn is_context_length(&self) -> bool {
        match self {
            Self::Code { source, .. }
            | Self::HttpStatus { source, .. }
            | Self::RetryAfter { source, .. } => source.is_context_length(),
            Self::ContextLength => true,
            _ => false,
        }
    }

    pub fn is_incomplete(&self) -> bool {
        match self {
            Self::Code { source, .. }
            | Self::HttpStatus { source, .. }
            | Self::RetryAfter { source, .. } => source.is_incomplete(),
            Self::Incomplete => true,
            _ => false,
        }
    }

    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::HttpStatus { status, .. } => Some(*status),
            Self::Code { source, .. } | Self::RetryAfter { source, .. } => source.http_status(),
            _ => None,
        }
    }

    pub(crate) fn with_http_status(self, status: u16) -> Self {
        Self::HttpStatus {
            source: Box::new(self),
            status,
            request_id: None,
        }
    }

    pub fn is_retryable(&self) -> bool {
        if let Self::Code { source, .. }
        | Self::RetryAfter { source, .. }
        | Self::HttpStatus { source, .. } = self
        {
            return source.is_retryable();
        }
        matches!(
            self,
            Self::RateLimit { .. } | Self::Unavailable(_) | Self::Timeout(_) | Self::Connection
        )
    }
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::Code { source, .. } | Self::HttpStatus { source, .. } => source.retry_after(),
            Self::RateLimit { retry_after, .. } => *retry_after,
            Self::RetryAfter { delay, .. } => Some(*delay),
            _ => None,
        }
    }

    pub(crate) fn public_failure_reason(&self) -> &'static str {
        match self {
            Self::Code { source, .. }
            | Self::RetryAfter { source, .. }
            | Self::HttpStatus { source, .. } => source.public_failure_reason(),
            Self::Authentication(_) => {
                "Provider authentication failed. Check credentials on the executing machine."
            }
            Self::UsageLimit => USAGE_LIMIT_MESSAGE,
            Self::ContextLength => CONTEXT_LENGTH_MESSAGE,
            Self::RateLimit { .. } => "Provider rate limit prevented completion.",
            Self::Unavailable(_) => "Provider temporarily unavailable.",
            Self::Timeout(_) => "Provider request timed out.",
            Self::Transport(_) => "Provider connection failed; request outcome may be uncertain.",
            Self::Connection => "Provider connection could not be established.",
            Self::Request(_) => "Provider request failed.",
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
            Ok(ProviderStreamEvent::Completed(Box::new(response)))
        })))
    }
}

pub fn from_config(config: &Config) -> Result<Box<dyn Provider>, ProviderError> {
    from_config_with_redactor(config, None)
}

pub(crate) fn from_config_with_redactor(
    config: &Config,
    redactor: Option<std::sync::Arc<crate::tools::Redactor>>,
) -> Result<Box<dyn Provider>, ProviderError> {
    config
        .validate_account()
        .map_err(|e| ProviderError::Authentication(e.to_string()))?;
    if config.account.is_some() {
        return Ok(Box::new(BoundProvider {
            config: config.clone(),
            redactor,
        }));
    }
    native_from_config(config, redactor)
}

fn native_from_config(
    config: &Config,
    redactor: Option<std::sync::Arc<crate::tools::Redactor>>,
) -> Result<Box<dyn Provider>, ProviderError> {
    config
        .validate_account()
        .map_err(|e| ProviderError::Authentication(e.to_string()))?;
    validate_config_endpoints(config)?;
    validate_inference_settings(config)
        .map_err(|error| ProviderError::Request(error.to_string()))?;
    match config.provider {
        ProviderKind::OpenaiResponses => Ok(Box::new(
            OpenAiResponsesProvider::new(
                config
                    .api_key()
                    .map_err(|e| ProviderError::Authentication(e.to_string()))?,
                config.base_url.clone(),
            )
            .with_account(config, redactor.clone()),
        )),
        ProviderKind::OpenaiChat => Ok(Box::new(
            OpenAiProvider::new(
                config
                    .api_key()
                    .map_err(|e| ProviderError::Authentication(e.to_string()))?,
                config.base_url.clone(),
            )
            .with_account(config, redactor.clone())
            .with_max_tokens_parameter(config.chat_use_max_tokens),
        )),
        ProviderKind::ChatGptOauth => {
            if let Some(binding) = &config.account {
                return crate::accounts::Registry::default_host()
                    .and_then(|registry| registry.oauth_provider(binding))
                    .map(|provider| {
                        Box::new(
                            provider
                                .with_authority(config.provider_authority.clone())
                                .with_redactor(redactor.clone()),
                        ) as Box<dyn Provider>
                    })
                    .map_err(|e| ProviderError::Authentication(e.to_string()));
            }
            let store = ChatGptTokenStore::new(ChatGptTokenStore::default_path()?);
            let mut endpoints = OAuthEndpoints::default();
            if let Some(base) = config.chatgpt_base_url.as_deref() {
                let base = base.trim_end_matches('/');
                endpoints.responses = format!("{base}/responses");
                endpoints.models = format!("{base}/models");
            }
            Ok(Box::new(
                ChatGptOauthProvider::from_store(store, endpoints).with_redactor(redactor),
            ))
        }
        ProviderKind::Anthropic => Ok(Box::new(
            AnthropicProvider::new(
                config
                    .api_key()
                    .map_err(|e| ProviderError::Authentication(e.to_string()))?,
                config.base_url.clone(),
            )
            .with_account(config, redactor.clone()),
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
        ))
        .with_http_status(response.status().as_u16()));
    }
    Ok(())
}

/// Never retain reqwest diagnostics (which can contain URLs or credentials).
/// Builder errors are local; other non-timeout transport outcomes are uncertain.
fn connection_refused(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut source = Some(error);
    while let Some(error) = source {
        if error.downcast_ref::<std::io::Error>().is_some_and(|e| {
            matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::NetworkUnreachable
                    | std::io::ErrorKind::HostUnreachable
            )
        }) {
            return true;
        }
        source = error.source();
    }
    false
}

pub(crate) fn map_transport(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout("request deadline elapsed".into())
    } else if error.is_connect() && connection_refused(&error) {
        ProviderError::Connection
    } else if error.is_builder() {
        ProviderError::Request("invalid local HTTP request".into())
    } else {
        ProviderError::Transport("connection failed; outcome may be uncertain".into())
    }
}

pub(crate) async fn checked_json(
    response: reqwest::Response,
) -> Result<serde_json::Value, ProviderError> {
    reject_redirect(&response)?;
    let request_id = upstream_request_id(&response);
    let status = response.status();
    let retry_after = response_retry_after(&response);
    let body = response
        .text()
        .await
        .map_err(|e| map_transport(e).with_http_status(status.as_u16()))?;
    let result = if status.is_success() {
        serde_json::from_str(&body)
            .map_err(|e| ProviderError::InvalidResponse(format!("{e}: {body}")))
    } else if status.as_u16() == 401 || status.as_u16() == 403 {
        Err(ProviderError::Authentication(body))
    } else if let Some(error) = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| rejection::classify(&value, Some(status.as_u16())))
    {
        Err(error)
    } else if status.as_u16() == 429 {
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
    };
    result.map_err(|error| {
        error
            .with_http_status(status.as_u16())
            .with_upstream_id(request_id)
    })
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

/// Own the admitted configuration, not a credential snapshot. Every request and
/// retry constructs a native adapter with the current same-identity credential.
struct BoundProvider {
    config: Config,
    redactor: Option<std::sync::Arc<crate::tools::Redactor>>,
}
#[async_trait]
impl Provider for BoundProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        native_from_config(&self.config, self.redactor.clone())?
            .models()
            .await
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        native_from_config(&self.config, self.redactor.clone())?
            .complete(request)
            .await
    }
    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        native_from_config(&self.config, self.redactor.clone())?
            .stream(request)
            .await
    }
}

fn check_provider_authority(
    authority: &Option<std::sync::Arc<dyn crate::policy::ExecutionAuthority>>,
) -> Result<(), ProviderError> {
    if let Some(authority) = authority {
        authority.check().map_err(|_| {
            ProviderError::Authentication("executing-host account authority withdrawn".into())
        })?;
    }
    Ok(())
}
