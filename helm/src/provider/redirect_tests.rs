use super::*;
use crate::model::{Message, Role};
use axum::{
    Router,
    body::Body,
    http::{Request, Response, StatusCode},
    routing::any,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const KEY: &str = "synthetic-redirect-key-canary";
const BODY: &str = "untrusted-redirect-body-canary";

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(status: u16, location: Option<String>, hold: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let app = Router::new().fallback(any(move |request: Request<Body>| {
            let seen = seen.clone();
            let location = location.clone();
            async move {
                seen.lock().unwrap().push((
                    request.uri().to_string(),
                    request
                        .headers()
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string(),
                ));
                if hold {
                    std::future::pending::<()>().await;
                }
                let mut response =
                    Response::builder().status(StatusCode::from_u16(status).unwrap());
                if let Some(location) = location {
                    response = response.header("location", location);
                }
                response.body(Body::from(format!("{KEY} {BODY}"))).unwrap()
            }
        }));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url,
            requests,
            task,
        }
    }
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "claude-fixture".into(),
        messages: vec![Message::new(Role::User, "canonical request")],
        tools: vec![],
        temperature: None,
        max_tokens: Some(8),
    }
}
async fn call(
    provider: &dyn Provider,
    path: usize,
    request: ModelRequest,
) -> Result<(), ProviderError> {
    match path {
        0 => provider.models().await.map(|_| ()),
        1 => provider.complete(request).await.map(|_| ()),
        _ => provider.stream(request).await.map(|_| ()),
    }
}
fn check_error(result: Result<(), ProviderError>) {
    let error = result.unwrap_err();
    assert!(matches!(error, ProviderError::Request(_)));
    let text = error.to_string();
    assert!(text.contains("redirect refused"));
    assert!(text.contains("final endpoint"));
    assert!(!text.contains(KEY));
    assert!(!text.contains(BODY));
    assert!(!text.contains("destination-canary"));
}

