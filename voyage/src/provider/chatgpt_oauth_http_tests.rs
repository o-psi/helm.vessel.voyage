//! Synthetic owner-local OAuth transport and durable token rotation tests.
use super::*;
use crate::provider::native_http_tests::server;
use futures_util::StreamExt;
use serde_json::json;

static BROWSER_CALLBACK_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn endpoints(url: &str) -> OAuthEndpoints {
    OAuthEndpoints {
        authorize: format!("{url}/authorize"),
        token: format!("{url}/token"),
        device_user_code: format!("{url}/device"),
        device_token: format!("{url}/poll"),
        responses: format!("{url}/responses"),
        models: format!("{url}/models"),
    }
}
fn identity() -> String {
    use base64::Engine;
    format!(
        "e30.{}.fixture",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"sub":"fixture-subject","account_id":"fixture-owner"}).to_string())
    )
}
fn tokens() -> OAuthTokens {
    OAuthTokens {
        access_token: "fixture-access".into(),
        refresh_token: "fixture-refresh".into(),
        id_token: Some(identity()),
        account_id: "fixture-owner".into(),
        expires_at: u64::MAX / 2,
    }
}
fn local(url: &str) -> (tempfile::TempDir, ChatGptOAuth) {
    let dir = tempfile::tempdir().unwrap();
    let store = TokenStore::new(dir.path().join("nested/tokens.json"));
    let provider = ChatGptOAuth::from_store(store, endpoints(url));
    (dir, provider)
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "fixture-model".into(),
        messages: vec![crate::model::Message::new(crate::model::Role::User, "hi")],
        tools: vec![],
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: Some(128),
    }
}
fn response() -> Value {
    json!({"status":"completed","service_tier":"fixture-access","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}],"usage":{"input_tokens":3,"output_tokens":1}})
}
#[tokio::test]
async fn oauth_native_dispatch_filters_tiers_and_builds_subscription_requests() {
    let (url,task) = server(vec![(200,response().to_string()),(200,format!("data: {}\n\n",json!({"type":"response.completed","response":response()}))),(200,json!({"models":[{"slug":"z","supported_reasoning_levels":[{"effort":"low"}],"default_reasoning_level":"low","service_tiers":[{"id":"default"}],"default_service_tier":"default","is_default":true},{"id":"hidden","visibility":"hide"},{"id":"not-api","supported_in_api":false},{"id":"a"}]}).to_string())]).await;
    let (_dir, p) = local(&url);
    p.replace(tokens()).await.unwrap();
    let answer = p.complete(request()).await.unwrap();
    assert_eq!(answer.message.content, "answer");
    assert!(answer.service_tier.is_none());
    let events = p.stream(request()).await.unwrap().collect::<Vec<_>>().await;
    let done = events
        .into_iter()
        .map(Result::unwrap)
        .find_map(|event| match event {
            crate::provider::ProviderStreamEvent::Completed(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert!(done.service_tier.is_none());
    assert_eq!(done.usage.input_tokens, 3);
    let models = p.models().await.unwrap();
    assert_eq!(models.len(), 2);
    let z = models.iter().find(|m| m.id == "z").unwrap();
    assert_eq!(z.default_reasoning_effort.as_deref(), Some("low"));
    assert!(z.reasoning_support_known && z.service_support_known);
    let requests = task.await.unwrap();
    for raw in &requests[..2] {
        let lower = raw.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer fixture-access"));
        assert!(lower.contains("chatgpt-account-id: fixture-owner"));
        assert!(lower.contains("originator: helm"));
        let body: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert!(body.get("max_output_tokens").is_none());
        assert_eq!(body["store"], false);
    }
    assert!(requests[2].contains("client_version=99.99.99"));
    assert!(p.account_usage().await.is_err());
    assert!(p.status().await.authenticated);
    p.logout().await.unwrap();
    assert!(!p.status().await.authenticated);
    assert!(p.valid_tokens().await.is_err());
}
#[tokio::test]
async fn oauth_refresh_persists_rotation_and_preserves_omitted_refresh_token() {
    for replacement in [None, Some("rotated-refresh")] {
        let mut value = json!({"access_token":"rotated-access","expires_in":7200,"account_id":"fixture-owner","id_token":identity()});
        if let Some(token) = replacement {
            value["refresh_token"] = json!(token);
        }
        let (url, task) = server(vec![(200, value.to_string())]).await;
        let (_dir, p) = local(&url);
        let mut old = tokens();
        old.expires_at = 1;
        p.replace(old).await.unwrap();
        let next = p.valid_tokens().await.unwrap();
        assert_eq!(next.access_token, "rotated-access");
        assert_eq!(next.refresh_token, replacement.unwrap_or("fixture-refresh"));
        assert!(p.store.load().await.unwrap() == Some(next.clone()));
        assert!(p.valid_tokens().await.unwrap() == next);
        let requests = task.await.unwrap();
        assert!(requests[0].contains("grant_type=refresh_token"));
        assert!(requests[0].contains("refresh_token=fixture-refresh"));
    }
}
#[tokio::test]
async fn oauth_failed_rotation_is_fenced_and_not_replayed() {
    for (status, body) in [
        (401, json!({"error":"invalid_grant"})),
        (
            200,
            json!({"access_token":"rotated","account_id":"different-owner","expires_in":7200}),
        ),
        (200, json!({"access_token":"rotated"})),
    ] {
        let (url, task) = server(vec![(status, body.to_string())]).await;
        let (_dir, p) = local(&url);
        let mut old = tokens();
        old.expires_at = 1;
        p.replace(old).await.unwrap();
        assert!(p.valid_tokens().await.is_err());
        task.await.unwrap();
        // A durable fence prevents replay, even though the original record remains.
        assert!(storage::read_tokens(&p.store.path).is_err());
        p.replace(tokens()).await.unwrap();
        assert!(p.valid_tokens().await.unwrap() == tokens());
    }
}
#[tokio::test]
async fn oauth_device_start_and_poll_classify_responses_without_saving_tokens() {
    for (status, body) in [
        (400, json!({"error":"authorization_pending"})),
        (400, json!({"error":"slow_down"})),
        (400, json!({"error":"access_denied"})),
        (400, json!({"error":"expired_token"})),
        (403, json!({})),
        (404, json!({})),
        (500, json!({})),
        (302, json!({})),
    ] {
        let (url, task) = server(vec![
            (
                200,
                json!({"device_auth_id":"device-fixture","user_code":"USER-CODE","interval":"2"})
                    .to_string(),
            ),
            (status, body.to_string()),
        ])
        .await;
        let (_dir, p) = local(&url);
        let auth = p.begin_device().await.unwrap();
        assert_eq!(auth.interval, 2);
        assert!(p.poll_device_grant(&auth).await.is_err());
        assert!(p.store.load().await.unwrap().is_none());
        let requests = task.await.unwrap();
        assert!(requests[0].contains(CLIENT_ID));
        assert!(requests[1].contains("device-fixture"));
    }
    let (url,task) = server(vec![(200,json!({"authorization_code":"code","code_verifier":"verifier"}).to_string()),(200,json!({"access_token":"new-access","refresh_token":"new-refresh","account_id":"fixture-owner","expires_in":3600}).to_string())]).await;
    let (_dir, p) = local(&url);
    let auth: DeviceAuthorization =
        serde_json::from_value(json!({"device_auth_id":"device","user_code":"USER","interval":1}))
            .unwrap();
    let grant = p.poll_device_grant(&auth).await.unwrap();
    let new = p.exchange_device_grant(grant).await.unwrap();
    assert_eq!(new.access_token, "new-access");
    assert!(p.store.load().await.unwrap().is_none());
    let requests = task.await.unwrap();
    assert!(requests[1].contains("code_verifier=verifier"));
}
#[tokio::test]
async fn oauth_auth_json_and_catalog_errors_are_sanitized() {
    for (status, body) in [
        (200, "not-json".to_owned()),
        (200, json!({}).to_string()),
        (401, "fixture-access".into()),
        (302, "redirect".into()),
    ] {
        let (url, task) = server(vec![(status, body)]).await;
        let (_dir, p) = local(&url);
        let error = p.begin_device().await.err().unwrap();
        assert!(!error.to_string().contains("fixture-access"));
        task.await.unwrap();
    }
    for value in [
        json!({}),
        json!({"models":[{}]}),
        json!({"models":[{"slug":"fixture-access"}]}),
        json!({"models":[{"slug":"a","supported_reasoning_levels":"bad"}]}),
        json!({"models":[{"slug":"a","is_default":"bad"}]}),
        json!({"models":[{"slug":"a","service_tiers":[{}]}]}),
    ] {
        let (url, task) = server(vec![(200, value.to_string())]).await;
        let (_dir, p) = local(&url);
        p.replace(tokens()).await.unwrap();
        assert!(p.models().await.is_err());
        task.await.unwrap();
    }
}
#[tokio::test]
async fn browser_pkce_roundtrip_uses_local_callback_and_token_exchange() {
    let _guard = BROWSER_CALLBACK_LOCK.lock().await;
    let (url,task) = server(vec![(200,json!({"access_token":"browser-access","refresh_token":"browser-refresh","account_id":"fixture-owner","expires_in":3600}).to_string())]).await;
    let (_dir, p) = local(&url);
    let tokens = p
        .login_browser(Duration::from_secs(5), |authorize| {
            let url = reqwest::Url::parse(authorize).unwrap();
            let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
            let mut callback = reqwest::Url::parse(&params["redirect_uri"]).unwrap();
            callback.set_host(Some("127.0.0.1")).unwrap();
            callback
                .query_pairs_mut()
                .append_pair("state", &params["state"])
                .append_pair("code", "browser-code");
            tokio::spawn(async move {
                let reply = reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .get(callback)
                    .send()
                    .await
                    .unwrap();
                assert!(reply.status().is_success());
            });
        })
        .await
        .unwrap();
    assert_eq!(tokens.access_token, "browser-access");
    assert!(p.store.load().await.unwrap() == Some(tokens));
    let requests = task.await.unwrap();
    assert!(requests[0].contains("code=browser-code"));
}

#[tokio::test]
async fn callback_parser_rejects_bad_wire_and_decodes_query_bytes() {
    async fn parse(bytes: Vec<u8>) -> Result<BTreeMap<String, String>, ProviderError> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let writer = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream.write_all(&bytes).await.unwrap();
        });
        let (mut stream, _) = listener.accept().await.unwrap();
        let result = read_callback(&mut stream, Duration::from_secs(1)).await;
        writer.await.unwrap();
        result
    }
    for bytes in [
        b"POST /auth/callback HTTP/1.1\r\n\r\n".to_vec(),
        b"GET /wrong?state=x HTTP/1.1\r\n\r\n".to_vec(),
        b"GET /auth/callback?state=% HTTP/1.1\r\n\r\n".to_vec(),
        b"GET /auth/callback?state=%GG HTTP/1.1\r\n\r\n".to_vec(),
        b"GET /auth/callback?state=%FF HTTP/1.1\r\n\r\n".to_vec(),
        vec![255],
    ] {
        assert!(parse(bytes).await.is_err());
    }
    let query = parse(
        b"GET /auth/callback?state=hello+world&code=a%2Fb&bare&&utf=%C3%A9 HTTP/1.1\r\n\r\n"
            .to_vec(),
    )
    .await
    .unwrap();
    assert_eq!(query["state"], "hello world");
    assert_eq!(query["code"], "a/b");
    assert_eq!(query["bare"], "");
    assert_eq!(query["utf"], "é");
    assert!(
        parse(b"GET /auth/callback HTTP/1.1\r\n\r\n".to_vec())
            .await
            .unwrap()
            .is_empty()
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _peer = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut stream, _) = listener.accept().await.unwrap();
    assert!(matches!(
        read_callback(&mut stream, Duration::from_millis(10)).await,
        Err(ProviderError::Timeout(_))
    ));
}

#[tokio::test]
async fn token_store_missing_corrupt_null_and_external_replacement_are_observed() {
    let (dir, p) = local("http://127.0.0.1:9");
    assert!(p.store.load().await.unwrap().is_none());
    assert!(!p.status().await.authenticated);
    p.store.clear().await.unwrap();
    p.store.save(&tokens()).await.unwrap();
    assert!(p.status().await.refreshable);
    let path = dir.path().join("nested/tokens.json");
    for bytes in [b"{".as_slice(), b"[]", b"{}", b"\"not tokens\""] {
        storage::write(&path, Some(bytes)).unwrap();
        assert!(p.store.load().await.is_err());
        assert!(!p.status().await.authenticated);
    }
    storage::write(&path, Some(b"null")).unwrap();
    assert!(p.store.load().await.unwrap().is_none());
    let mut invalid = tokens();
    invalid.account_id.clear();
    assert!(p.store.save(&invalid).await.is_err());
    p.store.save(&tokens()).await.unwrap();
    assert!(p.status().await.authenticated);
    storage::write(&path, None).unwrap();
    assert!(!p.status().await.authenticated);
    assert!(p.valid_tokens().await.is_err());
}

#[tokio::test]
async fn browser_login_rejects_wrong_state_and_missing_code_without_exchange() {
    let _guard = BROWSER_CALLBACK_LOCK.lock().await;
    for query in [
        vec![("state", "wrong"), ("code", "code")],
        vec![("error", "access_denied")],
        vec![],
    ] {
        let (_dir, p) = local("http://127.0.0.1:9");
        let result = p
            .login_browser(Duration::from_secs(2), move |authorize| {
                let url = reqwest::Url::parse(authorize).unwrap();
                let params: std::collections::HashMap<_, _> =
                    url.query_pairs().into_owned().collect();
                let mut callback = reqwest::Url::parse(&params["redirect_uri"]).unwrap();
                callback.set_host(Some("127.0.0.1")).unwrap();
                if query.is_empty() {
                    callback
                        .query_pairs_mut()
                        .append_pair("state", &params["state"]);
                }
                for (key, value) in query {
                    callback.query_pairs_mut().append_pair(key, value);
                }
                tokio::spawn(async move {
                    let _ = reqwest::Client::builder()
                        .no_proxy()
                        .build()
                        .unwrap()
                        .get(callback)
                        .send()
                        .await;
                });
            })
            .await;
        assert!(result.is_err());
        assert!(p.store.load().await.unwrap().is_none());
    }
    let (_dir, p) = local("http://127.0.0.1:9");
    assert!(matches!(
        p.login_browser(Duration::from_millis(10), |_| {}).await,
        Err(ProviderError::Timeout(_))
    ));
}

#[tokio::test]
async fn explicit_codex_import_checks_overwrite_and_never_changes_source() {
    let (dir, p) = local("http://127.0.0.1:9");
    let path = dir.path().join("import/auth.json");
    let raw = json!({"tokens":{"access_token":"import-access","refresh_token":"import-refresh","id_token":identity()}}).to_string();
    storage::write(&path, Some(raw.as_bytes())).unwrap();
    let imported = p.store.import_codex(&path, false).await.unwrap();
    assert_eq!(imported.account_id, "fixture-owner");
    assert!(p.store.status().await.unwrap().authenticated);
    assert!(p.store.import_codex(&path, false).await.is_err());
    assert!(p.store.import_codex(&path, true).await.is_ok());
    assert_eq!(storage::read(&path).unwrap().unwrap(), raw.as_bytes());
    for value in [
        json!({}),
        json!({"access_token":"a"}),
        json!({"access_token":"a","refresh_token":"r"}),
        json!({"tokens":{"access_token":"a","refresh_token":"r","account_id":""}}),
    ] {
        storage::write(&path, Some(value.to_string().as_bytes())).unwrap();
        assert!(TokenStore::read_import_tokens(&path).await.is_err());
        assert_eq!(
            p.store.load().await.unwrap().unwrap().access_token,
            "import-access"
        );
    }
}

#[tokio::test]
async fn account_bound_store_reauthenticates_rotates_and_logs_out_locally() {
    use voyage_protocol::accounts::Transport;
    let dir = tempfile::tempdir().unwrap();
    let registry = crate::accounts::Registry::new(dir.path().join("registry"));
    let connection = registry
        .add_connection(
            "Fixture OAuth".into(),
            "http://127.0.0.1:9".into(),
            vec![Transport::ChatgptOauth],
        )
        .unwrap();
    let account = registry
        .add_oauth(connection.id, "oauth".into(), "OAuth".into(), tokens())
        .unwrap();
    let binding = registry
        .freeze(account.id, Transport::ChatgptOauth)
        .unwrap();
    let store = TokenStore::bound(registry.clone(), binding.clone());
    assert!(store.load().await.unwrap() == Some(tokens()));
    store.save(&tokens()).await.unwrap();
    let mut rotated = tokens();
    rotated.access_token = "bound-rotation".into();
    let fence = registry.refresh_begin(&binding, &tokens()).unwrap();
    assert!(registry.refresh_begin(&binding, &tokens()).is_err());
    assert!(
        registry
            .oauth_save(&binding, &rotated, Some(uuid::Uuid::new_v4()))
            .is_err()
    );
    registry
        .oauth_save(&binding, &rotated, Some(fence))
        .unwrap();
    assert_eq!(
        store.load().await.unwrap().unwrap().access_token,
        "bound-rotation"
    );
    let updated = registry
        .reauthenticate_oauth(account.id, binding.identity_generation, tokens(), false)
        .unwrap();
    assert_eq!(updated.identity_generation, binding.identity_generation);
    let mut different = tokens();
    different.id_token = None;
    assert!(
        registry
            .reauthenticate_oauth(
                account.id,
                binding.identity_generation,
                different.clone(),
                false
            )
            .is_err()
    );
    let replacement = registry
        .reauthenticate_oauth(account.id, binding.identity_generation, different, true)
        .unwrap();
    assert!(replacement.identity_generation > binding.identity_generation);
    assert!(store.load().await.is_err());
    let current = registry
        .freeze(account.id, Transport::ChatgptOauth)
        .unwrap();
    let store = TokenStore::bound(registry.clone(), current);
    store.clear().await.unwrap();
    assert!(store.load().await.is_err());
    assert!(
        registry
            .freeze(account.id, Transport::ChatgptOauth)
            .is_err()
    );
}
