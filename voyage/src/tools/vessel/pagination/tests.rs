use super::*;
use std::os::unix::fs::PermissionsExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
#[tokio::test]
async fn cursor_continuations_recheck_authority_preserve_utf8_and_redact_split_secrets() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.max_output_bytes = 1800;
    ctx.redactor = std::sync::Arc::new(crate::tools::Redactor::new(["fixture-secret".into()]));
    let session = Uuid::new_v4();
    let run = Uuid::new_v4();
    let source = format!("{}fixture-secret{}", "é".repeat(4000), "界".repeat(3000));
    let expected = source.replace("fixture-secret", "[REDACTED]");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let source_task = source.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let end;
            loop {
                let mut b = [0; 4096];
                let n = socket.read(&mut b).await.unwrap();
                if n == 0 {
                    return;
                }
                bytes.extend_from_slice(&b[..n]);
                if let Some(e) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let h = String::from_utf8_lossy(&bytes[..e]).to_ascii_lowercase();
                    let len = h
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= e + 4 + len {
                        end = e + 4;
                        break;
                    }
                }
            }
            let request: Value = serde_json::from_slice(&bytes[end..]).unwrap();
            let c = &request["command"];
            assert!(c["offset"].is_u64(), "unexpected fixture wire: {request}");
            let offset = c["offset"].as_u64().unwrap() as usize;
            let limit = c["limit"].as_u64().unwrap() as usize;
            let stop = offset + cut(&source_task[offset..], limit);
            let result = json!({"session_id":session,"incarnation":Uuid::nil(),"result":{"run_id":run,"offset":offset,"next_offset":stop,"total_bytes":source_task.len(),"data":&source_task[offset..stop],"state":"running"}});
            let body = json!({"protocol":1,"result":result,"error":null,"outcome_unknown":false})
                .to_string();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            if socket.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
        }
    });
    std::fs::write(
        root.path().join("process-http.json"),
        json!({"endpoint":endpoint,"token":"a".repeat(64)}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(
        root.path().join("process-http.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let transport = super::super::transport::Transport::open(root.path(), None).unwrap();
    let pages = Pages::default();
    let initial = json!({"action":"run_output","session_id":session,"run_id":run,"offset":0});
    let mut request = initial.clone();
    let mut output = String::new();
    let mut calls = 0;
    loop {
        let report = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            pages.read(&transport, &ctx, &request),
        )
        .await
        .unwrap()
        .unwrap();
        let value: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
        assert!(!value.to_string().contains("fixture-secret"));
        assert_eq!(value["offset"], output.len());
        output.push_str(value["data"].as_str().unwrap());
        calls += 1;
        assert!(calls < 100);
        if value["has_more"] == false {
            assert_eq!(value["total_bytes"], output.len());
            break;
        }
        request = value["next_read"].clone();
        if calls == 1 {
            let mut wrong = request.clone();
            wrong["offset"] = json!(99);
            let result = pages.read(&transport, &ctx, &wrong).await.unwrap();
            assert!(result.output.text_fallback().contains("cursor_mismatch"));
        }
    }
    assert_eq!(output, expected);
    assert!(calls > 2);
    let mut missing = initial.clone();
    missing["offset"] = json!(4);
    assert!(
        pages
            .read(&transport, &ctx, &missing)
            .await
            .unwrap()
            .output
            .text_fallback()
            .contains("cursor_required")
    );
    task.abort();
    let _ = task.await;
}
