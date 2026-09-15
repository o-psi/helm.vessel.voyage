use super::*;
use serde_json::json;

fn jwt(claims: Value) -> String {
    format!(
        "offline.{}.unsigned",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    )
}
fn tokens() -> OAuthTokens {
    OAuthTokens {
        access_token: jwt(
            json!({"sub":"subject","account_id":"offline-account","exp":4102444800_u64}),
        ),
        refresh_token: "offline-refresh-not-a-credential".into(),
        id_token: None,
        expires_at: 4102444800,
        account_id: "offline-account".into(),
    }
}
fn store(root: &tempfile::TempDir, name: &str) -> TokenStore {
    // Never consult the user's default store or mutate process environment.
    TokenStore::new(root.path().join(name).join("tokens.json"))
}

#[tokio::test]
async fn store_round_trip_logout_and_malformed_private_record() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root, "nested/cache");
    assert!(store.load().await.unwrap().is_none());
    assert!(!store.status().await.unwrap().authenticated);
    store.clear().await.unwrap();
    assert!(!store.path.exists());
    let tokens = tokens();
    store.save(&tokens).await.unwrap();
    assert!(store.load().await.unwrap().as_ref() == Some(&tokens));
    let status = store.status().await.unwrap();
    assert!(status.authenticated && status.refreshable);
    assert_eq!(status.account_id.as_deref(), Some("offline-account"));
    assert_eq!(status.expires_at, Some(tokens.expires_at));
    store.clear().await.unwrap();
    assert_eq!(storage::read(&store.path).unwrap().unwrap(), b"null");
    assert!(store.load().await.unwrap().is_none());
    store
        .publish(Some(b"not JSON PRIVATE".to_vec()))
        .await
        .unwrap();
    let error = store.load().await.err().unwrap();
    assert_eq!(error.category(), "authentication");
    assert!(!error.to_string().contains("PRIVATE"));
    store.save(&tokens).await.unwrap();
    assert!(store.load().await.unwrap().is_some());
}

#[tokio::test]
async fn imports_nested_and_flat_codex_records_without_overwriting_implicitly() {
    let root = tempfile::tempdir().unwrap();
    let destination = store(&root, "destination");
    let source = store(&root, "source");
    let tokens = tokens();
    for nested in [false, true] {
        let value = serde_json::to_value(&tokens).unwrap();
        let document = if nested {
            json!({"tokens":value})
        } else {
            value
        };
        source
            .publish(Some(serde_json::to_vec(&document).unwrap()))
            .await
            .unwrap();
        let imported = destination
            .import_codex(&source.path, nested)
            .await
            .unwrap();
        assert_eq!(imported.account_id, tokens.account_id);
        assert_eq!(imported.expires_at, tokens.expires_at);
        assert!(destination.import_codex(&source.path, false).await.is_err());
    }
    for document in [
        json!({}),
        json!({"access_token":"a"}),
        json!({"access_token":"a","refresh_token":"r"}),
        json!({"access_token":"a","refresh_token":"r","account_id":""}),
    ] {
        source
            .publish(Some(serde_json::to_vec(&document).unwrap()))
            .await
            .unwrap();
        assert!(TokenStore::read_import_tokens(&source.path).await.is_err());
        assert!(destination.load().await.unwrap().as_ref() == Some(&tokens));
    }
    source.publish(Some(b"{".to_vec())).await.unwrap();
    assert!(TokenStore::read_import_tokens(&source.path).await.is_err());
}

#[tokio::test]
async fn request_builds_headers_and_body_but_never_sends() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root, "request");
    let tokens = tokens();
    store.save(&tokens).await.unwrap();
    let mut endpoints = OAuthEndpoints::default();
    endpoints.responses = "https://offline.invalid/responses".into();
    let provider = ChatGptOAuth::new(store.clone(), endpoints).await.unwrap();
    let body = json!({"model":"offline-model","input":[],"stream":true});
    let request = provider
        .responses_request(&body)
        .await
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.url().as_str(), "https://offline.invalid/responses");
    assert_eq!(
        request.headers()["authorization"],
        format!("Bearer {}", tokens.access_token)
    );
    assert_eq!(request.headers()["chatgpt-account-id"], "offline-account");
    assert_eq!(request.headers()["originator"], "helm");
    assert_eq!(request.headers()["openai-beta"], "responses=experimental");
    assert_eq!(
        serde_json::from_slice::<Value>(request.body().unwrap().as_bytes().unwrap()).unwrap(),
        body
    );
    assert!(provider.status().await.authenticated);
    provider.logout().await.unwrap();
    assert!(!provider.status().await.authenticated);
    assert!(provider.responses_request(&body).await.is_err());
    assert!(store.load().await.unwrap().is_none());
}

