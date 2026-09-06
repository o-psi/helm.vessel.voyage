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
    for fields in [&model.reasoning_efforts, &model.input_modalities] {
        if fields.len() > 16 {
            return Err(invalid());
        }
        for field in fields {
            validate_text(field, 128, true, secrets)?;
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
                retry_after: response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(std::time::Duration::from_secs),
            });
        }
        408 | 409 | 500..=599 => {
            return Err(ProviderError::Unavailable(format!(
                "model endpoint HTTP {code}"
            )));
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_validation_preserves_unicode_and_rejects_every_unsafe_field() {
        let good = ModelInfo {
            id: "éditeur/模型-🚢".into(),
            display_name: "日本語 Français العربية".into(),
            description: "A useful model".into(),
            reasoning_efforts: vec!["medium".into()],
            input_modalities: vec!["text".into()],
            is_default: false,
        };
        validate_model(&good, &["synthetic-key"]).unwrap();
        for bad in [
            "bad\x1b]52;x\x07",
            "bad\x00",
            "bad\x7f",
            "bad\u{85}",
            "bad\u{061c}",
            "bad\u{200e}",
            "bad\u{202e}",
            "bad\u{2066}",
            "synthetic-key",
        ] {
            for field in 0..5 {
                let mut model = good.clone();
                match field {
                    0 => model.id = bad.into(),
                    1 => model.display_name = bad.into(),
                    2 => model.description = bad.into(),
                    3 => model.reasoning_efforts = vec![bad.into()],
                    _ => model.input_modalities = vec![bad.into()],
                }
                let error = validate_model(&model, &["synthetic-key"])
                    .unwrap_err()
                    .to_string();
                assert!(!error.contains(bad));
            }
        }
        for field in 0..5 {
            let mut model = good.clone();
            match field {
                0 => model.id = "x".repeat(513),
                1 => model.display_name = "x".repeat(513),
                2 => model.description = "x".repeat(4097),
                3 => model.reasoning_efforts = vec!["x".repeat(129)],
                _ => model.input_modalities = vec!["text".into(); 17],
            }
            assert!(validate_model(&model, &[]).is_err());
        }
        assert!(validate_models(&vec![good.clone(); 1025], &[]).is_err());
        let mut large = good;
        large.description = "x".repeat(4096);
        assert!(validate_models(&vec![large; 1024], &[]).is_err());
        let boundary = ModelInfo::minimal("x".repeat(512));
        validate_models(&[boundary], &[]).unwrap();
    }
}
