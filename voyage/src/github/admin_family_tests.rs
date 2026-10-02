//! Attended local journal maintenance; no remote client, token or provider.
use super::*;
use crate::{
    config::AccessMode,
    github::{
        publication::{Action, Actor, Draft},
        repository::Object,
        store::{Operation, Receipt, ReceiptEvidence, State, Store},
    },
    tools::{ApprovalRequest, Approver, Redactor},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Decision {
    outcome: ApprovalOutcome,
    requests: Mutex<Vec<ApprovalRequest>>,
    effect: Option<Box<dyn Fn() + Send + Sync>>,
}
#[async_trait::async_trait]
impl Approver for Decision {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        self.requests.lock().unwrap().push(request.clone());
        if let Some(effect) = &self.effect {
            effect();
        }
        self.outcome.clone()
    }
}
#[derive(Debug)]
struct Authority(AtomicBool);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> Result<()> {
        ensure!(self.0.load(Ordering::Acquire), "owned authority retired");
        Ok(())
    }
}
struct Fixture {
    root: tempfile::TempDir,
    directory: std::path::PathBuf,
    owner: Owner,
    operation: Operation,
}
impl Fixture {
    fn new(body: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("local-journal");
        let owner = Owner::new(root.path(), Some(Uuid::new_v4()), Some(Uuid::new_v4())).unwrap();
        let operation = Store::open(directory.clone())
            .unwrap()
            .prepare(
                Draft {
                    object: Object::parse("https://github.com/owned/fixture/issues/7").unwrap(),
                    action: Action::Comment { body: body.into() },
                },
                Actor {
                    id: 7,
                    login: "owned-local-actor".into(),
                },
                "owned-policy".into(),
                None,
                None,
                owner.clone(),
            )
            .unwrap();
        Self {
            root,
            directory,
            owner,
            operation,
        }
    }
    fn context(&self, outcome: ApprovalOutcome) -> (ToolContext, Arc<Decision>) {
        let decision = Arc::new(Decision {
            outcome,
            requests: Mutex::new(Vec::new()),
            effect: None,
        });
        (self.context_with(decision.clone()), decision)
    }
    fn context_with(&self, decision: Arc<Decision>) -> ToolContext {
        let config = crate::Config {
            access: Some(AccessMode::Unrestricted),
            github_enabled: false,
            ..Default::default()
        };
        ToolContext {
            tool_call_id: None,
            artifact_scope: None,
            github: None,
            completion: None,
            policy: Arc::new(crate::policy::Policy::new(&config, self.root.path().into()).unwrap()),
            approver: decision,
            timeout: std::time::Duration::from_secs(2),
            max_output_bytes: 65536,
            environment: Default::default(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: Uuid::new_v4(),
            interaction: InteractionMode::Attended,
            redactor: Arc::new(Redactor::new(Vec::<String>::new())),
        }
    }
    fn store(&self) -> Store {
        Store::open(self.directory.clone()).unwrap()
    }
    fn saved(&self) -> Value {
        serde_json::to_value(self.store().admin_inspect(self.operation.id).unwrap()).unwrap()
    }
    async fn execute(
        &self,
        context: ToolContext,
        command: Command,
        scoped: Option<Owner>,
    ) -> Result<CommandResult> {
        execute_inner(context, command, scoped, self.directory.clone()).await
    }
    fn cancel(&self) -> Command {
        Command::Cancel {
            id: self.operation.id,
            digest: self.operation.digest.clone(),
        }
    }
}

#[tokio::test]
async fn local_reads_need_no_token_approval_or_write_authority_and_scope_inspection() {
    let f = Fixture::new("owned local content");
    let (mut context, decision) = f.context(ApprovalOutcome::Denied);
    let config = crate::Config {
        access: Some(AccessMode::ReadOnly),
        github_enabled: false,
        ..Default::default()
    };
    context.policy = Arc::new(crate::policy::Policy::new(&config, f.root.path().into()).unwrap());
    for command in [
        Command::List { offset: 0 },
        Command::Inspect { id: f.operation.id },
        Command::Audit,
    ] {
        let result = f
            .execute(context.clone(), command, Some(f.owner.clone()))
            .await
            .unwrap();
        assert!(serde_json::from_str::<Value>(&result.display).is_ok());
        assert!(result.reference.is_none() && result.feedback.is_none());
    }
    let other = Owner::new(f.root.path(), Some(Uuid::new_v4()), None).unwrap();
    assert!(
        f.execute(
            context.clone(),
            Command::Inspect { id: f.operation.id },
            Some(other)
        )
        .await
        .is_err()
    );
    assert!(
        f.execute(
            context.clone(),
            Command::Inspect { id: Uuid::new_v4() },
            None
        )
        .await
        .is_err()
    );
    let empty = f
        .execute(context, Command::List { offset: 512 }, None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&empty.display).unwrap(),
        serde_json::json!([])
    );
    assert!(decision.requests.lock().unwrap().is_empty());
    assert_eq!(
        f.store().admin_inspect(f.operation.id).unwrap().state,
        State::Prepared
    );
}

