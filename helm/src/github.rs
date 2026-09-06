//! Local GitHub context and exact attended publication.
pub mod admin;
pub mod approval;
#[cfg(test)]
mod approval_fixture;
#[cfg(test)]
mod authorization_fixture;
pub mod context;
#[cfg(test)]
mod context_fixture;
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
    #[cfg(test)]
    pub(crate) fn fixture(token: &str) -> Self {
        Self(std::sync::Arc::new(zeroize::Zeroizing::new(token.into())))
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

#[cfg(test)]
mod credential_tests {
    #[test]
    fn raw_json_secrets_are_refused_before_escaping_and_display_redacts_recursively() {
        let secret = "private\nquoted\"credential";
        let redactor = crate::tools::Redactor::new([secret.into()]);
        let value = serde_json::json!({"operation":{"actor":secret,"disposition":[secret]}});
        let encoded = serde_json::to_string_pretty(&value).unwrap();
        // The previous post-serialization strategy cannot see this binding.
        assert_eq!(redactor.redact(&encoded), encoded);
        assert!(super::value_has_secret(&value, &redactor));
        let projected = super::redact_value(&value, &redactor);
        assert_eq!(projected["operation"]["actor"], "[REDACTED]");
        assert_eq!(projected["operation"]["disposition"][0], "[REDACTED]");
        assert_eq!(
            value["operation"]["actor"], secret,
            "exact source must stay unchanged"
        );
        let key_value = serde_json::Value::Object(serde_json::Map::from_iter([(
            secret.to_owned(),
            serde_json::Value::String("public".into()),
        )]));
        assert!(super::value_has_secret(&key_value, &redactor));
        assert_eq!(
            super::redact_value(&key_value, &redactor)["[REDACTED]"],
            "public"
        );
    }
    #[test]
    fn redacted_key_collisions_are_explicitly_incomplete() {
        let redactor = crate::tools::Redactor::new(["private-one".into(), "private-two".into()]);
        let value = serde_json::json!({"nested":{"private-one":1,"private-two":2}});
        let projected = super::redact_value(&value, &redactor);
        assert_eq!(projected["incomplete"], true);
        assert_eq!(projected["data"]["nested"].as_object().unwrap().len(), 1);
        assert_eq!(value["nested"].as_object().unwrap().len(), 2);
    }
    #[test]
    fn explicit_configured_github_credentials_redact_supported_forms() {
        let mut config = crate::Config::default();
        let token = "fixture-github-secret-9876";
        config.env.insert("HELM_GITHUB_TOKEN".into(), token.into());
        assert!(super::credential_redactions(&config).is_empty());
        let credential = super::Credential::fixture(token);
        let forms = super::credential_forms(credential.expose());
        let redactor = crate::tools::Redactor::new(forms.clone());
        for form in forms {
            assert_eq!(
                redactor.redact(format!("before {form} after")),
                "before [REDACTED] after"
            );
            assert!(redactor.contains_secret(&form));
        }
        let mut undelegated = crate::Config::default();
        undelegated.env.insert("GH_TOKEN".into(), token.into());
        assert!(super::credential_redactions(&undelegated).is_empty());
    }
}
