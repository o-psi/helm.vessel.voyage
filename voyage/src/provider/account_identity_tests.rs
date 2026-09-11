use super::chatgpt_oauth::{OAuthTokens, login_identity, validate_tokens};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;

fn token(subject: &str, account: &str) -> String {
    format!("fixture.{}.signature", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({
        "sub": subject, "https://api.openai.com/auth": {"chatgpt_account_id": account}
    })).unwrap()))
}
fn tokens(subject: &str) -> OAuthTokens {
    OAuthTokens { access_token: token(subject, "billing-context"), refresh_token: "synthetic-refresh".into(),
        id_token: None, account_id: "billing-context".into(), expires_at: 4_000_000_000 }
}
#[test]
fn same_billing_different_login_is_different_identity() {
    assert_ne!(login_identity(&tokens("personal-user")).unwrap(), login_identity(&tokens("work-user")).unwrap());
    let mut refreshed = tokens("personal-user");
    refreshed.refresh_token = "synthetic-rotated".into();
    refreshed.expires_at += 100;
    assert_eq!(login_identity(&tokens("personal-user")).unwrap(), login_identity(&refreshed).unwrap());
}
#[test]
fn conflicting_claims_fail_without_echoing_tokens() {
    let mut t = tokens("personal-user");
    t.id_token = Some(token("other-user", "billing-context"));
    assert!(validate_tokens(&t).is_err());
    t.id_token = None;
    t.account_id = "different-billing-context".into();
    let error = validate_tokens(&t).unwrap_err().to_string();
    assert!(!error.contains(&t.access_token));
    assert!(!error.contains(&t.account_id));
}
#[test]
fn opaque_legacy_tokens_do_not_establish_login_identity() {
    let mut t = tokens("personal-user");
    t.access_token = "synthetic-opaque".into();
    assert_eq!(login_identity(&t).unwrap(), None);
    // Existing opaque credentials can remain usable until refresh, but cannot
    // authorize an unverified same-identity replacement.
    assert!(validate_tokens(&t).is_ok());
}
