//! Real HTTP candidate reads and durable uncertain-operation reconciliation.
use super::{
    Credential,
    publication::{Action, Draft, ReviewEvent},
    repository::Object,
    service::Service,
    store::{ReceiptEvidence, State, Store},
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
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const TOKEN: &str = "synthetic-recovery-credential";
const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BODY: &str = "Exact uncertain publication body";
#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    Adopt,
    Actor,
    Body,
    Object,
    Id,
    Event,
    Commit,
    Deny,
    Cancel,
    ReadOnly,
    Revoked,
}

#[derive(Debug)]
struct Authority(Arc<AtomicBool>);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.0.load(Ordering::SeqCst),
            "fixture execution authority revoked"
        );
        Ok(())
    }
}

struct Decision {
    case: Case,
    calls: Arc<AtomicUsize>,
    digest: Arc<Mutex<String>>,
    cancel: CancellationToken,
}
#[async_trait::async_trait]
impl Approver for Decision {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.action, "github.reconcile");
        assert!(request.reason.contains(&*self.digest.lock().unwrap()));
        assert!(request.reason.contains(BODY));
        assert!(request.reason.contains("No request will be resent"));
        assert!(!request.reason.contains(TOKEN));
        if self.case == Case::Cancel {
            self.cancel.cancel();
            std::future::pending::<ApprovalOutcome>().await
        } else if self.case == Case::Deny {
            ApprovalOutcome::Denied
        } else {
            ApprovalOutcome::Approved
        }
    }
}

