//! Actual HTTP send barriers; no production transport or policy overrides.
use super::{
    Credential,
    publication::{Action, Draft},
    repository::Object,
    service::Service,
    store::{State, Store},
};
use crate::{
    Config,
    config::AccessMode,
    policy::Policy,
    tools::{ApprovalOutcome, ApprovalRequest, Approver, InteractionMode, Redactor, ToolContext},
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Barrier,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    Actor,
    Head,
    Base,
    Concurrent,
    ReceiptBusy,
}
struct Approval {
    changed: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    barrier: Option<Arc<Barrier>>,
}
#[async_trait::async_trait]
impl Approver for Approval {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        assert_eq!(request.action, "github.publish");
        assert!(request.reason.contains("Exact send boundary body"));
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(barrier) = &self.barrier {
            barrier.wait().await;
        }
        self.changed.store(true, Ordering::SeqCst);
        ApprovalOutcome::Approved
    }
}

async fn exercise(case: Case) {
    let root = tempfile::tempdir().unwrap();
    let journal = root.path().join("journal");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin =
        reqwest::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let changed = Arc::new(AtomicBool::new(false));
    let posts = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let stop = CancellationToken::new();
    let (server_changed, server_posts, server_requests, server_stop, server_journal) = (
        changed.clone(),
        posts.clone(),
        requests.clone(),
        stop.clone(),
        journal.clone(),
    );
    let held_writer = Arc::new(std::sync::Mutex::new(None::<rusqlite::Connection>));
    let server_writer = held_writer.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = tokio::select! { biased; _=server_stop.cancelled()=>break, accepted=listener.accept()=>accepted.unwrap() };
            let mut request = Vec::new();
            let (end, length) = loop {
                let mut bytes = [0; 4096];
                let n = socket.read(&mut bytes).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&bytes[..n]);
                assert!(request.len() < 128 * 1024);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&request[..end]).unwrap();
                    assert!(
                        headers
                            .to_ascii_lowercase()
                            .contains("authorization: bearer synthetic-send-token")
                    );
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (end + 4, length);
                }
            };
            assert!(end + length < 128 * 1024);
            while request.len() < end + length {
                let mut bytes = [0; 4096];
                let n = socket.read(&mut bytes).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&bytes[..n]);
            }
            server_requests.fetch_add(1, Ordering::SeqCst);
            let route = std::str::from_utf8(&request[..end])
                .unwrap()
                .lines()
                .next()
                .unwrap();
            let changed = server_changed.load(Ordering::SeqCst);
            let (status, value) = if route.starts_with("GET /user ") {
                (
                    200,
                    json!({"id":if changed && case==Case::Actor {8}else{7},"login":"fixture"}),
                )
            } else if route.starts_with("GET /repos/o/r/pulls/1 ") {
                (
                    200,
                    json!({"number":1,"html_url":"https://github.com/o/r/pull/1","state":"open",
                    "head":{"sha":if changed && case==Case::Head {"c".repeat(40)}else{"a".repeat(40)}},
                    "base":{"sha":if changed && case==Case::Base {"d".repeat(40)}else{"b".repeat(40)},"ref":"main","repo":{"id":1}}}),
                )
            } else {
                assert!(
                    route.starts_with("POST /repos/o/r/issues/1/comments "),
                    "{route}"
                );
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&request[end..end + length])
                        .unwrap(),
                    json!({"body":"Exact send boundary body"})
                );
                assert_eq!(
                    server_posts.fetch_add(1, Ordering::SeqCst),
                    0,
                    "a second transmission is never allowed"
                );
                if case == Case::ReceiptBusy {
                    let db =
                        rusqlite::Connection::open(server_journal.join("journal.sqlite3")).unwrap();
                    db.execute_batch("BEGIN IMMEDIATE").unwrap();
                    *server_writer.lock().unwrap() = Some(db);
                }
                (
                    201,
                    json!({"id":9,"html_url":"https://github.com/o/r/pull/1#issuecomment-9","body":"Exact send boundary body","user":{"id":7}}),
                )
            };
            let bytes = serde_json::to_vec(&value).unwrap();
            socket.write_all(format!("HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).as_bytes()).await.unwrap();
            socket.write_all(&bytes).await.unwrap();
        }
    });
    let config = Config {
        github_enabled: true,
        access: Some(AccessMode::Unrestricted),
        ..Default::default()
    };
    let context = ToolContext {
        github: Some(Credential::fixture("synthetic-send-token")),
        completion: None,
        policy: Arc::new(Policy::new(&config, root.path().into()).unwrap()),
        approver: Arc::new(Approval {
            changed,
            calls: calls.clone(),
            barrier: (case == Case::Concurrent).then(|| Arc::new(Barrier::new(2))),
        }),
        timeout: Duration::from_secs(5),
        max_output_bytes: 64 * 1024,
        environment: Default::default(),
        cancellation: CancellationToken::new(),
        execution_id: Uuid::new_v4(),
        interaction: InteractionMode::Attended,
        redactor: Arc::new(Redactor::default()),
    };
    let session = Some(Uuid::new_v4());
    let service =
        Service::fixture(context.clone(), session, origin.clone(), journal.clone()).unwrap();
    let operation = service
        .prepare(Draft {
            object: Object::parse("https://github.com/o/r/pull/1").unwrap(),
            action: Action::Comment {
                body: "Exact send boundary body".into(),
            },
        })
        .await
        .unwrap();
    let original = serde_json::to_value(&operation).unwrap();
    let result = if case == Case::Concurrent {
        let second =
            Service::fixture(context.clone(), session, origin.clone(), journal.clone()).unwrap();
        let (a, b) = tokio::join!(
            service.publish(operation.id, &operation.digest),
            second.publish(operation.id, &operation.digest)
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "both callers must reach exact approval before racing send intent"
        );
        a.or(b)
    } else {
        service.publish(operation.id, &operation.digest).await
    };
    if let Some(db) = held_writer.lock().unwrap().take() {
        db.execute_batch("ROLLBACK").unwrap();
    }
    let stored = Store::open(journal.clone())
        .unwrap()
        .inspect(operation.id, service.owner())
        .unwrap();
    match case {
        Case::Actor | Case::Head | Case::Base => {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("changed after approval"), "{error}");
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(posts.load(Ordering::SeqCst), 0);
            assert_eq!(serde_json::to_value(stored).unwrap(), original);
        }
        Case::Concurrent => {
            assert!(result.is_ok());
            assert_eq!(stored.state, State::Published);
            assert_eq!(posts.load(Ordering::SeqCst), 1);
        }
        Case::ReceiptBusy => {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("https://github.com/o/r/pull/1#issuecomment-9"),
                "{error}"
            );
            assert!(error.contains("local receipt confirmation failed"));
            assert_eq!(stored.state, State::Sending);
            assert!(stored.receipt.is_none());
            assert_eq!(posts.load(Ordering::SeqCst), 1);
            let before = requests.load(Ordering::SeqCst);
            let reopened = Service::fixture(context, session, origin, journal).unwrap();
            let error = reopened
                .publish(operation.id, &operation.digest)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("already sent"));
            assert_eq!(requests.load(Ordering::SeqCst), before);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
    stop.cancel();
    server.await.unwrap();
}
#[tokio::test]
async fn account_and_pr_head_base_changes_during_approval_send_nothing() {
    for case in [Case::Actor, Case::Head, Case::Base] {
        tokio::time::timeout(Duration::from_secs(10), exercise(case))
            .await
            .unwrap();
    }
}
#[tokio::test]
async fn concurrent_approved_callers_commit_only_one_send() {
    tokio::time::timeout(Duration::from_secs(10), exercise(Case::Concurrent))
        .await
        .unwrap();
}
#[tokio::test]
async fn successful_http_receipt_storage_failure_retains_sending_and_exact_url() {
    tokio::time::timeout(Duration::from_secs(10), exercise(Case::ReceiptBusy))
        .await
        .unwrap();
}
