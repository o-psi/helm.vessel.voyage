use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn reject(body: &str) -> ProviderError {
    reject_with(429, "1", body).await
}

async fn reject_with(status: u16, retry_after: &str, body: &str) -> ProviderError {
    checked_stream_response(http_response(status, retry_after, body, body.len()).await)
        .await
        .unwrap_err()
}

pub(super) async fn http_response(
    status: u16,
    retry_after: &str,
    body: &str,
    length: usize,
) -> reqwest::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nRetry-After: {retry_after}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        length
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let byte = socket.read_u8().await.unwrap();
            request.push(byte);
            assert!(request.len() < 8192);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let response = endpoint_http_client(&native_http_client(), &endpoint)
        .get(endpoint)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .unwrap();
    server.await.unwrap();
    response
}

#[test]
fn retry_dates_seconds_and_overflow_never_shorten_server_waits() {
    use std::time::Duration;
    let date = httpdate::parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap();
    for text in [
        "Sun, 06 Nov 1994 08:49:37 GMT",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
    ] {
        assert_eq!(
            parse_retry_after(text, date - Duration::from_secs(7)),
            Some(Duration::from_secs(7))
        );
        assert_eq!(
            parse_retry_after(text, date + Duration::from_secs(1)),
            Some(Duration::ZERO)
        );
    }
    assert_eq!(
        parse_retry_after(" 12 ", date),
        Some(Duration::from_secs(12))
    );
    assert_eq!(
        parse_retry_after("999999999999999999999999999999999999", date),
        Some(Duration::from_secs(u64::MAX))
    );
    for text in ["", "-1", "+1", "NaN", "1.5", "invalid date"] {
        assert_eq!(parse_retry_after(text, date), None);
    }
}

#[tokio::test]
async fn unavailable_keeps_server_delay_and_safe_failure_category() {
    let error = reject_with(503, "12", "private provider diagnostic").await;
    assert!(error.is_retryable());
    assert_eq!(
        error.retry_after(),
        Some(std::time::Duration::from_secs(12))
    );
    assert_eq!(
        error.public_failure_reason(),
        "Provider temporarily unavailable."
    );
    assert_eq!(error.category(), "unavailable");
    assert_eq!(error.http_status(), Some(503));
}

#[tokio::test]
async fn exhausted_account_is_not_retried_or_exposed_as_raw_diagnostics() {
    for body in [
        r#"{"error":{"type":"usage_limit_reached","message":"secret-provider-text","plan_type":"private-plan"}}"#,
        r#"{"error":{"code":"insufficient_quota","message":"secret-provider-text"}}"#,
    ] {
        let error = reject(body).await;
        assert_eq!(error.category(), "usage_limit");
        assert_eq!(error.http_status(), Some(429));
        assert!(!error.is_retryable());
        assert_eq!(error.to_string(), USAGE_LIMIT_MESSAGE);
        assert_eq!(
            crate::agent::AgentError::Provider(error).public_failure_reason(),
            USAGE_LIMIT_MESSAGE
        );
    }
}

#[tokio::test]
async fn transient_or_unknown_throttling_keeps_bounded_retry_after() {
    for body in [
        r#"{"error":{"type":"rate_limit_exceeded"}}"#,
        r#"{"error":{"type":"unknown_limit"}}"#,
        "not JSON",
    ] {
        let error = reject(body).await;
        assert_eq!(error.category(), "rate_limit");
        assert_eq!(error.http_status(), Some(429));
        assert!(error.is_retryable());
        assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(1)));
    }
}

