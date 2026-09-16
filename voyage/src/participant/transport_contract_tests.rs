//! Private credential and IPC tests use disposable files and loopback fixtures only.
use super::*;
use serde_json::json;
#[cfg(unix)]
fn write_credential(path: &Path, endpoint: &str, token: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, serde_json::to_vec(&json!({"endpoint":endpoint,"token":token,"grant_id":uuid::Uuid::nil(),"session_id":uuid::Uuid::nil()})).unwrap()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}
#[cfg(unix)]
#[test]
fn credentials_require_private_regular_single_link_files_and_safe_origins() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("credential");
    let token = "a".repeat(64);
    for endpoint in [
        "http://127.0.0.1:1",
        "http://[::1]:1",
        "https://example.invalid",
    ] {
        write_credential(&path, endpoint, &token);
        assert_eq!(credential(&path).unwrap().endpoint, endpoint);
    }
    for endpoint in [
        "http://localhost:1",
        "http://192.0.2.1",
        "ftp://127.0.0.1",
        "https://user:pass@example.invalid",
        "https://example.invalid/path",
        "https://example.invalid?query",
        "https://example.invalid#fragment",
        "not a URL",
    ] {
        write_credential(&path, endpoint, &token);
        assert!(credential(&path).is_err(), "{endpoint}");
    }
    for token in [
        "".to_owned(),
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
    ] {
        write_credential(&path, "http://127.0.0.1", &token);
        assert!(credential(&path).is_err());
    }
    write_credential(&path, "http://127.0.0.1", &token);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(credential(&path).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = root.path().join("link");
    symlink(&path, &link).unwrap();
    assert!(credential(&link).is_err());
    let hard = root.path().join("hard");
    std::fs::hard_link(&path, &hard).unwrap();
    assert!(credential(&path).is_err());
    std::fs::remove_file(hard).unwrap();
    assert!(credential(&path).is_ok());
    std::fs::write(&path, vec![b' '; 16385]).unwrap();
    assert!(credential(&path).is_err());
    std::fs::write(&path, b"{}").unwrap();
    assert!(credential(&path).is_err());
    assert!(credential(root.path()).is_err());
    assert!(credential(&root.path().join("absent")).is_err());
}
async fn response(
    status: &str,
    body: String,
) -> (AccessCredential, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let wire = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            if let Some(end) = data.windows(4).position(|s| s == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&data[..end]).to_lowercase();
                let length = header
                    .lines()
                    .find_map(|s| s.strip_prefix("content-length:"))
                    .map(|s| s.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                if data.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let _ = stream.write_all(wire.as_bytes()).await;
        String::from_utf8(data).unwrap()
    });
    (
        AccessCredential {
            endpoint,
            token: "a".repeat(64),
            grant_id: uuid::Uuid::nil(),
            session_id: uuid::Uuid::nil(),
        },
        task,
    )
}
#[tokio::test]
async fn ipc_posts_scoped_envelope_and_preserves_remote_error_without_retry() {
    let body =
        json!({"protocol":VESSEL_API_VERSION,"result":{"fixture":true},"error":null}).to_string();
    let (access, task) = response("200 OK", body).await;
    let result = request(&access, VesselCommand::Capabilities).await.unwrap();
    assert_eq!(result.result["fixture"], true);
    let wire = task.await.unwrap();
    assert!(wire.starts_with(&format!(
        "POST {} HTTP/1.1",
        voyage_protocol::vessel::COMMAND_PATH
    )));
    assert!(wire.to_lowercase().contains("authorization: bearer "));
    let body: serde_json::Value =
        serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["protocol"], VESSEL_API_VERSION);
    assert!(
        wire.to_lowercase()
            .contains(&format!("x-voyage-grant: {}", uuid::Uuid::nil()))
    );
    assert!(body.get("command").is_some());
    let (access, task) = response("200 OK", json!({"protocol":VESSEL_API_VERSION,"result":null,"error":"assignment refused","outcome_unknown":false}).to_string()).await;
    let reply = request(&access, VesselCommand::Capabilities).await.unwrap();
    assert_eq!(reply.error.as_deref(), Some("assignment refused"));
    assert!(!reply.outcome_unknown);
    task.await.unwrap();
}
#[tokio::test]
async fn ipc_rejects_bad_status_json_and_protocol() {
    for (status, body, expected) in [
        ("403 Forbidden", "{}".to_owned(), "gateway rejected"),
        ("200 OK", "not-json".to_owned(), "expected"),
        (
            "200 OK",
            json!({"protocol":VESSEL_API_VERSION+1,"result":{},"error":null}).to_string(),
            "protocol mismatch",
        ),
    ] {
        let (access, task) = response(status, body).await;
        let error = request(&access, VesselCommand::Capabilities)
            .await
            .expect_err("request must fail")
            .to_string();
        assert!(error.contains(expected), "{error}");
        task.await.unwrap();
    }
}