#[tokio::test]
async fn admission_refuses_unattended_cancelled_and_retired_authority_before_storage() {
    for case in 0..3 {
        let f = Fixture::new("preserved");
        let (mut context, decision) = f.context(ApprovalOutcome::Approved);
        match case {
            0 => context.interaction = InteractionMode::Unattended,
            1 => context.cancellation.cancel(),
            _ => {
                context.policy = Arc::new(
                    context
                        .policy
                        .as_ref()
                        .clone()
                        .with_execution_authority(Arc::new(Authority(AtomicBool::new(false)))),
                )
            }
        }
        let absent = f.root.path().join("must-not-exist");
        assert!(
            execute(context.clone(), Command::Audit, None)
                .await
                .is_err()
        );
        assert!(
            execute_inner(context, Command::Audit, None, absent.clone())
                .await
                .is_err()
        );
        assert!(!absent.exists());
        assert!(decision.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn unavailable_owned_store_is_an_error_without_private_diagnostics_or_approval() {
    let f = Fixture::new("preserved original evidence");
    let before = f.saved();
    let (context, decision) = f.context(ApprovalOutcome::Approved);
    let blocked = f.root.path().join("private-fixture-diagnostic");
    std::fs::write(&blocked, b"owned non-directory sentinel").unwrap();
    let error = execute_inner(context, Command::Audit, None, blocked.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("store unavailable"));
    assert!(!error.contains("private-fixture-diagnostic"));
    assert_eq!(
        std::fs::read(blocked).unwrap(),
        b"owned non-directory sentinel"
    );
    assert_eq!(f.saved(), before);
    assert!(decision.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn every_local_maintenance_route_approves_one_exact_snapshot_and_preserves_uncertainty() {
    for kind in 0..5 {
        let f = Fixture::new("exact approved body");
        let command = match kind {
            0 => f.cancel(),
            1 => {
                f.store().begin_send(&f.operation).unwrap();
                Command::Dispose {
                    id: f.operation.id,
                    digest: f.operation.digest.clone(),
                    note: "explicit unknown outcome; never replay".into(),
                }
            }
            2 => {
                f.store().cancel_exact(&f.operation).unwrap();
                Command::Forget {
                    id: f.operation.id,
                    digest: f.operation.digest.clone(),
                }
            }
            3 => {
                let sending = f.store().begin_send(&f.operation).unwrap();
                f.store()
                    .dispose_exact(&sending, "retained unknown effect".into())
                    .unwrap();
                Command::Forget {
                    id: f.operation.id,
                    digest: f.operation.digest.clone(),
                }
            }
            _ => {
                let sending = f.store().begin_send(&f.operation).unwrap();
                f.store()
                    .publish(
                        &sending,
                        Receipt {
                            id: 19,
                            url: format!("{}#issuecomment-19", sending.draft.object.url()),
                            evidence: ReceiptEvidence::ApiResponse,
                        },
                    )
                    .unwrap();
                Command::Forget {
                    id: f.operation.id,
                    digest: f.operation.digest.clone(),
                }
            }
        };
        let before = f.saved();
        let (context, decision) = f.context(ApprovalOutcome::Approved);
        let execution = context.execution_id;
        let result = f.execute(context, command, None).await.unwrap();
        let output: Value = serde_json::from_str(&result.display).unwrap();
        let requests = decision.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].execution_id, execution);
        assert_eq!(requests[0].action, "github.dispose");
        assert_eq!(requests[0].mode, InteractionMode::Attended);
        let preview: Value = serde_json::from_str(&requests[0].reason).unwrap();
        assert_eq!(preview["operation"], before);
        assert!(
            preview["notice"]
                .as_str()
                .unwrap()
                .contains("never authorizes repetition")
        );
        if kind < 2 {
            let saved = f.store().admin_inspect(f.operation.id).unwrap();
            assert_eq!(
                saved.state,
                if kind == 0 {
                    State::Cancelled
                } else {
                    State::Disposed
                }
            );
            assert_eq!(saved.digest, f.operation.digest);
            assert_eq!(saved.owner, f.owner);
            assert!(f.store().begin_send(&saved).is_err());
            assert_eq!(
                output["state"],
                if kind == 0 { "cancelled" } else { "disposed" }
            );
            if kind == 1 {
                assert!(saved.receipt.is_none());
                assert!(saved.disposition.unwrap().contains("unknown outcome"));
            }
        } else {
            assert_eq!(output["forgotten"], f.operation.id.to_string());
            assert_eq!(output["audit_recorded"], true);
            assert!(f.store().admin_inspect(f.operation.id).is_err());
            let audit = f.store().audit().unwrap();
            assert_eq!(audit.entries.len(), 1);
            assert_eq!(audit.entries[0].id, f.operation.id);
            assert_eq!(
                audit.entries[0].former_state,
                match kind {
                    2 => State::Cancelled,
                    3 => State::Disposed,
                    _ => State::Published,
                }
            );
            assert_eq!(audit.entries[0].receipt.is_some(), kind == 4);
        }
    }
}

#[tokio::test]
async fn digest_and_state_refusals_do_not_request_approval_or_change_evidence() {
    for case in 0..4 {
        let f = Fixture::new("immutable");
        let command = match case {
            0 => Command::Cancel {
                id: f.operation.id,
                digest: "wrong".into(),
            },
            1 => Command::Dispose {
                id: f.operation.id,
                digest: f.operation.digest.clone(),
                note: "cannot dispose unsent".into(),
            },
            2 => Command::Forget {
                id: f.operation.id,
                digest: f.operation.digest.clone(),
            },
            _ => {
                f.store().begin_send(&f.operation).unwrap();
                f.cancel()
            }
        };
        let before = f.saved();
        let (context, decision) = f.context(ApprovalOutcome::Approved);
        assert!(f.execute(context, command, None).await.is_err());
        assert_eq!(f.saved(), before);
        assert!(f.store().audit().unwrap().entries.is_empty());
        assert!(decision.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn all_nonapproved_answers_and_read_only_maintenance_preserve_original_record() {
    for answer in [
        ApprovalOutcome::Denied,
        ApprovalOutcome::Expired,
        ApprovalOutcome::Cancelled,
        ApprovalOutcome::Invalidated,
        ApprovalOutcome::Unavailable,
    ] {
        let f = Fixture::new("retain exact prepared state");
        let before = f.saved();
        let (context, decision) = f.context(answer);
        assert!(
            f.execute(context, f.cancel(), Some(f.owner.clone()))
                .await
                .is_err()
        );
        assert_eq!(decision.requests.lock().unwrap().len(), 1);
        assert_eq!(f.saved(), before);
    }
    let f = Fixture::new("retain read-only state");
    let before = f.saved();
    let (mut context, decision) = f.context(ApprovalOutcome::Approved);
    let config = crate::Config {
        access: Some(AccessMode::ReadOnly),
        ..Default::default()
    };
    context.policy = Arc::new(crate::policy::Policy::new(&config, f.root.path().into()).unwrap());
    assert!(f.execute(context, f.cancel(), None).await.is_err());
    assert!(decision.requests.lock().unwrap().is_empty());
    assert_eq!(f.saved(), before);
}

#[tokio::test]
async fn approval_rechecks_cancellation_authority_and_exact_snapshot_before_mutation() {
    for case in 0..3 {
        let f = Fixture::new("one prepared record");
        let authority = Arc::new(Authority(AtomicBool::new(true)));
        let cancel = tokio_util::sync::CancellationToken::new();
        let (directory, operation, retired, cancelled) = (
            f.directory.clone(),
            f.operation.clone(),
            authority.clone(),
            cancel.clone(),
        );
        let decision = Arc::new(Decision {
            outcome: ApprovalOutcome::Approved,
            requests: Mutex::new(Vec::new()),
            effect: Some(Box::new(move || match case {
                0 => cancelled.cancel(),
                1 => retired.0.store(false, Ordering::Release),
                _ => {
                    Store::open(directory.clone())
                        .unwrap()
                        .begin_send(&operation)
                        .unwrap();
                }
            })),
        });
        let mut context = f.context_with(decision.clone());
        context.cancellation = cancel;
        context.policy = Arc::new(
            context
                .policy
                .as_ref()
                .clone()
                .with_execution_authority(authority),
        );
        assert!(f.execute(context, f.cancel(), None).await.is_err());
        assert_eq!(decision.requests.lock().unwrap().len(), 1);
        let retained = f.store().admin_inspect(f.operation.id).unwrap();
        assert_eq!(
            retained.state,
            if case == 2 {
                State::Sending
            } else {
                State::Prepared
            }
        );
        assert_eq!(retained.digest, f.operation.digest);
        assert!(retained.disposition.is_none() && retained.receipt.is_none());
        assert!(f.store().audit().unwrap().entries.is_empty());
    }
}

#[tokio::test]
async fn secret_previews_are_refused_before_approval_and_output_reads_are_redacted() {
    let secret = "owned-confidential-value";
    let f = Fixture::new(secret);
    let before = f.saved();
    let (mut context, decision) = f.context(ApprovalOutcome::Approved);
    context.redactor = Arc::new(Redactor::new(vec![secret.into()]));
    assert!(f.execute(context.clone(), f.cancel(), None).await.is_err());
    assert!(decision.requests.lock().unwrap().is_empty());
    assert_eq!(f.saved(), before);
    let read = f
        .execute(context, Command::Inspect { id: f.operation.id }, None)
        .await
        .unwrap();
    assert!(!read.display.contains(secret));
    assert!(serde_json::from_str::<Value>(&read.display).is_ok());
}

#[tokio::test]
async fn audit_clear_requires_exact_export_approval_and_postawait_snapshot() {
    for case in 0..4 {
        let f = Fixture::new("forgotten only after audit");
        let cancelled = f.store().cancel_exact(&f.operation).unwrap();
        f.store().forget_exact(&cancelled).unwrap();
        let original = f.store().audit().unwrap();
        let digest = if case == 0 {
            "stale-export".into()
        } else {
            original.digest.clone()
        };
        let decision = Arc::new(Decision {
            outcome: if case == 1 {
                ApprovalOutcome::Denied
            } else {
                ApprovalOutcome::Approved
            },
            requests: Mutex::new(Vec::new()),
            effect: if case == 2 {
                let directory = f.directory.clone();
                let owner = f.owner.clone();
                Some(Box::new(move || {
                    let mut store = Store::open(directory.clone()).unwrap();
                    let extra = store
                        .prepare(
                            Draft {
                                object: Object::parse("https://github.com/owned/fixture/issues/7")
                                    .unwrap(),
                                action: Action::Comment {
                                    body: "concurrent local record".into(),
                                },
                            },
                            Actor {
                                id: 7,
                                login: "owned-local-actor".into(),
                            },
                            "owned-policy".into(),
                            None,
                            None,
                            owner.clone(),
                        )
                        .unwrap();
                    let cancelled = store.cancel_exact(&extra).unwrap();
                    store.forget_exact(&cancelled).unwrap();
                }) as Box<dyn Fn() + Send + Sync>)
            } else {
                None
            },
        });
        let context = f.context_with(decision.clone());
        let result = f
            .execute(context, Command::ClearAudit { digest }, None)
            .await;
        if case == 3 {
            assert_eq!(
                serde_json::from_str::<Value>(&result.unwrap().display).unwrap()["removed_audit_entries"],
                1
            );
            assert!(f.store().audit().unwrap().entries.is_empty());
        } else {
            assert!(result.is_err());
            let audit = f.store().audit().unwrap();
            assert_eq!(audit.entries.len(), if case == 2 { 2 } else { 1 });
            assert_eq!(audit.entries[0].id, f.operation.id);
        }
        let requests = decision.requests.lock().unwrap();
        assert_eq!(requests.len(), usize::from(case != 0));
        if let Some(request) = requests.first() {
            let preview: Value = serde_json::from_str(&request.reason).unwrap();
            assert_eq!(preview["digest"], original.digest);
            assert_eq!(preview["entries"], 1);
            assert!(
                preview["notice"]
                    .as_str()
                    .unwrap()
                    .contains("never authorizes another remote send")
            );
        }
    }
}
