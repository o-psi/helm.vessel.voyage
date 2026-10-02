//! Ordinary operator dispatch over an owned journal and scripted loopback peer.
//! No ambient credential, provider, remote GitHub effect, or process-global HOME.
use super::*;
use crate::github::{
    http_fixture::{Fixture, Reply},
    publication::{Action, Actor, Draft},
    service::Service,
    store::{Operation, Owner, State, Store},
};
use crate::tools::{InteractionMode, Redactor, ToolContext, UnattendedApprover};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const ISSUE: &str = "https://github.com/owned/project/issues/7";
const PULL: &str = "https://github.com/owned/project/pull/7";
const TOKEN: &str = "synthetic-operator-credential";
const BODY: &str = "Exact reviewed local fixture body";

fn context(root: &Path) -> ToolContext {
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        github_enabled: true,
        ..Default::default()
    };
    ToolContext {
        tool_call_id: None,
        artifact_scope: None,
        github: Some(crate::github::Credential(Arc::new(
            zeroize::Zeroizing::new(TOKEN.into()),
        ))),
        completion: None,
        policy: Arc::new(crate::policy::Policy::new(&config, root.into()).unwrap()),
        approver: Arc::new(UnattendedApprover { allow: true }),
        timeout: Duration::from_secs(2),
        max_output_bytes: 65536,
        environment: Default::default(),
        cancellation: CancellationToken::new(),
        execution_id: Uuid::new_v4(),
        interaction: InteractionMode::Attended,
        redactor: Arc::new(Redactor::new(["PRIVATE_FIXTURE_BODY".into()])),
    }
}
fn view(url: &str, section: SectionArg) -> ViewArgs {
    ViewArgs {
        url: url.into(),
        section,
        page: 1,
        after: None,
        resource: None,
        thread: None,
        head: None,
    }
}
fn prepare(url: &str) -> PrepareArgs {
    PrepareArgs {
        url: Some(url.into()),
        body: Some(BODY.into()),
        body_file: None,
        draft_file: None,
        event: None,
        commit: None,
    }
}
fn actor() -> Reply {
    Reply::json("/user", json!({"id":19,"login":"owned-actor"}))
}
fn issue() -> Reply {
    Reply::json(
        "/repos/owned/project/issues/7",
        json!({"number":7,"html_url":ISSUE,"body":"Current issue body","user":{"login":"author","id":23}}),
    )
}
fn pull() -> Reply {
    Reply::json(
        "/repos/owned/project/pulls/7",
        json!({"number":7,"html_url":PULL,"state":"open","head":{"sha":"a".repeat(40)},"base":{"sha":"b".repeat(40),"repo":{"id":42},"ref":"main"}}),
    )
}
fn comments() -> Reply {
    Reply::json(
        "/repos/owned/project/issues/7/comments?per_page=100&page=1",
        json!([
            {"id":31,"html_url":format!("{ISSUE}#issuecomment-31"),"body":"First suggestion","user":{"login":"reviewer","id":41}},
            {"id":32,"html_url":format!("{ISSUE}#issuecomment-32"),"body":"Selected PRIVATE_FIXTURE_BODY suggestion","user":{"login":"reviewer","id":41}}
        ]),
    )
}
async fn dispatch(
    context: ToolContext,
    session: Option<Uuid>,
    command: Command,
    directory: &Path,
    peer: &Fixture,
) -> Result<CommandResult> {
    let client = peer.client();
    let path = directory.to_owned();
    execute_args_with(
        context,
        session,
        Args { command },
        Some(path.clone()),
        move |context, session| Service::for_operator_test(context, session, client, path),
    )
    .await
}
async fn local(
    context: ToolContext,
    session: Option<Uuid>,
    command: Command,
    directory: &Path,
) -> Result<CommandResult> {
    execute_args_with(
        context,
        session,
        Args { command },
        Some(directory.into()),
        |_, _| panic!("offline administration must not create a remote service"),
    )
    .await
}
fn value(result: CommandResult) -> Value {
    assert!(result.reference.is_none() && result.feedback.is_none());
    serde_json::from_str(&result.display).unwrap()
}
fn saved(directory: &Path, context: &ToolContext, session: Option<Uuid>, body: &str) -> Operation {
    Store::open(directory.into())
        .unwrap()
        .prepare(
            Draft {
                object: Object::parse(ISSUE).unwrap(),
                action: Action::Comment { body: body.into() },
            },
            Actor {
                id: 19,
                login: "owned-actor".into(),
            },
            context.policy.effective().digest().into(),
            None,
            None,
            Owner::new(context.policy.workspace(), session, None).unwrap(),
        )
        .unwrap()
}

