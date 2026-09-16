use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
async fn serve(status: u16, body: String) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
        }
        let response = format!(
            "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8(request).unwrap()
    });
    (url, task)
}
#[tokio::test]
async fn discovery_normalizes_catalogue_and_distinguishes_unavailable() {
    let (url, task) = serve(
        200,
        serde_json::json!({"data":[{"id":"z"},{"id":"a"},{"id":"a"}]}).to_string(),
    )
    .await;
    let found = models(&super::super::native_http_client(), &url, "synthetic-token")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        found.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "z"]
    );
    let request = task.await.unwrap();
    assert!(request.starts_with("GET /v1/models"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer synthetic-token")
    );
    for status in [404, 405, 501] {
        let (url, task) = serve(status, "unavailable".into()).await;
        assert!(
            models(&super::super::native_http_client(), &url, "")
                .await
                .unwrap()
                .is_none()
        );
        task.await.unwrap();
    }
}
#[tokio::test]
async fn malformed_secret_bearing_and_oversized_catalogues_are_refused() {
    for body in [
        serde_json::json!({}),
        serde_json::json!({"data":[{}]}),
        serde_json::json!({"data":[{"id":" "}]}),
        serde_json::json!({"data":[{"id":"synthetic-token-model"}]}),
        serde_json::json!({"data":[{"id":"bad\nmodel"}]}),
        serde_json::json!({"data":vec![serde_json::json!({"id":"x"});1025]}),
    ] {
        let (url, task) = serve(200, body.to_string()).await;
        let error = models(&super::super::native_http_client(), &url, "synthetic-token")
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("synthetic-token"));
        task.await.unwrap();
    }
}
