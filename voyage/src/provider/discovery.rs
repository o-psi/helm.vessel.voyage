//! Bounded OpenAI-compatible model discovery shared by setup and native providers.
use super::{CompatibleAuthentication, ModelInfo, ProviderError, normalize_models};
use serde_json::Value;

pub(crate) async fn models(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
) -> Result<Option<Vec<ModelInfo>>, ProviderError> {
    let response = client
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
    if matches!(response.status().as_u16(), 404 | 405 | 501) {
        return Ok(None);
    }
    let mut remaining = super::catalog::MAX_BYTES;
    let value = super::catalog::json(response, &mut remaining).await?;
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
    super::validate_models(&models, &[key])?;
    normalize_models(&mut models);
    Ok(Some(models))
}