#[tokio::test]
async fn anthropic_redirect_matrix_preserves_credentials_and_request_state() {
    let destination = Server::new(200, None, false).await;
    for status in [301, 302, 303, 307, 308] {
        for location in [
            Some(format!("{}/destination-canary?key={KEY}", destination.url)),
            Some("/messages".into()),
            Some("http://[invalid".into()),
            None,
        ] {
            let source = Server::new(status, location, false).await;
            let provider = AnthropicProvider::new(KEY.into(), Some(source.url.clone()));
            let request = request();
            let original = serde_json::to_value(&request).unwrap();
            for path in 0..3 {
                check_error(
                    tokio::time::timeout(
                        Duration::from_secs(3),
                        call(&provider, path, request.clone()),
                    )
                    .await
                    .unwrap(),
                );
                assert_eq!(serde_json::to_value(&request).unwrap(), original);
            }
            let seen = source.requests.lock().unwrap();
            assert_eq!(seen.len(), 3, "same-origin redirects/loops must not replay");
            assert!(seen.iter().all(|(_, key)| key == KEY));
        }
    }
    assert!(destination.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn anthropic_pending_requests_remain_cancellable() {
    let source = Server::new(307, Some("/messages".into()), true).await;
    let provider = AnthropicProvider::new(KEY.into(), Some(source.url.clone()));
    for path in 0..3 {
        let pending = call(&provider, path, request());
        tokio::pin!(pending);
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::select! {
                _ = &mut pending => panic!("held request unexpectedly completed"),
                _ = async {
                    while source.requests.lock().unwrap().len() <= path {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                } => {}
            }
        })
        .await
        .unwrap();
        // Dropping the request future is the provider-neutral cancellation path.
        assert!(
            tokio::time::timeout(Duration::from_millis(1), pending)
                .await
                .is_err()
        );
    }
    assert_eq!(source.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn oauth_redirects_do_not_replay_or_replace_credentials() {
    use super::chatgpt_oauth::{ChatGptOAuth, OAuthTokens, TokenStore};
    let destination = Server::new(200, None, false).await;
    let source = Server::new(
        307,
        Some(format!("{}/destination-canary", destination.url)),
        false,
    )
    .await;
    let directory = tempfile::tempdir().unwrap();
    let store = TokenStore::new(directory.path().join("tokens.json"));
    let mut tokens = OAuthTokens {
        access_token: KEY.into(),
        refresh_token: "synthetic-refresh-canary".into(),
        id_token: None,
        expires_at: u64::MAX,
        account_id: "synthetic-account-canary".into(),
    };
    store.save(&tokens).await.unwrap();
    let endpoints = OAuthEndpoints {
        authorize: source.url.clone(),
        token: source.url.clone(),
        device_user_code: source.url.clone(),
        device_token: source.url.clone(),
        responses: source.url.clone(),
        models: source.url.clone(),
    };
    let provider = ChatGptOAuth::new(store.clone(), endpoints.clone())
        .await
        .unwrap();
    for path in 0..3 {
        check_error(call(&provider, path, request()).await);
    }
    check_error(provider.begin_device().await.map(|_| ()));
    check_error(
        provider
            .poll_device(&DeviceAuthorization {
                device_auth_id: "synthetic-device-canary".into(),
                user_code: "code-canary".into(),
                verification_uri: source.url.clone(),
                interval: 1,
            })
            .await
            .map(|_| ()),
    );
    let flow = provider.begin_pkce("http://127.0.0.1/callback");
    check_error(
        provider
            .exchange_code("authorization-code-canary", &flow)
            .await
            .map(|_| ()),
    );
    assert_eq!(store.load().await.unwrap(), Some(tokens.clone()));
    tokens.expires_at = 1;
    store.save(&tokens).await.unwrap();
    let provider = ChatGptOAuth::new(store.clone(), endpoints.clone())
        .await
        .unwrap();
    check_error(provider.models().await.map(|_| ()));
    assert_eq!(store.load().await.unwrap(), Some(tokens));
    let provider = ChatGptOAuth::from_store(store, endpoints);
    check_error(provider.begin_device().await.map(|_| ()));
    assert_eq!(source.requests.lock().unwrap().len(), 8);
    assert!(destination.requests.lock().unwrap().is_empty());
}

// This real TLS fixture uses Python and OpenSSL, both provided by Linux CI.
// Other platform runs retain the portable HTTP and OAuth regression matrix.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn anthropic_tls_downgrade_never_reaches_cleartext_destination() {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = tokio::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tests/fixtures/provider_redirect_tls.py"
        ))
        .arg(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut ready = String::new();
    tokio::time::timeout(
        Duration::from_secs(15),
        BufReader::new(fixture.stdout.take().unwrap()).read_line(&mut ready),
    )
    .await
    .expect("TLS fixture readiness timed out")
    .unwrap();
    let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
    let mut provider =
        AnthropicProvider::new(KEY.into(), Some(ready["url"].as_str().unwrap().into()));
    // Trust only this generated certificate while using the production policy.
    provider.client = native_http_client_builder()
        .add_root_certificate(
            reqwest::Certificate::from_pem(
                &std::fs::read(directory.path().join("cert.pem")).unwrap(),
            )
            .unwrap(),
        )
        .build()
        .unwrap();
    for path in 0..3 {
        check_error(
            tokio::time::timeout(Duration::from_secs(5), call(&provider, path, request()))
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        std::fs::read_to_string(directory.path().join("source-requests"))
            .unwrap()
            .lines()
            .count(),
        3
    );
    assert!(!directory.path().join("destination-contacted").exists());
    fixture.kill().await.unwrap();
}

#[tokio::test]
async fn redirect_errors_do_not_wait_for_untrusted_response_bodies() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(any(|| async {
        Response::builder()
            .status(307)
            .header("location", "/loop")
            .body(Body::from_stream(futures_util::stream::pending::<
                Result<bytes::Bytes, std::convert::Infallible>,
            >()))
            .unwrap()
    }));
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let fixture = Server {
        url,
        requests: Arc::default(),
        task,
    };
    let provider = AnthropicProvider::new(KEY.into(), Some(fixture.url.clone()));
    for path in 0..3 {
        check_error(
            tokio::time::timeout(Duration::from_secs(2), call(&provider, path, request()))
                .await
                .unwrap(),
        );
    }
}