#[tokio::test]
async fn auth_reports_explicit_actor_without_credential_projection() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![actor()]).await;
    let result = dispatch(
        context(root.path()),
        None,
        Command::Auth,
        &root.path().join("journal"),
        &peer,
    )
    .await
    .unwrap();
    assert!(!result.display.contains(TOKEN));
    let output = value(result);
    assert_eq!(output["actor"]["id"], 19);
    assert_eq!(output["actor"]["login"], "owned-actor");
    assert_eq!(output["host"], "github.com");
    assert_eq!(peer.finish().await.len(), 1);
    assert!(!root.path().join("journal").exists());
}

#[tokio::test]
async fn ordinary_view_and_exact_continuation_observe_the_requested_issue() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![issue(), issue()]).await;
    let path = root.path().join("journal");
    let output = value(
        dispatch(
            context(root.path()),
            None,
            Command::View(view(ISSUE, SectionArg::Details)),
            &path,
            &peer,
        )
        .await
        .unwrap(),
    );
    assert_eq!(output["data"]["number"], 7);
    assert_eq!(output["data"]["body"], "Current issue body");
    let continuation =
        serde_json::to_string(&view(ISSUE, SectionArg::Details).read().unwrap()).unwrap();
    let output = value(
        dispatch(
            context(root.path()),
            None,
            Command::Continue {
                request: continuation,
            },
            &path,
            &peer,
        )
        .await
        .unwrap(),
    );
    assert_eq!(output["data"]["html_url"], ISSUE);
    assert_eq!(peer.finish().await.len(), 2);
    assert!(!path.exists());
}

#[tokio::test]
async fn reference_dispatch_returns_owning_frontend_work_with_observed_pull_head() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![pull()]).await;
    let path = root.path().join("journal");
    let result = dispatch(
        context(root.path()),
        Some(Uuid::new_v4()),
        Command::Reference { url: PULL.into() },
        &path,
        &peer,
    )
    .await
    .unwrap();
    let reference = result.reference.unwrap();
    reference.validate().unwrap();
    assert_eq!(reference.object.url(), PULL);
    assert_eq!(reference.head, Some("a".repeat(40)));
    assert!(result.feedback.is_none());
    assert_eq!(peer.finish().await.len(), 1);
    assert!(!path.exists());
}

#[tokio::test]
async fn feedback_dispatch_selects_exact_comment_and_preserves_redacted_provenance() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![issue(), comments()]).await;
    let result = dispatch(
        context(root.path()),
        Some(Uuid::new_v4()),
        Command::Feedback {
            view: view(ISSUE, SectionArg::Comments),
            id: Some(32),
        },
        &root.path().join("journal"),
        &peer,
    )
    .await
    .unwrap();
    let feedback = result.feedback.unwrap();
    assert_eq!(feedback.source, format!("{ISSUE}#issuecomment-32"));
    assert!(feedback.title.ends_with("feedback 32"));
    assert!(feedback.description.contains("reviewer (GitHub user 41)"));
    assert!(
        feedback
            .description
            .contains("Selected [REDACTED] suggestion")
    );
    assert!(!result.display.contains("PRIVATE_FIXTURE_BODY"));
    assert!(!feedback.description.contains("First suggestion"));
    assert_eq!(result.reference.unwrap().object.url(), ISSUE);
    assert_eq!(peer.finish().await.len(), 2);
}

#[tokio::test]
async fn issue_body_feedback_dispatch_keeps_observation_and_original_author() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![issue()]).await;
    let result = dispatch(
        context(root.path()),
        Some(Uuid::new_v4()),
        Command::Feedback {
            view: view(ISSUE, SectionArg::Details),
            id: None,
        },
        &root.path().join("journal"),
        &peer,
    )
    .await
    .unwrap();
    let feedback = result.feedback.unwrap();
    assert_eq!(feedback.source, ISSUE);
    assert!(feedback.description.contains("author (GitHub user 23)"));
    assert!(feedback.description.ends_with("Current issue body"));
    assert!(feedback.description.contains("Observed:"));
    assert_eq!(peer.finish().await.len(), 1);
}

