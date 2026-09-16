//! End-to-end discovery contracts over a disposable loopback JSON-RPC peer.
use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
async fn peer(results: Vec<Option<Value>>) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for result in results {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let body_start;
            loop {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        body_start = end + 4;
                        break;
                    }
                }
            }
            let request: Value = serde_json::from_slice(&bytes[body_start..]).unwrap();
            let wire = if let Some(result) = result {
                let body = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
            };
            stream.write_all(wire.as_bytes()).await.unwrap();
            requests.push(request);
        }
        requests
    });
    (endpoint, task)
}
fn initialized(tools: bool) -> Value {
    json!({"protocolVersion":"2025-06-18","capabilities":if tools {json!({"tools":{}})} else {json!({})},"serverInfo":{"name":"offline","version":"1"}})
}
#[tokio::test]
async fn http_discovery_paginates_and_dispatches_structured_output_without_processes() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.artifact_scope = Some(crate::artifacts::Scope {
        directory: root.path().join("artifacts"),
        session: uuid::Uuid::new_v4(),
    });
    let (endpoint, peer) = peer(vec![Some(initialized(true)),None,
        Some(json!({"tools":[{"name":"first","inputSchema":{"type":"object"},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}],"nextCursor":"page2"})),
        Some(json!({"tools":[{"name":"second","description":"second tool","inputSchema":{"type":"object"}}]})),
        Some(json!({"content":[{"type":"text","text":"observed"}],"structuredContent":{"ok":true},"isError":false})),
    ]).await;
    let server = McpServer::start_http("Fixture", &endpoint, None, &ctx.policy).unwrap();
    server.initialize().await.unwrap();
    let tools = server.discover().await.unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].definition().name, "mcp_fixture_first");
    assert!(tools[0].definition().annotations.is_some());
    let output = tools[0].execute_output(json!({}), &ctx).await.unwrap();
    assert_eq!(output.structured_content.unwrap()["ok"], true);
    server.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
    assert!(server.observed());
    let requests = peer.await.unwrap();
    assert_eq!(requests[1]["method"], "notifications/initialized");
    assert_eq!(requests[3]["params"]["cursor"], "page2");
    assert_eq!(requests[4]["params"]["name"], "first");
}
#[tokio::test]
async fn absent_capability_avoids_discovery_and_bad_initialization_closes_transport() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.artifact_scope = Some(crate::artifacts::Scope {
        directory: root.path().join("artifacts"),
        session: uuid::Uuid::new_v4(),
    });
    let (endpoint, task) = peer(vec![Some(initialized(false)), None]).await;
    let server = McpServer::start_http("fixture", &endpoint, None, &ctx.policy).unwrap();
    server.initialize().await.unwrap();
    assert!(server.discover().await.unwrap().is_empty());
    server.shutdown().await.unwrap();
    assert_eq!(task.await.unwrap().len(), 2);
    for value in [
        json!({"protocolVersion":"wrong","capabilities":{},"serverInfo":{}}),
        json!({"protocolVersion":"2025-06-18","capabilities":null,"serverInfo":{}}),
        json!({"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":null}),
    ] {
        let (endpoint, task) = peer(vec![Some(value)]).await;
        let server = McpServer::start_http("fixture", &endpoint, None, &ctx.policy).unwrap();
        assert!(server.initialize().await.is_err());
        assert!(server.transport.closed.load(Ordering::Acquire));
        server.shutdown().await.unwrap();
        task.await.unwrap();
    }
}
#[tokio::test]
async fn malformed_discovery_contracts_and_cursor_cycles_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.artifact_scope = Some(crate::artifacts::Scope {
        directory: root.path().join("artifacts"),
        session: uuid::Uuid::new_v4(),
    });
    for result in [
        json!({}),
        json!({"tools":[{}]}),
        json!({"tools":[{"name":"x","inputSchema":{"type":"string"}}]}),
        json!({"tools":[{"name":"x","inputSchema":{"type":"object"},"outputSchema":true}]}),
        json!({"tools":[{"name":"x","inputSchema":{"type":"object"},"annotations":"wrong"}]}),
    ] {
        let (endpoint, task) = peer(vec![Some(initialized(true)), None, Some(result)]).await;
        let server = McpServer::start_http("fixture", &endpoint, None, &ctx.policy).unwrap();
        server.initialize().await.unwrap();
        assert!(server.discover().await.is_err());
        server.shutdown().await.unwrap();
        task.await.unwrap();
    }
    let page = json!({"tools":[],"nextCursor":"cycle"});
    let (endpoint, task) = peer(vec![
        Some(initialized(true)),
        None,
        Some(page.clone()),
        Some(page),
    ])
    .await;
    let server = McpServer::start_http("fixture", &endpoint, None, &ctx.policy).unwrap();
    server.initialize().await.unwrap();
    assert!(server.discover().await.is_err());
    server.shutdown().await.unwrap();
    task.await.unwrap();
}
