use super::*;
use axum::{
    Router,
    body::Body,
    http::{Request, Response},
    routing::any,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
const KEY: &str = "synthetic-catalog-key";
struct Fixture {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new(pages: Vec<(u16, Vec<u8>)>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let pages = Arc::new(Mutex::new(VecDeque::from(pages)));
        let app = Router::new().fallback(any(move |request: Request<Body>| {
            let pages = pages.clone();
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(request.uri().to_string());
                let mut pages = pages.lock().unwrap();
                let (code, body) = if pages.len() > 1 {
                    pages.pop_front().unwrap()
                } else {
                    pages.front().unwrap().clone()
                };
                Response::builder()
                    .status(code)
                    .header("retry-after", "7")
                    .body(Body::from(body))
                    .unwrap()
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
    async fn json(value: Value) -> Self {
        Self::new(vec![(200, serde_json::to_vec(&value).unwrap())]).await
    }
    fn anthropic(&self) -> AnthropicProvider {
        AnthropicProvider::new(KEY.into(), Some(self.url.clone()))
    }
}
fn safe_error(error: ProviderError) {
    let text = error.to_string();
    assert!(!text.contains(KEY));
    assert!(!text.contains("secret-response-canary"));
    assert!(!text.contains('\x1b'));
    assert!(text.len() < 200);
}
#[tokio::test]
async fn native_catalogs_reject_unsafe_ids_and_body_errors() {
    for value in [
        json!({"data":[{"id":KEY}]}),
        json!({"data":[{"id":"bad\u{202e}"}]}),
        json!({"data":[{"id":"x".repeat(513)}]}),
        json!({"data":[{"id":4}]}),
        json!({"data":[]}),
    ] {
        // Empty catalog itself is valid; all other fixtures reject before display.
        let fixture = Fixture::json(value.clone()).await;
        let providers: Vec<Box<dyn Provider>> = vec![
            Box::new(fixture.anthropic()),
            Box::new(OpenAiProvider::new(KEY.into(), Some(fixture.url.clone()))),
            Box::new(OpenAiResponsesProvider::new(
                KEY.into(),
                Some(fixture.url.clone()),
            )),
        ];
        for provider in providers {
            if value["data"].as_array().is_some_and(Vec::is_empty) {
                assert!(provider.models().await.unwrap().is_empty());
            } else {
                safe_error(provider.models().await.unwrap_err());
            }
        }
    }
    for (code, body) in [
        (401, KEY.as_bytes().to_vec()),
        (403, KEY.as_bytes().to_vec()),
        (429, KEY.as_bytes().to_vec()),
        (500, KEY.as_bytes().to_vec()),
        (200, format!("invalid {KEY}").into_bytes()),
        (200, vec![b'x'; catalog::MAX_BYTES + 1]),
    ] {
        let fixture = Fixture::new(vec![(code, body)]).await;
        let error = fixture.anthropic().models().await.unwrap_err();
        match code {
            401 | 403 => assert!(matches!(error, ProviderError::Authentication(_))),
            429 => assert_eq!(error.retry_after(), Some(Duration::from_secs(7))),
            500 => assert!(error.is_retryable()),
            _ => assert!(matches!(error, ProviderError::InvalidResponse(_))),
        }
        safe_error(error);
    }
}
#[tokio::test]
async fn anthropic_catalog_pagination_is_bounded_and_preserves_unicode() {
    let page = |value| (200, serde_json::to_vec(&value).unwrap());
    let fixture = Fixture::new(vec![
        page(json!({"data":[{"id":"first"}],"has_more":true,"last_id":"cursor/α"})),
        page(json!({"data":[{"id":"模型","display_name":"Français 日本語 🚢"}],"has_more":false})),
    ])
    .await;
    let models = fixture.anthropic().models().await.unwrap();
    assert_eq!(models.len(), 2);
    assert!(
        models
            .iter()
            .any(|m| m.display_name == "Français 日本語 🚢")
    );
    assert!(fixture.requests.lock().unwrap()[1].contains("after_id=cursor%2F%CE%B1"));
    for cursor in [
        json!(KEY),
        json!("bad\u{2066}"),
        json!("x".repeat(513)),
        Value::Null,
        json!(4),
    ] {
        let fixture = Fixture::json(json!({"data":[],"has_more":true,"last_id":cursor})).await;
        safe_error(fixture.anthropic().models().await.unwrap_err());
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    }
    let fixture = Fixture::json(json!({"data":[],"has_more":true,"last_id":"same"})).await;
    safe_error(fixture.anthropic().models().await.unwrap_err());
    assert_eq!(fixture.requests.lock().unwrap().len(), 2);
    let fixture = Fixture::new(
        (0..17)
            .map(|index| {
                page(json!({"data":[],"has_more":true,"last_id":format!("cursor{index}")}))
            })
            .collect(),
    )
    .await;
    safe_error(fixture.anthropic().models().await.unwrap_err());
    assert_eq!(fixture.requests.lock().unwrap().len(), 16);
    let fixture=Fixture::new(vec![page(json!({"data":[],"padding":"x".repeat(catalog::MAX_BYTES/2),"has_more":true,"last_id":"first"})),page(json!({"data":[],"padding":"x".repeat(catalog::MAX_BYTES/2)}))]).await;
    safe_error(fixture.anthropic().models().await.unwrap_err());
}
#[tokio::test]
async fn oauth_catalog_rejects_all_token_echoes_and_metadata_fields() {
    use super::chatgpt_oauth::{ChatGptOAuth, OAuthTokens, TokenStore};
    let directory = tempfile::tempdir().unwrap();
    let store = TokenStore::new(directory.path().join("tokens"));
    let tokens = OAuthTokens {
        access_token: KEY.into(),
        refresh_token: "refresh-canary".into(),
        id_token: Some("identity-canary".into()),
        expires_at: u64::MAX,
        account_id: "account-canary".into(),
    };
    store.save(&tokens).await.unwrap();
    for bad in [
        KEY,
        "refresh-canary",
        "identity-canary",
        "account-canary",
        "bad\x1b",
        "bad\u{202e}",
    ] {
        for field in [
            "slug",
            "display_name",
            "description",
            "supported_reasoning_levels",
        ] {
            let mut entry = json!({"slug":"good","display_name":"Good"});
            entry[field] = if field == "supported_reasoning_levels" {
                json!([{"effort":bad}])
            } else {
                json!(bad)
            };
            let fixture = Fixture::json(json!({"models":[entry]})).await;
            let provider = ChatGptOAuth::new(
                store.clone(),
                OAuthEndpoints {
                    models: fixture.url.clone(),
                    ..OAuthEndpoints::default()
                },
            )
            .await
            .unwrap();
            let error = provider.models().await.unwrap_err();
            assert!(!error.to_string().contains(bad));
            safe_error(error);
        }
    }
    assert_eq!(store.load().await.unwrap(), Some(tokens));
}

#[tokio::test]
async fn native_catalog_optional_fields_reject_invalid_types_and_bounds() {
    for entry in [
        json!({"id":"good","display_name":7}),
        json!({"id":"good","display_name":"x".repeat(513)}),
    ] {
        let fixture = Fixture::json(json!({"data":[entry]})).await;
        safe_error(fixture.anthropic().models().await.unwrap_err());
    }
    for flag in [json!("true"), json!(3)] {
        let fixture = Fixture::json(json!({"data":[],"has_more":flag})).await;
        safe_error(fixture.anthropic().models().await.unwrap_err());
    }
    let fixture = Fixture::json(
        json!({"data":(0..1025).map(|i|json!({"id":format!("model-{i}")})).collect::<Vec<_>>()}),
    )
    .await;
    safe_error(fixture.anthropic().models().await.unwrap_err());
}

#[tokio::test]
async fn catalog_chunked_body_and_transport_failure_are_bounded_and_safe() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(any(|| async {
        let chunks = futures_util::stream::iter(vec![Ok::<_, std::convert::Infallible>(
            bytes::Bytes::from(vec![b'x'; catalog::MAX_BYTES + 1]),
        )]);
        Response::builder()
            .status(200)
            .body(Body::from_stream(chunks))
            .unwrap()
    }));
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let fixture = Fixture {
        url,
        requests: Arc::default(),
        task,
    };
    safe_error(
        tokio::time::timeout(Duration::from_secs(2), fixture.anthropic().models())
            .await
            .unwrap()
            .unwrap_err(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/{KEY}", listener.local_addr().unwrap());
    drop(listener);
    let error = AnthropicProvider::new(KEY.into(), Some(url))
        .models()
        .await
        .unwrap_err();
    assert!(matches!(error, ProviderError::Unavailable(_)));
    assert!(error.is_retryable());
    safe_error(error);
}