#[tokio::test]
async fn feedback_refuses_missing_selection_and_cross_secret_source() {
    for selected in [None, Some(99)] {
        let root = tempfile::tempdir().unwrap();
        let peer = Fixture::start(vec![issue(), comments()]).await;
        let error = dispatch(
            context(root.path()),
            Some(Uuid::new_v4()),
            Command::Feedback {
                view: view(ISSUE, SectionArg::Comments),
                id: selected,
            },
            &root.path().join("journal"),
            &peer,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains(if selected.is_none() {
            "needs --id"
        } else {
            "absent from this page"
        }));
        assert_eq!(peer.finish().await.len(), 2);
    }
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![issue()]).await;
    let mut context = context(root.path());
    context.redactor = Arc::new(Redactor::new(["owned/project".into()]));
    let error = dispatch(
        context,
        Some(Uuid::new_v4()),
        Command::Feedback {
            view: view(ISSUE, SectionArg::Details),
            id: None,
        },
        &root.path().join("journal"),
        &peer,
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("source contains a configured secret")
    );
    assert_eq!(peer.finish().await.len(), 1);
}

#[tokio::test]
async fn feedback_rejects_comment_identity_on_issue_body_after_exact_observation() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![issue()]).await;
    let error = dispatch(
        context(root.path()),
        Some(Uuid::new_v4()),
        Command::Feedback {
            view: view(ISSUE, SectionArg::Details),
            id: Some(31),
        },
        &root.path().join("journal"),
        &peer,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("body import does not take"));
    assert_eq!(peer.finish().await.len(), 1);
}

#[tokio::test]
async fn admission_refusals_do_not_connect_or_create_journals() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    for command in [
        Command::Reference { url: ISSUE.into() },
        Command::Feedback {
            view: view(ISSUE, SectionArg::Details),
            id: None,
        },
        Command::Continue {
            request: "x".repeat(16 * 1024 + 1),
        },
        Command::Continue {
            request: "PRIVATE_INVALID_JSON".into(),
        },
        Command::Logs {
            url: ISSUE.into(),
            job: 0,
        },
    ] {
        let peer = Fixture::start(vec![]).await;
        let error = dispatch(context(root.path()), None, command, &path, &peer)
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("PRIVATE_INVALID_JSON"));
        assert!(!error.contains("transport failed"));
        assert_eq!(peer.finish().await.len(), 0);
    }
    let peer = Fixture::start(vec![]).await;
    let error = dispatch(
        context(root.path()),
        Some(Uuid::new_v4()),
        Command::Feedback {
            view: view(PULL, SectionArg::Files),
            id: None,
        },
        &path,
        &peer,
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("select issue or review feedback")
    );
    peer.finish().await;
    assert!(!path.exists());
}

#[tokio::test]
async fn ordinary_prepare_file_preserves_exact_bytes_and_durable_session_owner() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("body.txt"), "résumé\nExact file body").unwrap();
    let peer = Fixture::start(vec![actor(), issue()]).await;
    let mut args = prepare(ISSUE);
    args.body = None;
    args.body_file = Some("body.txt".into());
    let session = Uuid::new_v4();
    let path = root.path().join("journal");
    let operation: Operation = serde_json::from_value(value(
        dispatch(
            context(root.path()),
            Some(session),
            Command::Prepare(args),
            &path,
            &peer,
        )
        .await
        .unwrap(),
    ))
    .unwrap();
    assert_eq!(operation.state, State::Prepared);
    assert_eq!(operation.owner.session, Some(session));
    assert_eq!(
        operation.draft.action,
        Action::Comment {
            body: "résumé\nExact file body".into()
        }
    );
    assert_eq!(
        Store::open(path)
            .unwrap()
            .inspect(operation.id, &operation.owner)
            .unwrap()
            .digest,
        operation.digest
    );
    assert_eq!(peer.finish().await.len(), 2);
}

