//! Local GitHub context and exact attended publication.
pub mod admin;

pub mod approval;

pub mod context;

pub mod logs;
pub mod operator;

pub mod tool;

/// Supported diagnostic projections for the explicitly delegated credential.
/// This is redaction, not a claim to detect arbitrary encodings or exfiltration.
#[derive(Clone)]
pub struct Credential(std::sync::Arc<zeroize::Zeroizing<String>>);
impl Credential {
    pub fn from_config(config: &crate::Config) -> Option<Self> {
        if !config.github_enabled {
            return None;
        }
        std::env::var("HELM_GITHUB_TOKEN")
            .ok()
            .map(|token| Self(std::sync::Arc::new(zeroize::Zeroizing::new(token))))
    }
    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }
}

pub fn credential_redactions(config: &crate::Config) -> Vec<String> {
    Credential::from_config(config)
        .map(|credential| credential_forms(credential.expose()))
        .unwrap_or_default()
}
pub(crate) fn credential_forms(token: &str) -> Vec<String> {
    use base64::Engine;
    if token.len() < 4 {
        return Vec::new();
    }
    vec![
        token.to_owned(),
        base64::engine::general_purpose::STANDARD.encode(token),
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(token),
        base64::engine::general_purpose::URL_SAFE.encode(token),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token),
        hex::encode(token),
        hex::encode_upper(token),
    ]
}
pub mod publication;
pub mod repository;
pub mod service;
pub mod store;
mod transport;

pub(super) fn value_has_secret(
    value: &serde_json::Value,
    redactor: &crate::tools::Redactor,
) -> bool {
    use serde_json::Value;
    match value {
        Value::String(text) => redactor.contains_secret(text),
        Value::Array(items) => items.iter().any(|item| value_has_secret(item, redactor)),
        Value::Object(items) => items
            .iter()
            .any(|(key, item)| redactor.contains_secret(key) || value_has_secret(item, redactor)),
        _ => false,
    }
}

pub(super) fn redact_value(
    value: &serde_json::Value,
    redactor: &crate::tools::Redactor,
) -> serde_json::Value {
    use serde_json::Value;
    fn walk(value: &Value, redactor: &crate::tools::Redactor, collision: &mut bool) -> Value {
        match value {
            Value::String(text) => Value::String(redactor.redact(text)),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| walk(item, redactor, collision))
                    .collect(),
            ),
            Value::Object(items) => {
                let mut output = serde_json::Map::new();
                for (key, item) in items {
                    let key = redactor.redact(key);
                    let item = walk(item, redactor, collision);
                    *collision |= output.insert(key, item).is_some();
                }
                Value::Object(output)
            }
            _ => value.clone(),
        }
    }
    let mut collision = false;
    let value = walk(value, redactor, &mut collision);
    if collision {
        serde_json::json!({"incomplete":true,"reason":"Secret redaction merged object keys; some display fields are omitted.","data":value})
    } else {
        value
    }
}
