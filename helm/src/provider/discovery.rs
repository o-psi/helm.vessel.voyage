//! Bounded OpenAI-compatible model discovery shared by setup and native providers.
use super::{CompatibleAuthentication, ModelInfo, ProviderError, normalize_models};
use serde_json::Value;

pub(crate) async fn models(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
) -> Result<Option<Vec<ModelInfo>>, ProviderError> {
    let mut response = client
        .get(format!("{endpoint}/models"))
        .apply_key(key)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                ProviderError::Timeout("model discovery deadline elapsed".into())
            } else {
                ProviderError::Request("connection failure: model endpoint unavailable".into())
            }
        })?;
    let code = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs);
    match code {
        404 | 405 | 501 => return Ok(None),
        401 | 403 => {
            return Err(ProviderError::Authentication(
                "authentication failure: endpoint rejected credentials".into(),
            ));
        }
        429 => {
            return Err(ProviderError::RateLimit {
                message: "model endpoint rate limited".into(),
                retry_after,
            });
        }
        408 | 409 | 500..=599 => {
            return Err(ProviderError::Unavailable(format!(
                "model endpoint HTTP {code}"
            )));
        }
        200..=299 => {}
        _ => {
            return Err(ProviderError::Request(format!(
                "protocol failure: model endpoint HTTP {code}"
            )));
        }
    }
    // Never include a provider body, URL, model ID or parser diagnostic in errors.
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProviderError::Request("connection failure while reading model list".into()))?
    {
        if bytes.len().saturating_add(chunk.len()) > 1024 * 1024 {
            return Err(ProviderError::InvalidResponse(
                "protocol failure: model list exceeds 1 MiB".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        ProviderError::InvalidResponse("protocol failure: invalid model-list JSON".into())
    })?;
    let data = value.get("data").and_then(Value::as_array).ok_or_else(|| {
        ProviderError::InvalidResponse("model-list failure: missing data array".into())
    })?;
    if data.len() > 1024 {
        return Err(ProviderError::InvalidResponse(
            "model-list failure: more than 1024 models".into(),
        ));
    }
    let mut models = Vec::with_capacity(data.len());
    for item in data {
        let id = item.get("id").and_then(Value::as_str).ok_or_else(|| {
            ProviderError::InvalidResponse("model-list failure: missing model ID".into())
        })?;
        if id.trim().is_empty()
            || id.len() > 512
            || id.chars().any(char::is_control)
            || (!key.is_empty() && id.contains(key))
        {
            return Err(ProviderError::InvalidResponse(
                "model-list failure: invalid or credential-bearing model ID".into(),
            ));
        }
        models.push(ModelInfo::minimal(id));
    }
    normalize_models(&mut models);
    Ok(Some(models))
}
