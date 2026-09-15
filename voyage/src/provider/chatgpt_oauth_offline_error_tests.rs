use super::*;
use serde_json::json;

#[test]
fn callback_encoding_roundtrips_unicode_and_rejects_malformed_escapes() {
    for text in [
        "",
        "simple",
        "a & b=two",
        "é/你好",
        "plus+space ",
        "%literal",
    ] {
        assert_eq!(percent_decode(&percent(text)).unwrap(), text);
    }
    assert_eq!(percent_decode("a+b%2Bc").unwrap(), "a b+c");
    for text in ["%", "%0", "%GG", "%FF", "%C3", "%é"] {
        assert!(percent_decode(text).is_err());
    }
    assert_eq!(
        form_urlencoded(&[("a b", "x&y"), ("empty", "")]),
        "a%20b=x%26y&empty="
    );
}

#[test]
fn private_storage_errors_are_sanitized_but_pending_refresh_is_distinct() {
    let error = account_error(anyhow::anyhow!("PRIVATE /credential/path"));
    assert_eq!(error.category(), "authentication");
    assert!(!error.to_string().contains("PRIVATE"));
    assert!(!error.to_string().contains("/credential/path"));
    let pending = account_error(RefreshPending.into());
    assert!(
        matches!(pending, ProviderError::Unavailable(ref message) if message == REFRESH_PENDING)
    );
    assert!(pending.is_retryable());
    assert_eq!(private_cache_error().category(), "authentication");
}

#[test]
fn reported_service_tier_cannot_leak_any_token_or_account_identifier() {
    let tokens = OAuthTokens {
        access_token: "offline-access".into(),
        refresh_token: "offline-refresh".into(),
        id_token: Some("offline-id".into()),
        expires_at: 1,
        account_id: "offline-account".into(),
    };
    for tier in [
        "priority",
        "offline-access",
        "offline-refresh",
        "offline-id",
        "offline-account",
        "bad\ntier",
        "",
    ] {
        let mut response = ModelResponse {
            message: crate::model::Message::new(crate::model::Role::Assistant, "answer"),
            usage: Default::default(),
            service_tier: Some(tier.into()),
        };
        filter_response_tier(&mut response, &tokens);
        assert_eq!(
            response.service_tier.as_deref(),
            if tier == "priority" {
                Some("priority")
            } else {
                None
            }
        );
    }
}

#[test]
fn token_response_defaults_and_required_fields_are_explicit() {
    let response: TokenResponse =
        serde_json::from_value(json!({"access_token":"offline"})).unwrap();
    assert_eq!(response.expires_in, default_expiry());
    assert!(response.refresh_token.is_none());
    assert!(response.id_token.is_none());
    assert!(response.account_id.is_none());
    assert!(serde_json::from_value::<TokenResponse>(json!({})).is_err());
    for value in [json!(null), json!(""), json!(7)] {
        assert!(required_string(&json!({"code":value}), "code").is_err());
    }
    assert_eq!(
        required_string(&json!({"code":"offline"}), "code").unwrap(),
        "offline"
    );
}
