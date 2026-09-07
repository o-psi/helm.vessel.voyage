use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn reject(body: &str) -> ProviderError {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nRetry-After: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
