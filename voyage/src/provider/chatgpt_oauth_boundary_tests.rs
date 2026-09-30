//! Bound-account request/refresh admission against private stores and numeric
//! loopback scripted OAuth. No operator credentials or live-provider effects.
use super::*;
use crate::provider::native_http_tests::server;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use voyage_protocol::accounts::{AccountBinding, Transport};

fn identity() -> String {
    format!(
        "fixture.{}.unsigned",
        URL_SAFE_NO_PAD
            .encode(json!({"sub":"fixture-user", "account_id":"fixture-owner"}).to_string())
    )
}
fn tokens(expired: bool) -> OAuthTokens {
    OAuthTokens {
        access_token: "fixture-access".into(),
        refresh_token: "fixture-refresh".into(),
        id_token: Some(identity()),
        account_id: "fixture-owner".into(),
        expires_at: if expired { 1 } else { u64::MAX / 2 },
    }
}
fn fixture(
    expired: bool,
) -> (
    tempfile::TempDir,
    crate::accounts::Registry,
    AccountBinding,
    TokenStore,
) {
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
        .add_oauth(
            connection.id,
            "fixture".into(),
            "Fixture".into(),
            tokens(expired),
        )
        .unwrap();
    let binding = registry
        .freeze(account.id, Transport::ChatgptOauth)
        .unwrap();
    let store = TokenStore::bound(registry.clone(), binding.clone());
    (dir, registry, binding, store)
}
fn endpoints(url: &str) -> OAuthEndpoints {
    OAuthEndpoints {
        token: format!("{url}/token"),
        responses: format!("{url}/responses"),
        models: format!("{url}/models"),
        ..Default::default()
    }
}
fn rotation() -> Value {
    json!({"access_token":"fixture-rotated-access", "refresh_token":"fixture-rotated-refresh", "id_token":identity(), "account_id":"fixture-owner", "expires_in":7200})
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "fixture-model".into(),
        messages: vec![crate::model::Message::new(
            crate::model::Role::User,
            "hello",
        )],
        tools: vec![],
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: None,
    }
}
fn answer() -> Value {
    json!({"status":"completed", "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}]})
}
#[derive(Debug)]
struct RevokeAt {
    checks: AtomicUsize,
    denied_check: usize,
}
impl crate::policy::ExecutionAuthority for RevokeAt {
    fn check(&self) -> anyhow::Result<()> {
        let check = self.checks.fetch_add(1, Ordering::SeqCst) + 1;
        anyhow::ensure!(check < self.denied_check, "PRIVATE authority diagnostic");
        Ok(())
    }
}
#[tokio::test]
async fn bound_refresh_is_persisted_before_request_and_redactor_tracks_rotated_identity() {
    let (url, task) = server(vec![
        (200, rotation().to_string()),
        (200, answer().to_string()),
        (200, answer().to_string()),
    ])
    .await;
    let (_dir, registry, binding, store) = fixture(true);
    let redactor = Arc::new(crate::tools::Redactor::new([]));
    let provider = ChatGptOAuth::from_store(store.clone(), endpoints(&url))
        .with_redactor(Some(redactor.clone()));
    assert_eq!(
        provider.complete(request()).await.unwrap().message.content,
        "answer"
    );
    assert_eq!(
        provider.complete(request()).await.unwrap().message.content,
        "answer"
    );
    let durable = registry.oauth_load(&binding).unwrap();
    assert_eq!(durable.access_token, "fixture-rotated-access");
    assert_eq!(durable.refresh_token, "fixture-rotated-refresh");
    assert_eq!(
        store.status().await.unwrap().account_id.as_deref(),
        Some("fixture-owner")
    );
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with("POST /token "));
    assert!(requests[0].contains("refresh_token=fixture-refresh"));
    for raw in &requests[1..] {
        assert!(raw.starts_with("POST /responses "));
        let lower = raw.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer fixture-rotated-access"));
        assert!(lower.contains("chatgpt-account-id: fixture-owner"));
        assert!(!raw.contains("fixture-refresh"));
    }
    for secret in [
        "fixture-access",
        "fixture-refresh",
        "fixture-rotated-access",
        "fixture-rotated-refresh",
        "fixture-owner",
    ] {
        assert!(!redactor.redact(secret).contains(secret));
    }
    let reopened = ChatGptOAuth::new(store, endpoints(&url)).await.unwrap();
    assert!(reopened.valid_tokens().await.unwrap() == durable);
}
#[tokio::test]
async fn foreground_revocation_before_rotation_does_not_send_or_change_credential() {
    for denied_check in [1, 2] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (_dir, registry, binding, store) = fixture(true);
        let authority = Arc::new(RevokeAt {
            checks: AtomicUsize::new(0),
            denied_check,
        });
        let provider = ChatGptOAuth::from_store(store, endpoints(&endpoint))
            .with_authority(Some(authority.clone()));
        let error = provider.valid_tokens().await.err().unwrap();
        assert_eq!(error.category(), "authentication");
        assert!(!format!("{error:?}").contains("PRIVATE"));
        assert_eq!(authority.checks.load(Ordering::SeqCst), denied_check);
        assert!(registry.oauth_load(&binding).unwrap() == tokens(true));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn revocation_after_refresh_claim_retains_fence_without_dispatch_or_replay() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (_dir, registry, binding, store) = fixture(true);
    let authority = Arc::new(RevokeAt {
        checks: AtomicUsize::new(0),
        denied_check: 3,
    });
    let provider =
        ChatGptOAuth::from_store(store, endpoints(&endpoint)).with_authority(Some(authority));
    assert_eq!(
        provider.valid_tokens().await.err().unwrap().category(),
        "authentication"
    );
    let error = registry.oauth_load(&binding).err().unwrap();
    assert!(error.downcast_ref::<RefreshPending>().is_some());
    assert!(registry.refresh_begin(&binding, &tokens(true)).is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn revocation_after_committed_rotation_preserves_effect_and_next_provider_reuses_it() {
    let (url, task) = server(vec![(200, rotation().to_string())]).await;
    let (_dir, registry, binding, store) = fixture(true);
    let authority = Arc::new(RevokeAt {
        checks: AtomicUsize::new(0),
        denied_check: 4,
    });
    let provider =
        ChatGptOAuth::from_store(store.clone(), endpoints(&url)).with_authority(Some(authority));
    assert_eq!(
        provider.valid_tokens().await.err().unwrap().category(),
        "authentication"
    );
    assert_eq!(task.await.unwrap().len(), 1);
    let durable = registry.oauth_load(&binding).unwrap();
    assert_eq!(durable.access_token, "fixture-rotated-access");
    let next = ChatGptOAuth::new(store, endpoints(&url)).await.unwrap();
    assert!(next.valid_tokens().await.unwrap() == durable);
}
#[tokio::test]
async fn bound_expiry_rejection_refreshes_once_without_automatic_inference_replay() {
    let (url, task) = server(vec![
        (401, json!({"error":{"code":"token_expired"}}).to_string()),
        (200, rotation().to_string()),
        (200, answer().to_string()),
    ])
    .await;
    let (_dir, registry, binding, store) = fixture(false);
    let provider = ChatGptOAuth::from_store(store, endpoints(&url));
    let error = provider.complete(request()).await.err().unwrap();
    assert_eq!(error.category(), "authentication");
    assert!(error.is_retryable());
    assert_eq!(error.http_status(), Some(401));
    assert_eq!(
        registry.oauth_load(&binding).unwrap().access_token,
        "fixture-rotated-access"
    );
    assert_eq!(
        provider.complete(request()).await.unwrap().message.content,
        "answer"
    );
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].contains("Bearer fixture-access"));
    assert!(requests[1].starts_with("POST /token "));
    assert!(requests[2].contains("Bearer fixture-rotated-access"));
}
#[tokio::test]
async fn unresolved_store_initialization_refuses_read_write_logout_without_private_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let store = TokenStore {
        path: dir.path().join("must-not-publish.json"),
        binding: None,
        legacy_default: false,
        initialization_error: true,
    };
    for error in [
        store.load().await.err().unwrap(),
        store.save(&tokens(false)).await.err().unwrap(),
        store.clear().await.err().unwrap(),
    ] {
        assert_eq!(error.category(), "authentication");
        assert!(!error.to_string().contains("must-not-publish"));
    }
    assert!(!store.path.exists());
    let provider = ChatGptOAuth::from_store(store, endpoints("http://127.0.0.1:9"));
    assert!(!provider.status().await.authenticated);
    assert!(provider.valid_tokens().await.is_err());
}