#[tokio::test]
async fn context_rejections_are_content_free_and_never_transient() {
    for (status, body) in [
        (
            400,
            r#"{"error":{"code":"context_length_exceeded","message":"PRIVATE"}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"invalid_request_error","message":"prompt is too long: 210000 tokens > 200000 maximum PRIVATE"}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"invalid_request_error","message":"This model's maximum context length is 8192 tokens. PRIVATE"}}"#,
        ),
        // Explicit context codes override transient HTTP status and Retry-After.
        (
            429,
            r#"{"error":{"code":"context_length_exceeded","message":"PRIVATE"}}"#,
        ),
        (
            503,
            r#"{"error":{"code":"context_length_exceeded","message":"PRIVATE"}}"#,
        ),
    ] {
        let error = reject_with(status, "12", body).await;
        assert_eq!(error.category(), "context_length");
        assert_eq!(error.http_status(), Some(status));
        assert!(!error.is_retryable());
        assert_eq!(error.retry_after(), None);
        assert_eq!(error.to_string(), CONTEXT_LENGTH_MESSAGE);
        assert_eq!(error.public_failure_reason(), CONTEXT_LENGTH_MESSAGE);
        assert!(!format!("{error:?}").contains("PRIVATE"));
        let redacted = multimodal::redact(error);
        assert_eq!(redacted.category(), "context_length");
        assert_eq!(redacted.http_status(), Some(status));
    }
}

