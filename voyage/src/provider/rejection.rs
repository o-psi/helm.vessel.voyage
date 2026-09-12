//! Content-free classification of explicit provider rejections, never response text.
use super::ProviderError;
use serde_json::Value;

/// Handles HTTP error envelopes, Responses failed events, and flat SSE errors.
/// Call only for a rejected request/error event, not arbitrary successful output.
/// HTTP authentication and output-incomplete handling take precedence at callers.
pub(super) fn classify(value: &Value, status: Option<u16>) -> Option<ProviderError> {
    let error = value
        .pointer("/response/error")
        .or_else(|| value.get("error"))
        .unwrap_or(value);
    let code = error.get("code").and_then(Value::as_str);
    let kind = error.get("type").and_then(Value::as_str);
    // Account exhaustion must never become context recovery or transient retry.
    if [code, kind]
        .into_iter()
        .flatten()
        .any(|code| matches!(code, "usage_limit_reached" | "insufficient_quota"))
    {
        let code = if code == Some("insufficient_quota") || kind == Some("insufficient_quota") {
            "insufficient_quota"
        } else {
            "usage_limit_reached"
        };
        return Some(ProviderError::Code {
            source: Box::new(ProviderError::UsageLimit),
            code,
        });
    }
    // OpenAI Chat, Responses and ChatGPT use this explicit input rejection code.
    if code == Some("context_length_exceeded") || kind == Some("context_length_exceeded") {
        return Some(ProviderError::Code {
            source: Box::new(ProviderError::ContextLength),
            code: "context_length_exceeded",
        });
    }
    // Anthropic uses invalid_request_error with "prompt is too long: N tokens >
    // M maximum". Older OpenAI errors use "This model's maximum context length
    // is N tokens...". Require that error type and HTTP 400 (or an SSE error),
    // rather than matching generic token/length/413/rate-limit diagnostics.
    if !matches!(status, None | Some(400)) || kind != Some("invalid_request_error") {
        return None;
    }
    let message = error.get("message").and_then(Value::as_str)?;
    let anthropic = message.starts_with("prompt is too long: ")
        && message.contains(" tokens > ")
        && message.contains(" maximum");
    let openai = message.starts_with("This model's maximum context length is ")
        && message.contains(" tokens");
    (anthropic || openai).then_some(ProviderError::ContextLength)
}
