//! Actual preparation, private journal and publication refusal boundaries.
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
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const TOKEN: &str = "synthetic-authorization-fixture-token";
const NEW_SECRET: &str = "later-secret\"quoted\nline\u{1b}";

struct ApproveEverything(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl Approver for ApproveEverything {
    async fn approve(&self, _: &ApprovalRequest) -> ApprovalOutcome {
        self.0.fetch_add(1, Ordering::SeqCst);
        ApprovalOutcome::Approved
    }
}

#[derive(Clone, Copy)]
enum Refusal { Unattended, ReadOnly, NewSecret }

#[tokio::test]
async fn unattended_valid_publication_ignores_approver_that_would_allow() {
    refusal(Refusal::Unattended).await;
}
#[tokio::test]
async fn read_only_valid_publication_never_asks_or_sends() {
    refusal(Refusal::ReadOnly).await;
}
#[tokio::test]
async fn newly_configured_quoted_control_secret_refuses_prepared_publication() {
    refusal(Refusal::NewSecret).await;
}

async fn refusal(case: Refusal) {
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = reqwest::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let posts = Arc::new(AtomicUsize::new(0));
    let stop = CancellationToken::new();
    let server_stop = stop.clone();
    let seen = requests.clone();
    let sent = posts.clone();
    let body = format!("Exact body with synthetic data: {NEW_SECRET}");
    let expected_body = body.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = tokio::select! {
                biased;
                _ = server_stop.cancelled() => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            let mut request = Vec::new();
            let end = loop {
                let mut chunk = [0; 4096];
                let count = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut chunk))
                    .await.unwrap().unwrap();
                assert!(count > 0, "incomplete fixture request");
                request.extend_from_slice(&chunk[..count]);
                assert!(request.len() <= 128 * 1024, "oversized fixture request");
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = std::str::from_utf8(&request[..end]).unwrap().to_owned();
            assert!(headers.to_ascii_lowercase().contains(&format!("authorization: bearer {TOKEN}")));
            let length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
            }).unwrap_or(0);
            assert!(end + length <= 128 * 1024);
            while request.len() < end + length {
                let mut chunk = [0; 4096];
                let count = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut chunk))
                    .await.unwrap().unwrap();
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
                assert!(request.len() <= 128 * 1024);
            }
            seen.fetch_add(1, Ordering::SeqCst);
            let (status, response) = if headers.starts_with("GET /user ") {
                (200, json!({"id":7,"login":"fixture"}))
            } else if headers.starts_with("GET /repos/o/r/issues/1 ") {
                (200, json!({"number":1,"html_url":"https://github.com/o/r/issues/1","state":"open"}))
            } else {
                assert!(headers.starts_with("POST /repos/o/r/issues/1/comments "), "unexpected fixture route");
                sent.fetch_add(1, Ordering::SeqCst);
                assert_eq!(serde_json::from_slice::<serde_json::Value>(&request[end..end+length]).unwrap(), json!({"body":expected_body}));
                (201, json!({"id":9,"html_url":"https://github.com/o/r/issues/1#issuecomment-9","body":expected_body,"user":{"id":7,"login":"fixture"}}))
            };
            let response = serde_json::to_vec(&response).unwrap();
            socket.write_all(format!("HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",response.len()).as_bytes()).await.unwrap();
            socket.write_all(&response).await.unwrap();
        }
    });
    let approvals = Arc::new(AtomicUsize::new(0));
    let config = Config { github_enabled:true, access:Some(AccessMode::Unrestricted), ..Default::default() };
    let mut context = ToolContext {
        github:Some(Credential::fixture(TOKEN)),
        completion:None,
        policy:Arc::new(Policy::new(&config,directory.path().into()).unwrap()),
        approver:Arc::new(ApproveEverything(approvals.clone())),
        timeout:Duration::from_secs(5),
        max_output_bytes:64*1024,
        environment:Default::default(),
        cancellation:CancellationToken::new(),
        execution_id:Uuid::new_v4(),
        interaction:InteractionMode::Attended,
        redactor:Arc::new(Redactor::default()),
    };
    let session = Uuid::new_v4();
    let path = directory.path().join("operations");
    let service = Service::fixture(context.clone(),Some(session),origin.clone(),path.clone()).unwrap();
    let operation = tokio::time::timeout(Duration::from_secs(5), service.prepare(Draft {
        object:Object::parse("https://github.com/o/r/issues/1").unwrap(),
        action:Action::Comment { body },
    })).await.unwrap().unwrap();
    assert_eq!(operation.state, State::Prepared);
    assert_eq!(operation.actor.id,7);
    assert_eq!(operation.owner.session,Some(session));
    assert_eq!(requests.load(Ordering::SeqCst),2,"preparation must use real account and object reads");
    assert_eq!(approvals.load(Ordering::SeqCst),0);
    let before = serde_json::to_value(&operation).unwrap();
    let expected = match case {
        Refusal::Unattended => {
            context.interaction = InteractionMode::Unattended;
            "requires exact attended confirmation"
        }
        Refusal::ReadOnly => {
            let readonly = Config { access:Some(AccessMode::ReadOnly),..config };
            context.policy = Arc::new(Policy::new(&readonly,directory.path().into()).unwrap());
            "denied in read-only mode"
        }
        Refusal::NewSecret => {
            context.redactor = Arc::new(Redactor::new(vec![NEW_SECRET.to_owned()]));
            "current configured secret"
        }
    };
    drop(service);
    let service = Service::fixture(context,Some(session),origin,path.clone()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5),service.publish(operation.id,&operation.digest)).await.unwrap();
    let error = result.expect_err("current authority must refuse a genuinely prepared operation");
    assert!(error.to_string().contains(expected),"unexpected refusal: {error}");
    assert!(!error.to_string().contains(NEW_SECRET));
    assert_eq!(approvals.load(Ordering::SeqCst),0,"denied authority must not delegate to approver");
    assert_eq!(posts.load(Ordering::SeqCst),0);
    assert_eq!(requests.load(Ordering::SeqCst),2,"refusal must precede new HTTP requests");
    if matches!(case, Refusal::ReadOnly) {
        let error = tokio::time::timeout(Duration::from_secs(5),service.prepare(operation.draft.clone()))
            .await.unwrap().expect_err("read-only Service preparation must refuse before network or mutation");
        assert!(error.to_string().contains("read-only"),"unexpected preparation refusal: {error}");
        let error = tokio::time::timeout(Duration::from_secs(5),service.cancel(operation.id,operation.digest.clone()))
            .await.unwrap().expect_err("read-only Service cancellation must preserve prepared state");
        assert!(error.to_string().contains("read-only"),"unexpected cancellation refusal: {error}");
        assert_eq!(approvals.load(Ordering::SeqCst),0);
        assert_eq!(requests.load(Ordering::SeqCst),2);
        assert_eq!(posts.load(Ordering::SeqCst),0);
    }
    let stored = Store::open(path).unwrap().inspect(operation.id,service.owner()).unwrap();
    assert_eq!(serde_json::to_value(stored).unwrap(),before,"refusal must preserve exact prepared state");
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5),server).await.unwrap().unwrap();
}