#[tokio::test]
async fn generic_size_errors_and_authentication_are_not_context_rejections() {
    for (status, body) in [
        (
            413,
            r#"{"error":{"type":"request_too_large","message":"request too large"}}"#,
        ),
        (
            400,
            r#"{"error":{"type":"invalid_request_error","message":"max_tokens is too large"}}"#,
        ),
        (400, r#"{"error":{"message":"context length exceeded"}}"#),
        (400, "context_length_exceeded"),
        (401, r#"{"error":{"code":"context_length_exceeded"}}"#),
        (403, r#"{"error":{"code":"context_length_exceeded"}}"#),
        (
            429,
            r#"{"error":{"type":"invalid_request_error","message":"prompt is too long: 210000 tokens > 200000 maximum"}}"#,
        ),
        (
            500,
            r#"{"error":{"type":"invalid_request_error","message":"This model's maximum context length is 8192 tokens."}}"#,
        ),
    ] {
        let error = reject_with(status, "12", body).await;
        assert_ne!(error.category(), "context_length");
        assert_eq!(error.http_status(), Some(status));
    }
    let error =
        reject(r#"{"error":{"type":"usage_limit_reached","code":"context_length_exceeded"}}"#)
            .await;
    assert_eq!(error.category(), "usage_limit");
    assert_eq!(error.http_status(), Some(429));
}

#[test]
fn failure_metadata_is_content_free_and_wrappers_preserve_policy() {
    let delay = std::time::Duration::from_secs(9);
    for (error, category, retryable) in [
        (
            ProviderError::Authentication("PRIVATE".into()),
            "authentication",
            false,
        ),
        (ProviderError::UsageLimit, "usage_limit", false),
        (ProviderError::ContextLength, "context_length", false),
        (
            ProviderError::RateLimit {
                message: "PRIVATE".into(),
                retry_after: Some(delay),
            },
            "rate_limit",
            true,
        ),
        (
            ProviderError::Unavailable("PRIVATE".into()),
            "unavailable",
            true,
        ),
        (ProviderError::Timeout("PRIVATE".into()), "timeout", true),
        (
            ProviderError::Transport("PRIVATE".into()),
            "transport",
            false,
        ),
        (ProviderError::Request("PRIVATE".into()), "request", false),
        (
            ProviderError::InvalidResponse("PRIVATE".into()),
            "invalid_response",
            false,
        ),
        (ProviderError::Incomplete, "incomplete", false),
    ] {
        assert_eq!(error.http_status(), None);
        let reason = error.public_failure_reason();
        let retry_after = error.retry_after();
        let wrapped = error.with_http_status(503);
        assert_eq!(wrapped.is_context_length(), category == "context_length");
        assert_eq!(wrapped.is_incomplete(), category == "incomplete");
        assert_eq!(wrapped.category(), category);
        assert_eq!(wrapped.http_status(), Some(503));
        assert_eq!(wrapped.is_retryable(), retryable);
        assert_eq!(wrapped.retry_after(), retry_after);
        assert_eq!(wrapped.public_failure_reason(), reason);
        assert!(!reason.contains("PRIVATE"));
        let wrapped = ProviderError::RetryAfter {
            source: Box::new(wrapped),
            delay,
        };
        assert_eq!(wrapped.http_status(), Some(503));
        assert_eq!(wrapped.is_context_length(), category == "context_length");
        assert_eq!(wrapped.is_incomplete(), category == "incomplete");
        assert_eq!(wrapped.category(), category);
        assert_eq!(wrapped.is_retryable(), retryable);
        assert_eq!(wrapped.retry_after(), Some(delay));
        let redacted = multimodal::redact(wrapped);
        assert_eq!(redacted.category(), category);
        assert_eq!(redacted.http_status(), Some(503));
        assert!(!format!("{redacted:?}").contains("PRIVATE"));
    }
    assert_eq!(
        ProviderError::Request("PRIVATE".into()).public_failure_reason(),
        "Provider request failed."
    );
}

#[tokio::test]
async fn http_metadata_preserves_authentication_and_unknown_error_classification() {
    for (status, category, retryable) in [
        (401, "authentication", false),
        (403, "authentication", false),
        (302, "request", false),
        (400, "request", false),
        (413, "request", false),
        (429, "rate_limit", true),
        (408, "unavailable", true),
        (409, "unavailable", true),
        (500, "unavailable", true),
    ] {
        let error = reject_with(
            status,
            "3",
            r#"{"error":{"code":"PRIVATE_UNKNOWN_CODE","message":"PRIVATE"}}"#,
        )
        .await;
        assert_eq!(error.category(), category);
        assert_eq!(error.http_status(), Some(status));
        assert_eq!(error.is_retryable(), retryable);
        assert!(!error.public_failure_reason().contains("PRIVATE"));
    }
}

#[tokio::test]
async fn local_builder_failure_is_not_an_uncertain_transport_failure() {
    let error = native_http_client()
        .get("invalid URL PRIVATE")
        .build()
        .unwrap_err();
    assert!(error.is_builder());
    let error = map_transport(error);
    assert_eq!(error.category(), "request");
    assert_eq!(error.http_status(), None);
    assert!(!error.is_retryable());
    assert!(!format!("{error:?}").contains("PRIVATE"));
}

#[tokio::test]
async fn truncated_http_body_is_transport_not_rejection_or_invalid_json() {
    let response = http_response(200, "0", "{", 100).await;
    let error = checked_json(response).await.unwrap_err();
    assert_eq!(error.category(), "transport");
    assert_eq!(error.http_status(), Some(200));
    assert!(!error.is_retryable());
    let response = http_response(200, "0", "{", 1).await;
    let error = checked_json(response).await.unwrap_err();
    assert_eq!(error.category(), "invalid_response");
    assert_eq!(error.http_status(), Some(200));
}

#[tokio::test]
async fn catalog_transport_and_status_metadata_are_preserved() {
    for (status, category) in [
        (401, "authentication"),
        (429, "rate_limit"),
        (503, "unavailable"),
        (400, "request"),
    ] {
        let response = http_response(status, "3", "{}", 2).await;
        let error = catalog::json(response, &mut 1024).await.unwrap_err();
        assert_eq!(error.category(), category);
        assert_eq!(error.http_status(), Some(status));
    }
    let response = http_response(200, "0", "{", 100).await;
    let error = catalog::json(response, &mut 1024).await.unwrap_err();
    assert_eq!(error.category(), "transport");
    assert_eq!(error.http_status(), Some(200));
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn connection_failure_is_uncertain_and_never_contains_the_endpoint() {
    let error = endpoint_http_client(&native_http_client(), "http://127.0.0.1:0")
        .get("http://127.0.0.1:0/PRIVATE")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .unwrap_err();
    let error = map_transport(error);
    assert_eq!(error.category(), "transport");
    assert_eq!(error.http_status(), None);
    assert!(!error.is_retryable());
    assert!(!format!("{error:?}").contains("PRIVATE"));
    assert!(!format!("{error:?}").contains("127.0.0.1"));
}

#[tokio::test]
async fn actual_reqwest_timeout_keeps_existing_retry_policy() {
    // A listening socket that never sends headers, without a hanging server task.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let error = endpoint_http_client(&native_http_client(), &endpoint)
        .get(endpoint)
        .timeout(std::time::Duration::from_millis(50))
        .send()
        .await
        .unwrap_err();
    assert!(error.is_timeout());
    let error = map_transport(error);
    assert_eq!(error.category(), "timeout");
    assert!(error.is_retryable());
    assert_eq!(error.http_status(), None);
}