#[tokio::test]
async fn typed_draft_file_and_all_review_events_bind_the_observed_commit() {
    for event in ["COMMENT", "APPROVE", "REQUEST_CHANGES"] {
        let root = tempfile::tempdir().unwrap();
        let peer = Fixture::start(vec![actor(), pull(), pull()]).await;
        let mut args = prepare(PULL);
        args.event = Some(event.into());
        args.commit = Some("a".repeat(40));
        let operation: Operation = serde_json::from_value(value(
            dispatch(
                context(root.path()),
                None,
                Command::Prepare(args),
                &root.path().join("journal"),
                &peer,
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(operation.draft.body()["event"], event);
        assert_eq!(operation.observed_head, Some("a".repeat(40)));
        assert_eq!(operation.observed_base.unwrap().reference, "main");
        assert_eq!(peer.finish().await.len(), 3);
    }
    let root = tempfile::tempdir().unwrap();
    let draft = Draft {
        object: Object::parse(ISSUE).unwrap(),
        action: Action::Comment { body: BODY.into() },
    };
    std::fs::write(
        root.path().join("draft.json"),
        serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    let args = PrepareArgs {
        url: None,
        body: None,
        body_file: None,
        draft_file: Some("draft.json".into()),
        event: None,
        commit: None,
    };
    let peer = Fixture::start(vec![actor(), issue()]).await;
    let operation: Operation = serde_json::from_value(value(
        dispatch(
            context(root.path()),
            None,
            Command::Prepare(args),
            &root.path().join("journal"),
            &peer,
        )
        .await
        .unwrap(),
    ))
    .unwrap();
    assert_eq!(operation.draft, draft);
    assert_eq!(peer.finish().await.len(), 2);
}

#[tokio::test]
async fn prepare_refuses_ambiguous_parameters_invalid_drafts_and_secrets_without_network() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("invalid.json"), "PRIVATE_BAD_DRAFT").unwrap();
    let mut cases = Vec::new();
    let mut args = prepare(ISSUE);
    args.url = None;
    cases.push(args);
    let mut args = prepare(ISSUE);
    args.body = None;
    cases.push(args);
    let mut args = prepare(ISSUE);
    args.body_file = Some("body.txt".into());
    cases.push(args);
    let mut args = prepare(PULL);
    args.event = Some("INVALID".into());
    cases.push(args);
    let mut args = prepare(PULL);
    args.event = Some("COMMENT".into());
    cases.push(args);
    let mut args = prepare(ISSUE);
    args.commit = Some("a".repeat(40));
    cases.push(args);
    let mut args = prepare(ISSUE);
    args.draft_file = Some("invalid.json".into());
    cases.push(args);
    cases.push(PrepareArgs {
        url: None,
        body: None,
        body_file: None,
        draft_file: Some("invalid.json".into()),
        event: None,
        commit: None,
    });
    let mut args = prepare(ISSUE);
    args.body = Some(TOKEN.into());
    cases.push(args);
    let mut args = prepare(ISSUE);
    args.body = Some("PRIVATE_FIXTURE_BODY".into());
    cases.push(args);
    for args in cases {
        let peer = Fixture::start(vec![]).await;
        let error = dispatch(
            context(root.path()),
            None,
            Command::Prepare(args),
            &root.path().join("journal"),
            &peer,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(!error.contains("transport failed"));
        assert!(!error.contains("PRIVATE_BAD_DRAFT"));
        assert!(!error.contains(TOKEN));
        assert_eq!(peer.finish().await.len(), 0);
    }
    assert!(!root.path().join("journal").exists());
}

#[tokio::test]
async fn offline_receipt_reads_and_cancellation_need_no_remote_service_or_token() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let mut context = context(root.path());
    context.github = None;
    let session = Uuid::new_v4();
    let operation = saved(&path, &context, Some(session), "body PRIVATE_FIXTURE_BODY");
    let listed = value(
        local(
            context.clone(),
            Some(session),
            Command::List { offset: 0 },
            &path,
        )
        .await
        .unwrap(),
    );
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], operation.id.to_string());
    let inspected = local(
        context.clone(),
        Some(session),
        Command::Inspect { id: operation.id },
        &path,
    )
    .await
    .unwrap();
    assert!(inspected.display.contains("[REDACTED]"));
    assert!(!inspected.display.contains("PRIVATE_FIXTURE_BODY"));
    assert!(
        local(
            context.clone(),
            Some(Uuid::new_v4()),
            Command::Inspect { id: operation.id },
            &path
        )
        .await
        .is_err()
    );
    let cancelled = value(
        local(
            context.clone(),
            Some(session),
            Command::Cancel {
                id: operation.id,
                digest: operation.digest.clone(),
            },
            &path,
        )
        .await
        .unwrap(),
    );
    assert_eq!(cancelled["state"], "cancelled");
    let forgotten = value(
        local(
            context.clone(),
            Some(session),
            Command::Forget {
                id: operation.id,
                digest: operation.digest.clone(),
            },
            &path,
        )
        .await
        .unwrap(),
    );
    assert_eq!(forgotten["forgotten"], operation.id.to_string());
    assert!(
        forgotten["notice"]
            .as_str()
            .unwrap()
            .contains("does not prove")
    );
    assert_eq!(
        value(
            local(context, Some(session), Command::List { offset: 0 }, &path)
                .await
                .unwrap()
        ),
        json!([])
    );
    assert_eq!(Store::open(path).unwrap().audit().unwrap().entries.len(), 1);
}