const CHILD_ROOT: &str = "VOYAGE_PROVIDER_BOUNDARY_ROOT";
#[test]
fn legacy_store_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let path = TokenStore::default_path().unwrap();
    assert!(path.starts_with(root.join("data")));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let stale = TokenStore::new(path.clone());
        assert!(stale.legacy_default && stale.binding.is_none());
        stale.save(&tokens(false)).await.unwrap();
        let retained_cache = storage::read(&path).unwrap().unwrap();
        let registry = crate::accounts::Registry::default_host().unwrap();
        assert!(registry.migrate_legacy_oauth(false).is_err());
        assert!(stale.load().await.unwrap() == Some(tokens(false)));
        let account_id = registry.migrate_legacy_oauth(true).unwrap().unwrap();
        assert!(stale.load().await.is_err());
        assert!(stale.save(&tokens(false)).await.is_err());
        assert!(stale.clear().await.is_err());
        assert_eq!(storage::read(&path).unwrap().unwrap(), retained_cache);
        let selected = TokenStore::new(path.clone());
        assert_eq!(selected.binding.as_ref().unwrap().1.account_id, account_id);
        assert!(selected.load().await.unwrap() == Some(tokens(false)));
        selected.save(&tokens(false)).await.unwrap();
        selected.clear().await.unwrap();
        assert!(selected.load().await.is_err());
        assert!(TokenStore::new(path.clone()).load().await.is_err());
        assert_eq!(storage::read(&path).unwrap().unwrap(), retained_cache);
        let provider =
            ChatGptOAuth::from_store(TokenStore::new(path), endpoints("http://127.0.0.1:9"));
        assert!(!provider.status().await.authenticated);
        assert!(provider.valid_tokens().await.is_err());
        std::fs::write(
            root.join("data/helm/accounts/registry.json"),
            b"corrupt private registry",
        )
        .unwrap();
        let damaged = TokenStore::new(TokenStore::default_path().unwrap());
        assert!(damaged.initialization_error);
        assert!(damaged.load().await.is_err());
        assert!(damaged.save(&tokens(false)).await.is_err());
        assert!(damaged.clear().await.is_err());
        assert_eq!(
            storage::read(&damaged.path).unwrap().unwrap(),
            retained_cache
        );
        std::fs::write(root.join("completed"), "migration-and-logout-fenced").unwrap();
    });
}
#[test]
fn migrating_legacy_credentials_fences_existing_store_and_logout_does_not_resurrect_cache() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "provider::chatgpt_oauth::boundary_tests::legacy_store_child",
        ])
        .env(CHILD_ROOT, &root)
        .env("HOME", &root)
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "private legacy-store child failed");
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("private legacy-store child exceeded bounded timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(root.join("completed")).unwrap(),
        "migration-and-logout-fenced"
    );
}
