use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn fixture(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length = header
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    (url, task)
}
fn response(status: u16, mime: &str, extra: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    )
}
#[test]
fn endpoint_and_credentials_refuse_unsafe_inputs() {
    for endpoint in [
        "http://127.0.0.1:8080/mcp",
        "http://[::1]/mcp",
        "https://example.com/mcp",
    ] {
        validate_http_endpoint(endpoint).unwrap();
    }
    for endpoint in [
        "http://example.com/mcp",
        "http://localhost/mcp",
        "file:///tmp/a",
        "https://u:p@example.com/",
        "https://example.com/?key=x",
        "https://example.com/#x",
        "garbage",
    ] {
        assert!(validate_http_endpoint(endpoint).is_err(), "{endpoint}");
    }
    for token in ["", "bad\nheader", "bad\rheader"] {
        assert!(HttpTransport::new("https://example.com", Some(token)).is_err());
    }
}
#[tokio::test]
async fn session_headers_notification_and_cleanup_are_bound_and_idempotent() {
    let body = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
    let (url, task) = fixture(vec![
        response(
            200,
            "application/json",
            "Mcp-Session-Id: fixture-session\r\n",
            body,
        ),
        response(202, "application/json", "", ""),
        response(404, "application/json", "", ""),
    ])
    .await;
    let transport = HttpTransport::new(&url, Some("synthetic-fixture-token")).unwrap();
    assert_eq!(transport.request(b"{}", 1, true).await.unwrap()["id"], 1);
    transport.send(b"{}").await.unwrap();
    transport.shutdown().await.unwrap();
    transport.shutdown().await.unwrap();
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 3);
    let first = requests[0].to_ascii_lowercase();
    assert!(first.contains("authorization: bearer synthetic-fixture-token"));
    assert!(!first.contains("mcp-session-id:"));
    let second = requests[1].to_ascii_lowercase();
    assert!(second.contains("mcp-session-id: fixture-session"));
    assert!(second.contains("mcp-protocol-version: 2025-06-18"));
    assert!(requests[2].starts_with("DELETE /mcp "));
}
#[tokio::test]
async fn sse_accepts_multiline_data_bom_comments_and_unrelated_events() {
    let body = "\u{feff}:comment\r\nevent: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\r\n\r\ndata: {\"jsonrpc\":\"2.0\",\n data-ignored\ndata: \"id\":7,\"result\":{\"ok\":true}}\n\n";
    let (url, task) = fixture(vec![response(
        200,
        "text/event-stream; charset=utf-8",
        "",
        body,
    )])
    .await;
    let transport = HttpTransport::new(&url, None).unwrap();
    assert_eq!(
        transport.request(b"{}", 7, false).await.unwrap()["result"]["ok"],
        true
    );
    task.await.unwrap();
}
#[tokio::test]
async fn malformed_responses_are_refused_without_automatic_replay() {
    for (status, mime, extra, body, expected) in [
        (
            302,
            "application/json",
            "Location: http://127.0.0.1:1/\r\n",
            "",
            "status 302",
        ),
        (500, "application/json", "", "", "status 500"),
        (200, "text/plain", "", "{}", "content type"),
        (200, "application/json", "", "broken", "malformed JSON"),
        (
            200,
            "application/json",
            "",
            r#"{"jsonrpc":"2.0","id":2,"result":{}}"#,
            "mismatched",
        ),
        (
            200,
            "application/json",
            "",
            r#"{"jsonrpc":"2.0","id":1,"result":{},"error":{}}"#,
            "envelope",
        ),
        (
            200,
            "text/event-stream",
            "",
            "data: invalid\n\n",
            "malformed JSON",
        ),
        (
            200,
            "text/event-stream",
            "",
            "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
            "before matching",
        ),
        (
            200,
            "application/json",
            "Mcp-Session-Id: bad session\r\n",
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
            "session header",
        ),
    ] {
        let (url, task) = fixture(vec![response(status, mime, extra, body)]).await;
        let transport = HttpTransport::new(&url, None).unwrap();
        let error = transport
            .request(b"{}", 1, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error} expected {expected}");
        assert_eq!(task.await.unwrap().len(), 1);
    }
}
#[tokio::test]
async fn notification_body_and_status_are_not_accepted_as_acknowledgement() {
    for (status, body) in [(200, ""), (202, "unexpected")] {
        let (url, task) = fixture(vec![response(status, "application/json", "", body)]).await;
        assert!(
            HttpTransport::new(&url, None)
                .unwrap()
                .send(b"{}")
                .await
                .is_err()
        );
        task.await.unwrap();
    }
}