#[tokio::test]
async fn offline_read_only_refuses_mutation_and_exact_digest_mismatch_preserves_record() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let mut context = context(root.path());
    context.github = None;
    let operation = saved(&path, &context, None, BODY);
    let config = crate::Config {
        access: Some(crate::config::AccessMode::ReadOnly),
        ..Default::default()
    };
    let mut readonly = context.clone();
    readonly.policy = Arc::new(crate::policy::Policy::new(&config, root.path().into()).unwrap());
    assert_eq!(
        value(
            local(
                readonly.clone(),
                None,
                Command::Inspect { id: operation.id },
                &path
            )
            .await
            .unwrap()
        )["state"],
        "prepared"
    );
    for command in [
        Command::Cancel {
            id: operation.id,
            digest: operation.digest.clone(),
        },
        Command::Forget {
            id: operation.id,
            digest: operation.digest.clone(),
        },
    ] {
        assert!(
            local(readonly.clone(), None, command, &path)
                .await
                .unwrap_err()
                .to_string()
                .contains("read-only")
        );
    }
    assert!(
        local(
            context.clone(),
            None,
            Command::Cancel {
                id: operation.id,
                digest: "f".repeat(64)
            },
            &path
        )
        .await
        .is_err()
    );
    assert!(
        local(
            context,
            None,
            Command::Forget {
                id: operation.id,
                digest: operation.digest.clone()
            },
            &path
        )
        .await
        .is_err()
    );
    assert_eq!(
        Store::open(path)
            .unwrap()
            .inspect(operation.id, &operation.owner)
            .unwrap()
            .state,
        State::Prepared
    );
}

fn accepted_receipt() -> Value {
    json!({"id":51,"user":{"id":19},"body":BODY,"html_url":format!("{ISSUE}#issuecomment-51")})
}
#[tokio::test]
async fn operator_publication_sends_exact_body_once_and_records_actual_receipt() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let peer = Fixture::start(vec![
        actor(),
        issue(),
        actor(),
        issue(),
        actor(),
        issue(),
        Reply {
            method: "POST",
            status: 201,
            ..Reply::json("/repos/owned/project/issues/7/comments", accepted_receipt())
        },
    ])
    .await;
    let context = context(root.path());
    let prepared: Operation = serde_json::from_value(value(
        dispatch(
            context.clone(),
            None,
            Command::Prepare(prepare(ISSUE)),
            &path,
            &peer,
        )
        .await
        .unwrap(),
    ))
    .unwrap();
    let published: Operation = serde_json::from_value(value(
        dispatch(
            context.clone(),
            None,
            Command::Publish {
                id: prepared.id,
                digest: prepared.digest.clone(),
            },
            &path,
            &peer,
        )
        .await
        .unwrap(),
    ))
    .unwrap();
    assert_eq!(published.state, State::Published);
    assert_eq!(published.receipt.unwrap().id, 51);
    assert!(
        dispatch(
            context,
            None,
            Command::Publish {
                id: prepared.id,
                digest: prepared.digest
            },
            &path,
            &peer
        )
        .await
        .is_err()
    );
    let requests = peer.finish().await;
    assert_eq!(requests.len(), 7);
    assert_eq!(
        requests
            .iter()
            .filter(|bytes| bytes.starts_with(b"POST "))
            .count(),
        1
    );
    assert!(
        String::from_utf8_lossy(requests.last().unwrap())
            .ends_with(&json!({"body":BODY}).to_string())
    );
}

#[tokio::test]
async fn uncertain_publication_reconciles_candidate_identity_without_resending() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let context = context(root.path());
    let operation = saved(&path, &context, None, BODY);
    Store::open(path.clone())
        .unwrap()
        .begin_send(&operation)
        .unwrap();
    let peer = Fixture::start(vec![
        actor(),
        Reply::json(
            "/repos/owned/project/issues/comments/51",
            accepted_receipt(),
        ),
    ])
    .await;
    let recovered: Operation = serde_json::from_value(value(
        dispatch(
            context,
            None,
            Command::Reconcile {
                id: operation.id,
                digest: operation.digest,
                remote_id: 51,
            },
            &path,
            &peer,
        )
        .await
        .unwrap(),
    ))
    .unwrap();
    assert_eq!(recovered.state, State::Published);
    assert!(matches!(
        recovered.receipt.unwrap().evidence,
        crate::github::store::ReceiptEvidence::OperatorAdopted
    ));
    let requests = peer.finish().await;
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|bytes| bytes.starts_with(b"GET ")));
}

