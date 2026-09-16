use super::*;
use serde_json::{Value, json};

fn workspace() -> Value {
    json!({"schema_version":1,"kind":"workspace","endpoint":"https://example.invalid","grant_id":Uuid::new_v4(),"principal_id":Uuid::new_v4(),"vessel_id":Uuid::new_v4(),"token":"offline-fixture-token"})
}

#[test]
fn credential_validation_does_not_disclose_input_and_checks_all_fields() {
    let original = workspace();
    let parsed = Credential::parse(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(parsed.endpoint(), "https://example.invalid");
    assert_eq!(parsed.token(), "offline-fixture-token");
    assert_eq!(
        parsed.vessel_id().unwrap().to_string(),
        original["vessel_id"].as_str().unwrap()
    );
    assert_eq!(
        parsed.principal_id().unwrap().to_string(),
        original["principal_id"].as_str().unwrap()
    );
    for (field, value) in [
        ("schema_version", json!(2)),
        ("kind", json!("other")),
        ("vessel_id", json!(Uuid::nil())),
        ("principal_id", json!(Uuid::nil())),
        ("grant_id", json!(Uuid::nil())),
        ("token", json!("")),
        ("token", json!("a b")),
        ("token", json!("é")),
        ("token", json!("x".repeat(4097))),
        ("endpoint", json!("http://example.invalid")),
        ("extra", json!("not accepted")),
    ] {
        let mut invalid = original.clone();
        invalid[field] = value;
        let error = Credential::parse(&serde_json::to_vec(&invalid).unwrap())
            .err()
            .unwrap();
        assert!(!error.to_string().contains("offline-fixture-token"));
    }
    assert!(Credential::parse(b"not json").is_err());
}

#[test]
fn endpoint_rejects_ambient_authority_and_preserves_only_approved_origin() {
    for base in ["https://example.invalid/base", "http://127.0.0.1:9876/base"] {
        let url = endpoint(base, "/v1/command").unwrap();
        assert_eq!(url.path(), "/v1/command");
        assert!(url.query().is_none());
    }
    for base in [
        "not a url",
        "file:///tmp/x",
        "http://localhost",
        "http://example.invalid",
        "https://user@example.invalid",
        "https://user:password@example.invalid",
        "https://example.invalid?q=x",
        "https://example.invalid#fragment",
    ] {
        assert!(endpoint(base, "/").is_err(), "{base}");
    }
}

#[test]
fn authorization_headers_are_pinned_and_never_in_urls() {
    let value = workspace();
    let credential = Credential::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let request = credential
        .authorize(
            http.post("https://example.invalid/command"),
            credential.vessel_id(),
        )
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        request.headers()["authorization"],
        "Bearer offline-fixture-token"
    );
    assert_eq!(
        request.headers()["x-voyage-grant"],
        credential.grant_id().to_string()
    );
    assert_eq!(
        request.headers()["x-voyage-vessel"],
        credential.vessel_id().unwrap().to_string()
    );
    assert!(!request.url().as_str().contains("token"));
    assert!(
        credential
            .authorize(http.post("https://example.invalid"), Some(Uuid::new_v4()))
            .is_err()
    );
}

#[test]
fn protocol_error_classification_has_stable_precedence() {
    for (text, expected) in [
        ("REVOKED grant", ConnectionFailure::Revoked),
        ("expired pairing", ConnectionFailure::Expired),
        ("version mismatch", ConnectionFailure::Version),
        ("protocol unsupported", ConnectionFailure::Version),
    ] {
        assert_eq!(classified(text), Some(expected));
    }
    assert_eq!(classified("unclassified failure"), None);
    assert_eq!(
        classified("expired and revoked"),
        Some(ConnectionFailure::Unavailable)
    );
}
