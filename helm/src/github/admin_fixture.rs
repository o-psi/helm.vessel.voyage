//! Actual offline operator adapter, approval and private-store recovery boundaries.
use super::{
    admin::{self, Command},
    publication::{Action, Actor, Draft},
    repository::Object,
    store::{Owner, State, Store},
};
use crate::{
    Config,
    config::AccessMode,
    policy::Policy,
    tools::{ApprovalOutcome, ApprovalRequest, Approver, InteractionMode, Redactor, ToolContext},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Decision {
    calls: Arc<AtomicUsize>,
    allow: bool,
    change: Option<(std::path::PathBuf, super::store::Operation)>,
    cancel: Option<CancellationToken>,
}
#[async_trait::async_trait]
impl Approver for Decision {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.action, "github.dispose");
        assert!(!request.reason.is_empty());
        if let Some((directory, operation)) = &self.change {
            Store::open(directory.clone())
                .unwrap()
                .begin_send(operation)
                .unwrap();
        }
        if let Some(cancel) = &self.cancel {
            cancel.cancel();
        }
        if self.allow {
            ApprovalOutcome::Approved
        } else {
            ApprovalOutcome::Denied
        }
    }
}
fn context(root: &std::path::Path, calls: Arc<AtomicUsize>) -> ToolContext {
    let config = Config {
        access: Some(AccessMode::Unrestricted),
        ..Default::default()
    };
    ToolContext {
        github: None,
        completion: None,
        policy: Arc::new(Policy::new(&config, root.into()).unwrap()),
        approver: Arc::new(Decision {
            calls,
            allow: true,
            change: None,
            cancel: None,
        }),
        timeout: Duration::from_secs(5),
        max_output_bytes: 64 * 1024,
        environment: Default::default(),
        cancellation: CancellationToken::new(),
        execution_id: Uuid::new_v4(),
        interaction: InteractionMode::Attended,
        redactor: Arc::new(Redactor::default()),
    }
}
fn prepare(store: &mut Store, owner: &Owner, index: usize) -> super::store::Operation {
    store
        .prepare(
            Draft {
                object: Object::parse("https://github.com/fixture/repo/issues/1").unwrap(),
                action: Action::Comment {
                    body: format!("private draft {index}"),
                },
            },
            Actor {
                id: 7,
                login: "fixture".into(),
            },
            "fixture-policy".into(),
            None,
            None,
            owner.clone(),
        )
        .unwrap()
}

#[tokio::test]
async fn orphan_at_full_capacity_can_be_inspected_disposed_audited_and_forgotten_offline() {
    let root = tempfile::tempdir().unwrap();
    let orphan = root.path().join("deleted-workspace");
    std::fs::create_dir(&orphan).unwrap();
    let owner = Owner::new(&orphan, Some(Uuid::new_v4()), Some(Uuid::new_v4())).unwrap();
    let directory = root.path().join("journal");
    let mut store = Store::open(directory.clone()).unwrap();
    let operation = prepare(&mut store, &owner, 0);
    store.begin_send(&operation).unwrap();
    for index in 1..512 {
        prepare(&mut store, &owner, index);
    }
    assert!(
        store
            .prepare(
                operation.draft.clone(),
                operation.actor.clone(),
                "another-policy".into(),
                None,
                None,
                owner.clone()
            )
            .is_err()
    );
    drop(store);
    std::fs::remove_dir(&orphan).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let context = context(root.path(), calls.clone());
    assert!(context.github.is_none());
    assert!(context.environment.is_empty());
    let inspected = admin::fixture(
        context.clone(),
        Command::Inspect { id: operation.id },
        None,
        directory.clone(),
    )
    .await
    .unwrap();
    assert!(inspected.display.contains("private draft 0"));
    let other = Owner::new(root.path(), Some(Uuid::new_v4()), None).unwrap();
    assert!(
        admin::fixture(
            context.clone(),
            Command::Inspect { id: operation.id },
            Some(other),
            directory.clone()
        )
        .await
        .is_err()
    );
    let dispose = Command::Dispose {
        id: operation.id,
        digest: operation.digest.clone(),
        note: "Investigated; remote outcome remains unknown".into(),
    };
    admin::fixture(context.clone(), dispose, None, directory.clone())
        .await
        .unwrap();
    assert_eq!(
        Store::open(directory.clone())
            .unwrap()
            .inspect(operation.id, &owner)
            .unwrap()
            .state,
        State::Disposed
    );
    admin::fixture(
        context.clone(),
        Command::Forget {
            id: operation.id,
            digest: operation.digest.clone(),
        },
        None,
        directory.clone(),
    )
    .await
    .unwrap();
    let audit = admin::fixture(context.clone(), Command::Audit, None, directory.clone())
        .await
        .unwrap();
    assert!(!audit.display.contains("private draft 0"));
    let audit: serde_json::Value = serde_json::from_str(&audit.display).unwrap();
    assert_eq!(audit["entries"].as_array().unwrap().len(), 1);
    assert_eq!(audit["entries"][0]["former_state"], "disposed");
    assert!(
        admin::fixture(
            context.clone(),
            Command::ClearAudit {
                digest: "stale".into()
            },
            None,
            directory.clone()
        )
        .await
        .is_err()
    );
    admin::fixture(
        context,
        Command::ClearAudit {
            digest: audit["digest"].as_str().unwrap().into(),
        },
        None,
        directory.clone(),
    )
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let mut store = Store::open(directory).unwrap();
    assert!(store.inspect(operation.id, &owner).is_err());
    prepare(&mut store, &owner, 513);
}

