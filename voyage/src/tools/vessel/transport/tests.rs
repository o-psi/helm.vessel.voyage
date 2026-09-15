use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};
fn local(root: &Path, url: &str, token: &str) {
    std::fs::write(
        root.join("process-http.json"),
        json!({"endpoint":url,"token":token}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(
        root.join("process-http.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
}
#[test]
fn local_discovery_is_private_and_forbids_remote_or_credentialed_urls() {
    let root = tempfile::tempdir().unwrap();
    let token = "a".repeat(64);
    for url in ["http://127.0.0.1:1234/other", "http://[::1]:1234"] {
        local(root.path(), url, &token);
        let transport = Transport::open(root.path(), None).unwrap();
        assert_eq!(transport.endpoint.path(), COMMAND_PATH);
        assert_eq!(
            transport.redact_complete(json!({"nested":[token.clone()]}))["nested"][0],
            "[REDACTED]"
        );
    }
    for url in [
        "https://example.com",
        "http://localhost",
        "http://user:pass@127.0.0.1",
        "http://127.0.0.1?key=secret",
        "http://127.0.0.1#fragment",
    ] {
        local(root.path(), url, &token);
        assert!(Transport::open(root.path(), None).is_err(), "{url}");
    }
    for token in ["short".into(), "x".repeat(64)] {
        local(root.path(), "http://127.0.0.1", &token);
        assert!(Transport::open(root.path(), None).is_err());
    }
    local(root.path(), "http://127.0.0.1", &token);
    let path = root.path().join("process-http.json");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Transport::open(root.path(), None).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let hard = root.path().join("hard");
    std::fs::hard_link(&path, &hard).unwrap();
    assert!(private_read(&path, 4096).is_err());
    std::fs::remove_file(hard).unwrap();
    let link = root.path().join("link");
    symlink(&path, &link).unwrap();
    assert!(private_read(&link, 4096).is_err());
    assert!(private_read(&path, 1).is_err());
}
#[tokio::test]
async fn remote_diagnostics_are_classified_not_disclosed_or_replayed() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    for (error, code) in [
        ("revision secret", "stale_revision"),
        ("permission secret", "permission_denied"),
        ("cleanup secret", "cleanup_pending"),
        ("busy secret", "busy"),
        ("not found secret", "not_found"),
        ("unsupported secret", "unsupported_or_unavailable"),
        ("expired secret", "expired"),
        ("private secret", "refused"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let root = tempfile::tempdir().unwrap();
        local(root.path(), &url, &"a".repeat(64));
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            socket.read(&mut buffer).await.unwrap();
            let body=json!({"protocol":VESSEL_API_VERSION,"result":null,"error":error,"outcome_unknown":true}).to_string();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
        });
        let value = Transport::open(root.path(), None)
            .unwrap()
            .exchange(VesselCommand::Capabilities)
            .await
            .unwrap();
        assert_eq!(value["code"], code);
        assert_eq!(value["status"], "outcome_unknown");
        assert!(!value.to_string().contains("secret"));
        task.await.unwrap();
    }
}