async fn exercise(review: bool, case: Case) {
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin: reqwest::Url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let object_url = if review {
        "https://github.com/o/r/pull/1"
    } else {
        "https://github.com/o/r/issues/1"
    };
    let detail_path = if review {
        "/repos/o/r/pulls/1"
    } else {
        "/repos/o/r/issues/1"
    };
    let candidate_path = if review {
        "/repos/o/r/pulls/1/reviews/9"
    } else {
        "/repos/o/r/issues/comments/9"
    };
    let marker = if review {
        "pullrequestreview"
    } else {
        "issuecomment"
    };
    let mut candidate = json!({"id":9,"html_url":format!("{object_url}#{marker}-9"),"body":BODY,"user":{"id":7,"login":"fixture"},"commit_id":HEAD,"state":"COMMENTED"});
    match case {
        Case::Body => candidate["body"] = json!("different"),
        Case::Object => candidate["html_url"] = json!(format!("{object_url}2#{marker}-9")),
        Case::Id => {
            candidate["id"] = json!(10);
            candidate["html_url"] = json!(format!("{object_url}#{marker}-10"));
        }
        Case::Event => candidate["state"] = json!("APPROVED"),
        Case::Commit => candidate["commit_id"] = json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        _ => (),
    }
    let recovering = Arc::new(AtomicBool::new(false));
    let recovery_stage = recovering.clone();
    let candidate_reads = Arc::new(AtomicUsize::new(0));
    let candidates = candidate_reads.clone();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let stop = CancellationToken::new();
    let shutdown = stop.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = tokio::select! { biased; _=shutdown.cancelled()=>break, accepted=listener.accept()=>accepted.unwrap() };
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let count = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut chunk))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                assert!(bytes.len() <= 16384);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8(bytes).unwrap();
            let path = text
                .lines()
                .next()
                .unwrap()
                .strip_prefix("GET ")
                .expect("reconciliation must never POST")
                .strip_suffix(" HTTP/1.1")
                .unwrap();
            assert!(
                text.to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {TOKEN}"))
            );
            seen.fetch_add(1, Ordering::SeqCst);
            let value = if path == "/user" {
                json!({"id":if recovery_stage.load(Ordering::SeqCst) && case==Case::Actor {8} else {7},"login":"fixture"})
            } else if path == detail_path {
                json!({"number":1,"html_url":object_url,"state":"open","head":{"sha":HEAD},"base":{"sha":HEAD,"ref":"main","repo":{"id":1}}})
            } else {
                assert_eq!(path, candidate_path, "unexpected recovery request");
                assert!(recovery_stage.load(Ordering::SeqCst));
                candidates.fetch_add(1, Ordering::SeqCst);
                candidate.clone()
            };
            let body = serde_json::to_vec(&value).unwrap();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        }
    });
    let cancel = CancellationToken::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let digest = Arc::new(Mutex::new(String::new()));
    let config = Config {
        github_enabled: true,
        access: Some(AccessMode::Unrestricted),
        ..Default::default()
    };
    let revoked = Arc::new(AtomicBool::new(false));
    let mut context = ToolContext {
        github: Some(Credential::fixture(TOKEN)),
        completion: None,
        policy: Arc::new(
            Policy::new(&config, directory.path().into())
                .unwrap()
                .with_execution_authority(Arc::new(Authority(revoked.clone()))),
        ),
        approver: Arc::new(Decision {
            case,
            calls: calls.clone(),
            digest: digest.clone(),
            cancel: cancel.clone(),
        }),
        timeout: Duration::from_secs(5),
        max_output_bytes: 65536,
        environment: Default::default(),
        cancellation: cancel,
        execution_id: Uuid::new_v4(),
        interaction: InteractionMode::Attended,
        redactor: Arc::new(Redactor::default()),
    };
    let session = Uuid::new_v4();
    let path = directory.path().join("operations");
    let service =
        Service::fixture(context.clone(), Some(session), origin.clone(), path.clone()).unwrap();
    let action = if review {
        Action::Review {
            event: ReviewEvent::Comment,
            commit_id: HEAD.into(),
            body: BODY.into(),
            comments: vec![],
        }
    } else {
        Action::Comment { body: BODY.into() }
    };
    let operation = tokio::time::timeout(
        Duration::from_secs(5),
        service.prepare(Draft {
            object: Object::parse(object_url).unwrap(),
            action,
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    *digest.lock().unwrap() = operation.digest.clone();
    Store::open(path.clone())
        .unwrap()
        .begin_send(&operation)
        .unwrap();
    drop(service);
    recovering.store(true, Ordering::SeqCst);
    if case == Case::ReadOnly {
        let readonly = Config {
            access: Some(AccessMode::ReadOnly),
            ..config
        };
        context.policy = Arc::new(Policy::new(&readonly, directory.path().into()).unwrap());
    }
    let service = Service::fixture(context, Some(session), origin, path.clone()).unwrap();
    if case == Case::Revoked {
        revoked.store(true, Ordering::SeqCst);
    }
    let before = requests.load(Ordering::SeqCst);
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        service.reconcile(operation.id, &operation.digest, 9),
    )
    .await
    .unwrap();
    let durable = Store::open(path)
        .unwrap()
        .inspect(operation.id, &operation.owner)
        .unwrap();
    if case == Case::Adopt {
        assert!(result.is_ok());
        assert_eq!(durable.state, State::Published);
        let receipt = durable.receipt.unwrap();
        assert_eq!(receipt.id, 9);
        assert!(matches!(receipt.evidence, ReceiptEvidence::OperatorAdopted));
        let after = requests.load(Ordering::SeqCst);
        assert!(
            service
                .reconcile(operation.id, &operation.digest, 9)
                .await
                .is_err()
        );
        assert_eq!(
            requests.load(Ordering::SeqCst),
            after,
            "adopted operation was resent"
        );
    } else {
        assert!(result.is_err());
        assert_eq!(durable.state, State::Sending);
        assert!(durable.receipt.is_none());
    }
    let should_prompt = matches!(case, Case::Adopt | Case::Deny | Case::Cancel);
    assert_eq!(calls.load(Ordering::SeqCst), usize::from(should_prompt));
    assert_eq!(
        candidate_reads.load(Ordering::SeqCst),
        usize::from(!matches!(
            case,
            Case::Actor | Case::ReadOnly | Case::Revoked
        ))
    );
    if matches!(case, Case::ReadOnly | Case::Revoked) {
        assert_eq!(requests.load(Ordering::SeqCst), before);
    }
    stop.cancel();
    server.await.unwrap();
}

#[tokio::test]
async fn uncertain_comment_and_review_adoption_survive_reopen_without_resend() {
    for review in [false, true] {
        exercise(review, Case::Adopt).await;
    }
}
#[tokio::test]
async fn mismatched_recovery_candidates_preserve_uncertainty() {
    for review in [false, true] {
        for case in [Case::Actor, Case::Body, Case::Object, Case::Id] {
            exercise(review, case).await;
        }
    }
    for case in [Case::Event, Case::Commit] {
        exercise(true, case).await;
    }
}
#[tokio::test]
async fn denied_cancelled_and_read_only_reconciliation_never_adopt_or_send() {
    for case in [Case::Deny, Case::Cancel, Case::ReadOnly, Case::Revoked] {
        exercise(false, case).await;
    }
}