#[test]
fn pkce_queries_are_encoded_and_challenges_bind_unique_verifiers() {
    let root = tempfile::tempdir().unwrap();
    let provider = ChatGptOAuth::from_store(store(&root, "pkce"), OAuthEndpoints::default());
    let first = provider.begin_pkce("http://localhost:1234/callback?a=1&b=two words");
    let second = provider.begin_pkce(first.redirect_uri.clone());
    assert_ne!(first.state, second.state);
    assert_ne!(first.verifier, second.verifier);
    assert_eq!(first.verifier.len(), 96);
    let url = reqwest::Url::parse(&first.url).unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["redirect_uri"], first.redirect_uri);
    assert_eq!(query["state"], first.state);
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(
        query["code_challenge"],
        URL_SAFE_NO_PAD.encode(Sha256::digest(first.verifier.as_bytes()))
    );
    assert_eq!(query["scope"], "openid profile email offline_access");
    assert_eq!(percent("a /+é~"), "a%20%2F%2B%C3%A9~");
}

#[test]
fn token_validation_boundaries_and_identity_consistency() {
    let good = tokens();
    validate_tokens(&good).unwrap();
    assert!(login_identity(&good).unwrap().is_some());
    for field in 0..9 {
        let mut bad = good.clone();
        match field {
            0 => bad.access_token.clear(),
            1 => bad.refresh_token.clear(),
            2 => bad.account_id.clear(),
            3 => bad.expires_at = 0,
            4 => bad.access_token = "a".repeat(16_385),
            5 => bad.refresh_token = "r".repeat(16_385),
            6 => bad.account_id = "i".repeat(513),
            7 => bad.id_token = Some("i".repeat(16_385)),
            _ => bad.refresh_token = "line\nbreak".into(),
        }
        assert!(validate_tokens(&bad).is_err(), "field {field}");
    }
    for claims in [
        json!({"sub":"other"}),
        json!({"sub":""}),
        json!({"sub":7}),
        json!({"sub":"bad\nsubject"}),
        json!({"sub":"x".repeat(513)}),
        json!({"account_id":"other"}),
        json!({"chatgpt_account_id":null}),
        json!({"https://api.openai.com/auth":{"chatgpt_account_id":"other"}}),
    ] {
        let mut bad = good.clone();
        bad.id_token = Some(jwt(claims));
        assert!(login_identity(&bad).is_err());
    }
    let mut opaque = good.clone();
    opaque.access_token = "opaque".into();
    assert!(login_identity(&opaque).unwrap().is_none());
    opaque.id_token = Some(jwt(json!({"sub":"subject","account_id":"offline-account"})));
    assert_eq!(
        login_identity(&opaque).unwrap(),
        login_identity(&good).unwrap()
    );
    for token in ["", "one", "x.!.x", "x.bnVsbA.x"] {
        assert!(account_id_from_jwt(token).is_none());
    }
    for claims in [
        json!({"account_id":"a"}),
        json!({"chatgpt_account_id":"a"}),
        json!({"https://api.openai.com/auth":{"chatgpt_account_id":"a"}}),
    ] {
        assert_eq!(account_id_from_jwt(&jwt(claims)).as_deref(), Some("a"));
    }
}

#[test]
fn device_intervals_accept_native_strings_but_reject_invalid_values() {
    for interval in [json!(3), json!("3")] {
        let value: DeviceAuthorization = serde_json::from_value(
            json!({"device_auth_id":"offline","user_code":"ABCD","interval":interval}),
        )
        .unwrap();
        assert_eq!(value.interval, 3);
        assert!(!value.verification_uri.is_empty());
        assert!(value.expires_in.is_none());
    }
    let default: DeviceAuthorization =
        serde_json::from_value(json!({"device_auth_id":"offline","user_code":"ABCD"})).unwrap();
    assert_eq!(default.interval, default_poll_interval());
    for interval in [
        json!(null),
        json!(-1),
        json!(1.5),
        json!(true),
        json!("-1"),
        json!("bogus"),
        json!("18446744073709551616"),
    ] {
        assert!(
            serde_json::from_value::<DeviceAuthorization>(
                json!({"device_auth_id":"offline","user_code":"ABCD","interval":interval})
            )
            .is_err()
        );
    }
}
