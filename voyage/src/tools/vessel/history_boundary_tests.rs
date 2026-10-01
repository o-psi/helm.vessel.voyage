//! Synthetic public HTTP peers exercise search authority and continuation boundaries.
use super::*;
use std::os::unix::fs::PermissionsExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn peer(
    session: Uuid,
    results: Vec<Value>,
) -> (
    tempfile::TempDir,
    super::super::transport::Transport,
    tokio::task::JoinHandle<Vec<Value>>,
) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let path = root.path().join("process-http.json");
    std::fs::write(
        &path,
        json!({"endpoint":endpoint,"token":"a".repeat(64)}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let transport = super::super::transport::Transport::open(root.path(), None).unwrap();
    let task = tokio::spawn(async move {
        let mut commands = vec![];
        for result in results {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let mut bytes = Vec::new();
            let body = loop {
                let mut buffer = [0; 2048];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0 && bytes.len() + count <= 16384);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                            .unwrap();
                    }
                }
            };
            assert_eq!(body["command"]["session_id"], json!(session));
            assert!(matches!(
                body["command"]["op"].as_str(),
                Some("history" | "message_chunk")
            ));
            commands.push(body["command"].clone());
            let response = json!({"protocol":1,"result":{"session_id":session,"incarnation":Uuid::new_v4(),"result":result},"error":null,"outcome_unknown":false}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
        }
        commands
    });
    (root, transport, task)
}

fn history(messages: Vec<Value>, total: u64) -> Value {
    json!({"revision":7,"message_offset":0,"total_messages":total,"messages":messages})
}
fn message(index: u64, role: &str, content: &str) -> Value {
    json!({"message_index":index,"role":role,"content":content,"projection_truncated":false})
}

#[tokio::test]
async fn search_role_and_match_limit_preserve_unscanned_cursor_and_redacted_offsets() {
    let id = Uuid::new_v4();
    let results = vec![history(
        vec![
            message(0, "user", "ERROR"),
            message(1, "assistant", "界ERROR"),
            message(2, "assistant", "ERROR"),
        ],
        6,
    )];
    let (root, transport, task) = peer(id, results).await;
    let context = crate::tools::reliability_tests::context(root.path());
    let result = search(
        &transport,
        id,
        "ERROR",
        Some(Role::Assistant),
        0,
        1,
        Some(7),
        &context,
    )
    .await
    .unwrap();
    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    assert_eq!(result["entries"][0]["message_index"], 1);
    assert_eq!(result["entries"][0]["match_start"], 3);
    assert_eq!(result["next_offset"], 2);
    assert_eq!(result["has_more"], true);
    let commands = task.await.unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["expected_revision"], 7);
}

#[tokio::test]
async fn search_rejects_invalid_history_identity_and_no_progress_without_extra_reads() {
    let cases = [
        json!({"message_offset":0,"total_messages":1,"messages":[]}),
        json!({"revision":7,"message_offset":0,"messages":[]}),
        json!({"revision":7,"message_offset":0,"total_messages":1,"messages":null}),
        json!({"revision":7,"message_offset":1,"total_messages":1,"messages":[]}),
        history(vec![], 1),
        history(vec![message(1, "assistant", "ERROR")], 1),
        history(vec![message(0, "assistant", "ERROR")], 0),
    ];
    for response in cases {
        let id = Uuid::new_v4();
        let (root, transport, task) = peer(id, vec![response]).await;
        let context = crate::tools::reliability_tests::context(root.path());
        assert!(
            search(&transport, id, "ERROR", None, 0, 10, None, &context)
                .await
                .is_err()
        );
        assert_eq!(task.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn search_reads_full_truncated_message_and_redacts_secret_before_matching() {
    let id = Uuid::new_v4();
    let full = json!({"role":"assistant","content":"fixture-secret ERROR"}).to_string();
    let midpoint = full.find("secret").unwrap();
    let projected = json!({"message_index":0,"role":"assistant","content":"excerpt","projection_truncated":true});
    let chunks = vec![
        history(vec![projected], 1),
        json!({"index":0,"revision":7,"offset":0,"next_offset":midpoint,"total_bytes":full.len(),"data":&full[..midpoint]}),
        json!({"index":0,"revision":7,"offset":midpoint,"next_offset":full.len(),"total_bytes":full.len(),"data":&full[midpoint..]}),
    ];
    let (root, transport, task) = peer(id, chunks).await;
    let mut context = crate::tools::reliability_tests::context(root.path());
    context.redactor =
        std::sync::Arc::new(crate::tools::Redactor::new(["fixture-secret".to_owned()]));
    let result = search(&transport, id, "ERROR", None, 0, 10, None, &context)
        .await
        .unwrap();
    assert_eq!(result["entries"][0]["kind"], "match");
    assert!(!result.to_string().contains("fixture-secret"));
    assert!(
        result["entries"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("[REDACTED]")
    );
    let commands = task.await.unwrap();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[2]["offset"], midpoint);
    assert_eq!(commands[2]["expected_revision"], 7);
}

#[tokio::test]
async fn search_withholds_oversized_full_source_and_never_treats_excerpt_as_negative() {
    let id = Uuid::new_v4();
    let projected =
        json!({"message_index":0,"role":"assistant","content":"ERROR","projection_truncated":true});
    let (root, transport, task) = peer(
        id,
        vec![
            history(vec![projected], 1),
            json!({"index":0,"revision":7,"total_bytes":4194305}),
        ],
    )
    .await;
    let context = crate::tools::reliability_tests::context(root.path());
    let result = search(&transport, id, "ERROR", None, 0, 10, None, &context)
        .await
        .unwrap();
    assert_eq!(result["entries"][0]["kind"], "unsearched");
    assert_eq!(result["entries"][0]["reason"], "source_limit");
    assert_eq!(result["next_offset"], 1);
    assert_eq!(task.await.unwrap().len(), 2);
}

#[tokio::test]
async fn revision_change_discards_earlier_matches_in_incomplete_scan() {
    let id = Uuid::new_v4();
    let projected =
        json!({"message_index":1,"role":"assistant","content":"ERROR","projection_truncated":true});
    let (root, transport, task) = peer(
        id,
        vec![
            history(vec![message(0, "assistant", "ERROR"), projected], 2),
            json!({"status":"revision_changed","complete":false}),
        ],
    )
    .await;
    let context = crate::tools::reliability_tests::context(root.path());
    let result = search(&transport, id, "ERROR", None, 0, 10, None, &context)
        .await
        .unwrap();
    assert_eq!(result["status"], "revision_changed");
    assert!(result.get("entries").is_none());
    assert_eq!(task.await.unwrap().len(), 2);
}