#[tokio::test]
async fn cancellation_and_current_authority_precede_any_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let context = context(root.path());
    context.cancellation.cancel();
    assert!(
        local(context, None, Command::List { offset: 0 }, &path)
            .await
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert!(!path.exists());
}

#[derive(Debug)]
struct Retired;
impl crate::policy::ExecutionAuthority for Retired {
    fn check(&self) -> Result<()> {
        anyhow::bail!("owned operator authority retired")
    }
}
#[tokio::test]
async fn revoked_operator_authority_refuses_offline_administration_before_opening_storage() {
    let root = tempfile::tempdir().unwrap();
    let mut context = context(root.path());
    context.policy = Arc::new(
        (*context.policy)
            .clone()
            .with_execution_authority(Arc::new(Retired)),
    );
    let path = root.path().join("journal");
    assert!(
        local(context, None, Command::List { offset: 0 }, &path)
            .await
            .unwrap_err()
            .to_string()
            .contains("retired")
    );
    assert!(!path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn public_default_journal_dispatch_is_isolated_and_its_child_is_reaped() {
    let root = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "github::operator::production_tests::owned_default_journal_child",
            "--nocapture",
        ])
        .env("HOME", root.path())
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("VOYAGE_OPERATOR_JOURNAL_CHILD_ROOT", root.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = child.spawn().unwrap();
    let status = match tokio::time::timeout(Duration::from_secs(15), child.wait()).await {
        Ok(status) => status.unwrap(),
        Err(_) => {
            child.kill().await.unwrap();
            let _ = child.wait().await.unwrap();
            panic!("owned journal child exceeded its deadline");
        }
    };
    assert!(status.success(), "owned default journal child failed");
    assert_eq!(
        std::fs::read(root.path().join("completion-witness")).unwrap(),
        b"public default dispatch observed"
    );
    assert!(
        root.path()
            .join("data/helm/github/journal.sqlite3")
            .is_file()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn owned_default_journal_child() {
    let Some(root) = std::env::var_os("VOYAGE_OPERATOR_JOURNAL_CHILD_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    assert_eq!(
        std::env::var_os("HOME"),
        Some(root.clone().into_os_string())
    );
    assert_eq!(
        std::env::var_os("XDG_DATA_HOME"),
        Some(root.join("data").into_os_string())
    );
    let mut context = context(&root);
    context.github = None;
    let output = value(
        execute_args(
            context,
            None,
            Args {
                command: Command::List { offset: 0 },
            },
        )
        .await
        .unwrap(),
    );
    assert_eq!(output, json!([]));
    std::fs::write(
        root.join("completion-witness"),
        b"public default dispatch observed",
    )
    .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn operator_remote_discovery_reports_local_candidates_without_selecting_a_fork() {
    async fn git(root: &Path, args: &[&str]) {
        let mut child = tokio::process::Command::new("git");
        child
            .args(args)
            .current_dir(root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = child.spawn().unwrap();
        let status = match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            Ok(status) => status.unwrap(),
            Err(_) => {
                child.kill().await.unwrap();
                let _ = child.wait().await.unwrap();
                panic!("owned Git fixture exceeded deadline");
            }
        };
        assert!(status.success());
    }
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]).await;
    git(
        root.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/owned/project.git",
        ],
    )
    .await;
    git(
        root.path(),
        &["remote", "add", "fork", "git@github.com:other/project.git"],
    )
    .await;
    let mut context = context(root.path());
    context.github = None;
    context
        .environment
        .insert("PATH".into(), "/usr/bin:/bin".into());
    let path = root.path().join("journal");
    let output = value(local(context, None, Command::Remotes, &path).await.unwrap());
    assert_eq!(output.as_array().unwrap().len(), 2);
    assert!(
        output
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["remote"] == "origin"
                && candidate["repository"]["owner"] == "owned")
    );
    assert!(
        output
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["remote"] == "fork"
                && candidate["repository"]["owner"] == "other")
    );
    assert!(
        output
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| candidate["unavailable"] == false)
    );
    assert!(!path.exists());
}