#[tokio::test]
async fn admin_refusal_and_cancel_during_decision_preserve_exact_records() {
    let root = tempfile::tempdir().unwrap();
    let owner = Owner::new(root.path(), None, None).unwrap();
    let directory = root.path().join("journal");
    let mut store = Store::open(directory.clone()).unwrap();
    let operation = prepare(&mut store, &owner, 0);
    drop(store);
    for mode in ["unattended", "read-only", "denied", "cancel"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut context = context(root.path(), calls.clone());
        if mode == "unattended" {
            context.interaction = InteractionMode::Unattended;
        }
        if mode == "read-only" {
            context.policy = Arc::new(
                Policy::new(
                    &Config {
                        access: Some(AccessMode::ReadOnly),
                        ..Default::default()
                    },
                    root.path().into(),
                )
                .unwrap(),
            );
        }
        context.approver = Arc::new(Decision {
            calls: calls.clone(),
            allow: mode != "denied",
            change: None,
            cancel: (mode == "cancel").then(|| context.cancellation.clone()),
        });
        assert!(
            admin::fixture(
                context,
                Command::Cancel {
                    id: operation.id,
                    digest: operation.digest.clone()
                },
                None,
                directory.clone()
            )
            .await
            .is_err()
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            usize::from(mode == "denied" || mode == "cancel")
        );
        let stored = Store::open(directory.clone())
            .unwrap()
            .inspect(operation.id, &owner)
            .unwrap();
        assert_eq!(
            serde_json::to_value(stored).unwrap(),
            serde_json::to_value(&operation).unwrap()
        );
    }
}

#[tokio::test]
async fn concurrent_send_during_admin_confirmation_rejects_stale_mutable_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let owner = Owner::new(root.path(), None, None).unwrap();
    let directory = root.path().join("journal");
    let mut store = Store::open(directory.clone()).unwrap();
    let operation = prepare(&mut store, &owner, 0);
    drop(store);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut context = context(root.path(), calls.clone());
    context.approver = Arc::new(Decision {
        calls: calls.clone(),
        allow: true,
        change: Some((directory.clone(), operation.clone())),
        cancel: None,
    });
    assert!(
        admin::fixture(
            context,
            Command::Cancel {
                id: operation.id,
                digest: operation.digest.clone()
            },
            None,
            directory.clone()
        )
        .await
        .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let store = Store::open(directory).unwrap();
    assert_eq!(
        store.inspect(operation.id, &owner).unwrap().state,
        State::Sending
    );
    assert!(store.audit().unwrap().entries.is_empty());
}

#[tokio::test]
async fn admin_displays_escaped_secrets_safely_and_refuses_rewritten_approval() {
    let root = tempfile::tempdir().unwrap();
    let owner = Owner::new(root.path(), None, None).unwrap();
    let directory = root.path().join("journal");
    let mut store = Store::open(directory.clone()).unwrap();
    let secret = "private\nquoted\"body";
    let operation = store
        .prepare(
            Draft {
                object: Object::parse("https://github.com/fixture/repo/issues/1").unwrap(),
                action: Action::Comment {
                    body: secret.into(),
                },
            },
            Actor {
                id: 7,
                login: "fixture".into(),
            },
            "fixture-policy".into(),
            None,
            None,
            owner.clone(),
        )
        .unwrap();
    drop(store);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut context = context(root.path(), calls.clone());
    context.redactor = Arc::new(Redactor::new([secret.into()]));
    let display = admin::fixture(
        context.clone(),
        Command::Inspect { id: operation.id },
        None,
        directory.clone(),
    )
    .await
    .unwrap();
    let display: serde_json::Value = serde_json::from_str(&display.display).unwrap();
    assert_eq!(display["draft"]["action"]["body"], "[REDACTED]");
    assert!(
        admin::fixture(
            context,
            Command::Cancel {
                id: operation.id,
                digest: operation.digest.clone()
            },
            None,
            directory.clone()
        )
        .await
        .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        serde_json::to_value(
            Store::open(directory)
                .unwrap()
                .inspect(operation.id, &owner)
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(operation).unwrap()
    );
}
