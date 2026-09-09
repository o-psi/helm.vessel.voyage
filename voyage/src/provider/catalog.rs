//! Shared confidentiality, terminal-text and resource bounds for model catalogs.
use super::{ModelInfo, ProviderError};
use serde_json::Value;

pub(crate) const MAX_MODELS: usize = 1024;
pub(crate) const MAX_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_PAGES: usize = 16;

fn invalid() -> ProviderError {
    ProviderError::InvalidResponse(
        "model-list failure: invalid, oversized or credential-bearing metadata".into(),
    )
}

pub(crate) fn validate_text(
    text: &str,
    limit: usize,
    required: bool,
    secrets: &[&str],
) -> Result<(), ProviderError> {
    if text.len() > limit || (required && text.trim().is_empty())
        || text.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{206f}'))
        || secrets.iter().any(|secret| !secret.is_empty() && text.contains(secret)) {
        return Err(invalid());
    }
    Ok(())
}

/// Validate without rewriting identifiers or leaking rejected provider metadata.
pub fn validate_model(model: &ModelInfo, secrets: &[&str]) -> Result<(), ProviderError> {
    validate_text(&model.id, 512, true, secrets)?;
    validate_text(&model.display_name, 512, true, secrets)?;
    validate_text(&model.description, 4096, false, secrets)?;
    for fields in [
        &model.reasoning_efforts,
        &model.input_modalities,
        &model.service_tiers,
    ] {
        if fields.len() > 16 {
            return Err(invalid());
        }
        for field in fields {
            validate_text(field, 128, true, secrets)?;
        }
    }
    if let Some(default) = &model.default_reasoning_effort {
        validate_text(default, 128, true, secrets)?;
        if (model.reasoning_support_known || !model.reasoning_efforts.is_empty())
            && !model.reasoning_efforts.contains(default)
        {
            return Err(invalid());
        }
    }
    if let Some(default) = &model.default_service_tier {
        validate_text(default, 128, true, secrets)?;
        if default != "default"
            && (model.service_support_known || !model.service_tiers.is_empty())
            && !model.service_tiers.contains(default)
        {
            return Err(invalid());
        }
    }
    Ok(())
}

pub fn validate_models(models: &[ModelInfo], secrets: &[&str]) -> Result<(), ProviderError> {
    if models.len() > MAX_MODELS {
        return Err(invalid());
    }
    let mut bytes = 0usize;
    for model in models {
        validate_model(model, secrets)?;
        bytes = bytes.saturating_add(
            model.id.len()
                + model.display_name.len()
                + model.description.len()
                + model
                    .default_reasoning_effort
                    .as_ref()
                    .map_or(0, String::len)
                + model.default_service_tier.as_ref().map_or(0, String::len)
                + model.service_tiers.iter().map(String::len).sum::<usize>()
                + model
                    .reasoning_efforts
                    .iter()
                    .map(String::len)
                    .sum::<usize>()
                + model
                    .input_modalities
                    .iter()
                    .map(String::len)
                    .sum::<usize>(),
        );
        if bytes > MAX_BYTES {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Validate the complete displayed catalog, including a local current-model entry.
/// The caller supplies its effective runtime secret detector without transferring secrets.
pub fn validate_models_for_display(
    models: &[ModelInfo],
    contains_secret: impl Fn(&str) -> bool,
) -> Result<(), ProviderError> {
    validate_models(models, &[])?;
    for model in models {
        let fields = [&model.id, &model.display_name, &model.description]
            .into_iter()
            .chain(model.default_service_tier.iter())
            .chain(model.service_tiers.iter())
            .chain(model.default_reasoning_effort.iter())
            .chain(model.reasoning_efforts.iter())
            .chain(model.input_modalities.iter());
        if fields.into_iter().any(|field| contains_secret(field)) {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(crate) fn optional_text<'a>(
    value: &'a Value,
    key: &str,
    default: &'a str,
) -> Result<&'a str, ProviderError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::String(text)) => Ok(text),
        _ => Err(invalid()),
    }
}

pub(crate) fn optional_bool(
    value: &Value,
    key: &str,
    default: bool,
) -> Result<bool, ProviderError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(flag)) => Ok(*flag),
        _ => Err(invalid()),
    }
}

pub(crate) fn strings(
    value: &Value,
    key: &str,
    nested: Option<&str>,
) -> Result<Vec<String>, ProviderError> {
    let entries = match value.get(key) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(entries)) if entries.len() <= 16 => entries,
        _ => return Err(invalid()),
    };
    entries
        .iter()
        .map(|entry| {
            let entry = if let Some(key) = nested {
                entry.get(key).ok_or_else(invalid)?
            } else {
                entry
            };
            entry.as_str().map(str::to_owned).ok_or_else(invalid)
        })
        .collect()
}

pub(crate) fn transport(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout("model discovery timed out".into())
    } else if error.is_connect() {
        ProviderError::Unavailable("model discovery connection failed".into())
    } else {
        ProviderError::Request("model discovery connection failed".into())
    }
}

/// Budget counts all downloaded pages, including unrecognized JSON fields.
pub(crate) async fn json(
    mut response: reqwest::Response,
    remaining: &mut usize,
) -> Result<Value, ProviderError> {
    super::reject_redirect(&response).map_err(|error| match error {
        ProviderError::Request(message) => {
            ProviderError::Request(format!("protocol failure: {message}"))
        }
        other => other,
    })?;
    let code = response.status().as_u16();
    match code {
        200..=299 => {}
        401 | 403 => {
            return Err(ProviderError::Authentication(
                "authentication failure: endpoint rejected credentials".into(),
            ));
        }
        429 => {
            return Err(ProviderError::RateLimit {
                message: "model endpoint rate limited".into(),
                retry_after: super::response_retry_after(&response),
            });
        }
        408 | 409 | 500..=599 => {
            let error = ProviderError::Unavailable(format!("model endpoint HTTP {code}"));
            return Err(match super::response_retry_after(&response) {
                Some(delay) => ProviderError::RetryAfter {
                    source: Box::new(error),
                    delay,
                },
                None => error,
            });
        }
        _ => {
            return Err(ProviderError::Request(format!(
                "model endpoint HTTP {code}"
            )));
        }
    }
    if response
        .content_length()
        .is_some_and(|length| length > *remaining as u64)
    {
        return Err(ProviderError::InvalidResponse(
            "protocol failure: model list exceeds 1 MiB".into(),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if chunk.len() > *remaining {
            return Err(ProviderError::InvalidResponse(
                "protocol failure: model list exceeds 1 MiB".into(),
            ));
        }
        *remaining -= chunk.len();
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        ProviderError::InvalidResponse("protocol failure: invalid model-list JSON".into())
    })
}

/// Nullable optional strings retain absence instead of inventing a default.
pub(crate) fn nullable_text(value: &Value, key: &str) -> Result<Option<String>, ProviderError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        _ => Err(invalid()),
    }
}
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().min(u64::MAX as u128) as u64)
}
