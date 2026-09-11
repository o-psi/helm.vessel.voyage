use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn reject(body: &str) -> ProviderError {
    reject_with(429, "1", body).await
}

async fn reject_with(status: u16, retry_after: &str, body: &str) -> ProviderError {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nRetry-After: {retry_after}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
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
    let response = native_http_client().get(endpoint).send().await.unwrap();
    let error = checked_stream_response(response).await.unwrap_err();
    server.await.unwrap();
    error
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
    assert!(matches!(error, ProviderError::RetryAfter { .. }));
}

#[tokio::test]
async fn exhausted_account_is_not_retried_or_exposed_as_raw_diagnostics() {
    for body in [
        r#"{"error":{"type":"usage_limit_reached","message":"secret-provider-text","plan_type":"private-plan"}}"#,
        r#"{"error":{"code":"insufficient_quota","message":"secret-provider-text"}}"#,
    ] {
        let error = reject(body).await;
        assert!(matches!(error, ProviderError::UsageLimit));
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
        assert!(matches!(error, ProviderError::RateLimit { .. }));
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
        assert!(matches!(error, ProviderError::ContextLength));
        assert!(!error.is_retryable());
        assert_eq!(error.retry_after(), None);
        assert_eq!(error.to_string(), CONTEXT_LENGTH_MESSAGE);
        assert_eq!(error.public_failure_reason(), CONTEXT_LENGTH_MESSAGE);
        assert!(!format!("{error:?}").contains("PRIVATE"));
        assert!(matches!(
            multimodal::redact(error),
            ProviderError::ContextLength
        ));
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
        assert!(!matches!(
            reject_with(status, "12", body).await,
            ProviderError::ContextLength
        ));
    }
    let error =
        reject(r#"{"error":{"type":"usage_limit_reached","code":"context_length_exceeded"}}"#)
            .await;
    assert!(matches!(error, ProviderError::UsageLimit));
}
