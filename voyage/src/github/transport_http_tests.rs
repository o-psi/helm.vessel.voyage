use super::*;
use crate::github::http_fixture::{Fixture, Reply};
use serde_json::json;

#[tokio::test]
async fn authenticated_json_request_preserves_status_headers_and_payload() {
    let mut reply = Reply::post(json!({"created":true}));
    reply.status = 201;
    reply.headers.push(("X-Fixture".into(), "observed".into()));
    let server = Fixture::start(vec![reply]).await;
    let response = server
        .client()
        .request(
            Method::POST,
            "/graphql",
            Some(&json!({"text":"hello"})),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(response.headers["x-fixture"], "observed");
    assert_eq!(response.json().unwrap(), json!({"created":true}));
    let requests = server.finish().await;
    let request = String::from_utf8(requests[0].clone()).unwrap();
    let (headers, body) = request.split_once("\r\n\r\n").unwrap();
    let headers = headers.to_ascii_lowercase();
    for expected in [
        "authorization: bearer offline-fixture-token",
        "accept: application/vnd.github+json",
        "user-agent: helm-github/1",
        "content-type: application/json",
    ] {
        assert!(headers.contains(expected), "{expected}");
    }
    assert!(headers.contains(&format!("x-github-api-version: {API_VERSION}")));
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        json!({"text":"hello"})
    );
}

#[tokio::test]
async fn redirects_and_server_failures_are_returned_without_replay() {
    for status in [302, 429, 503] {
        let mut reply = Reply::json("/user", json!({"private":"diagnostic"}));
        reply.status = status;
        reply
            .headers
            .push(("Location".into(), "http://127.0.0.1:1/never".into()));
        let server = Fixture::start(vec![reply]).await;
        let error = server
            .client()
            .get("/user", &CancellationToken::new())
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("diagnostic"));
        assert!(!error.contains("127.0.0.1"));
        assert!(error.contains(if status == 302 {
            "redirected"
        } else if status == 429 {
            "rate limited"
        } else {
            "HTTP 503"
        }));
        assert_eq!(server.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn invalid_paths_large_bodies_and_cancellation_never_connect() {
    let client = Client::for_test("127.0.0.1:1".parse().unwrap());
    let cancel = CancellationToken::new();
    for path in [
        "https://example.test/",
        "//example.test/",
        "/a/../b",
        "/./b",
        "/a#b",
        "/a\\b",
    ] {
        assert!(
            client
                .get(path, &cancel)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("invalid GitHub operation")
        );
    }
    assert!(
        client
            .request(
                Method::POST,
                "/graphql",
                Some(&json!("x".repeat(MAX_REQUEST_BYTES))),
                &cancel
            )
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("request exceeds limit")
    );
    cancel.cancel();
    assert_eq!(
        client
            .get("/user", &cancel)
            .await
            .err()
            .unwrap()
            .to_string(),
        "GitHub operation cancelled"
    );
}

#[tokio::test]
async fn response_size_limits_cover_declared_and_chunked_bodies() {
    for chunked in [false, true] {
        let mut reply = Reply::json("/user", Value::Null);
        if chunked {
            reply
                .headers
                .push(("Transfer-Encoding".into(), "chunked".into()));
            reply.body = format!(
                "{:x}\r\n{}\r\n0\r\n\r\n",
                MAX_RESPONSE_BYTES + 1,
                "x".repeat(MAX_RESPONSE_BYTES + 1)
            )
            .into_bytes();
        } else {
            reply.headers.push((
                "Content-Length".into(),
                (MAX_RESPONSE_BYTES + 1).to_string(),
            ));
            reply.body.clear();
        }
        let server = Fixture::start(vec![reply]).await;
        assert_eq!(
            server
                .client()
                .get("/user", &CancellationToken::new())
                .await
                .err()
                .unwrap()
                .to_string(),
            "GitHub response exceeds limit"
        );
        server.finish().await;
    }
}

#[tokio::test]
async fn interrupted_response_and_invalid_json_have_sanitized_errors() {
    let mut reply = Reply::json("/user", Value::Null);
    reply.headers.push(("Content-Length".into(), "100".into()));
    let server = Fixture::start(vec![reply]).await;
    let error = server
        .client()
        .get("/user", &CancellationToken::new())
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("interrupted"));
    assert!(!error.contains("offline-fixture-token"));
    server.finish().await;
    let mut reply = Reply::json("/user", Value::Null);
    reply.body = b"private invalid json".to_vec();
    let server = Fixture::start(vec![reply]).await;
    assert_eq!(
        server
            .client()
            .get("/user", &CancellationToken::new())
            .await
            .unwrap()
            .json()
            .unwrap_err()
            .to_string(),
        "GitHub returned invalid JSON"
    );
    server.finish().await;
}
