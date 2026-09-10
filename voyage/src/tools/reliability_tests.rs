//! Focused offline checks for tool outcome/patch contracts; no provider calls.
use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use voyage_protocol::tool_result::{CommandOutcome, ExecutionOutcome, IncompleteReason};

pub(crate) fn context(root: &std::path::Path) -> ToolContext {
    let config = crate::Config {
        access: Some(AccessMode::Unrestricted),
        ..Default::default()
    };
    ToolContext {
        artifact_scope: None,
        github: None,
        completion: None,
        policy: Arc::new(Policy::new(&config, root.to_path_buf()).unwrap()),
        approver: Arc::new(UnattendedApprover { allow: false }),
        timeout: Duration::from_secs(3),
        max_output_bytes: 4096,
        environment: Default::default(),
        cancellation: Default::default(),
        execution_id: uuid::Uuid::new_v4(),
        interaction: InteractionMode::Unattended,
        redactor: Arc::new(Redactor::default()),
    }
}
#[tokio::test]
async fn observed_shell_status_and_capture_limits() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = context(root.path());
    let mut registry = ToolRegistry::default();
    let shell = ManagedShell::new();
    registry.register(shell.clone());
    for (command, code) in [
        ("printf hello", Some(0)),
        ("exit 7", Some(7)),
        ("kill -TERM $$", None),
    ] {
        let report = registry
            .execute_report_with_workflow_secrets("shell", json!({"command":command}), &ctx, None)
            .await
            .unwrap();
        assert_eq!(report.outcome.execution, ExecutionOutcome::Succeeded);
        assert_eq!(
            report.outcome.command,
            Some(
                code.map_or(CommandOutcome::Signalled, |code| CommandOutcome::Exited {
                    code
                })
            )
        );
        assert_eq!(report.output.is_error, code != Some(0));
    }
    ctx.max_output_bytes = 128;
    let report = registry
        .execute_report_with_workflow_secrets(
            "shell",
            json!({"command":"printf '%0200d' 0; exit 3"}),
            &ctx,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        report.outcome.command,
        Some(CommandOutcome::Exited { code: 3 })
    );
    assert!(report.outcome.incomplete.is_some());
    assert!(report.output.text_fallback().len() <= 128);
    assert!(
        shell
            .shutdown(Duration::from_secs(3))
            .await
            .observation_complete
    );
}

struct Counting(Arc<AtomicUsize>);
#[async_trait]
impl Tool for Counting {
    fn definition(&self) -> crate::model::ToolDefinition {
        crate::model::ToolDefinition {
            name: "counting".into(),
            description: "fixture".into(),
            input_schema: json!({"type":"object"}),
            output_schema: None,
            annotations: None,
        }
    }
    async fn execute(&self, _: Value, _: &ToolContext) -> Result<String, ToolError> {
        panic!("typed implementation must dispatch once")
    }
    async fn execute_report(&self, _: Value, _: &ToolContext) -> Result<ToolReport, ToolError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ToolReport::command("x".repeat(8192), Some(9), false))
    }
}
#[tokio::test]
async fn budget_keeps_facts_without_replaying_effects() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path());
    let count = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::default();
    registry.register(Counting(count.clone()));
    let report = registry
        .execute_report_with_workflow_secrets("counting", json!({}), &ctx, None)
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        report.outcome.command,
        Some(CommandOutcome::Exited { code: 9 })
    );
    assert_eq!(
        report.outcome.incomplete,
        Some(IncompleteReason::OutputLimit)
    );
    assert!(report.output.is_error);
}
#[test]
fn refusal_unknown_and_legacy_persistence() {
    for (error, expected) in [
        (
            ToolError::Denied("fixture".into()),
            ExecutionOutcome::PolicyRefused,
        ),
        (
            ToolError::Timeout(Duration::from_secs(1)),
            ExecutionOutcome::Unknown,
        ),
        (ToolError::Cancelled, ExecutionOutcome::Cancelled),
        (
            ToolError::Failed("fixture".into()),
            ExecutionOutcome::ExecutionError,
        ),
    ] {
        let report = ToolReport::error(error);
        assert_eq!(report.outcome.execution, expected);
        let mut message =
            crate::model::Message::tool_result("stable-call", report.output.text_fallback(), false);
        message.tool_outcome = Some(report.outcome.clone());
        message.tool_output = Some(Box::new(report.output));
        let saved = serde_json::to_vec(&message).unwrap();
        let loaded: crate::model::Message = serde_json::from_slice(&saved).unwrap();
        assert_eq!(loaded.tool_outcome, Some(report.outcome));
        assert_eq!(serde_json::to_vec(&loaded).unwrap(), saved);
    }
    let legacy = crate::model::Message::tool_result("old", "unchanged", true);
    let saved = serde_json::to_value(&legacy).unwrap();
    assert!(saved.get("tool_outcome").is_none());
    let loaded: crate::model::Message = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(serde_json::to_value(loaded).unwrap(), saved);
}
#[tokio::test]
async fn malformed_patch_does_not_write_and_has_distinct_diagnostics() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path());
    let path = root.path().join("file");
    std::fs::write(&path, "before\n").unwrap();
    let hash = hex::encode(Sha256::digest(b"before\n"));
    for patch in [
        "not a unified diff",
        "--- a/file\n+++ b/file\n",
        "```diff\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-before\n+after\n```\n",
    ] {
        let absent = root.path().join("must-not-exist");
        let error = ApplyPatch
            .execute(json!({"path":absent,"patch":patch}), &ctx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no target file was written"));
        assert!(!absent.exists());
    }
    let malformed = "--- a/file\n+++ b/file\n@@ -1,2 +1,1 @@\n-before\n+after\n";
    let error = ApplyPatch
        .execute(
            json!({"path":path,"base_sha256":hash,"patch":malformed}),
            &ctx,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("no target file was written"));
    assert!(error.contains("hunk counts"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "before\n");
    let error = ApplyPatch
        .execute(
            json!({"path":path,"base_sha256":"0".repeat(64),"patch":malformed}),
            &ctx,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("file changed since it was read"));
    let patch = "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-before\n+after\n";
    ApplyPatch
        .execute(json!({"path":path,"base_sha256":hash,"patch":patch}), &ctx)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "after\n");
}

#[test]
fn canonical_journal_reopen_preserves_outcomes_and_fences_old_writers() {
    use crate::{
        attachment::journal::{Journal, RunState, TurnAdmission},
        model::{Message, Role, Usage},
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let mut journal = Journal::open(path.clone()).unwrap();
    let session = crate::session::Session::new(root.path().to_path_buf(), "fixture".into());
    journal.create_session(&session).unwrap();
    drop(journal);
    // Synthetic old-version journal, not any user's storage.
    let conn = rusqlite::Connection::open(path.join("journal.sqlite3")).unwrap();
    conn.execute("UPDATE attachment_schema SET version=10", [])
        .unwrap();
    drop(conn);
    let mut old_observer = Journal::open(path.clone()).unwrap();
    let mut journal = Journal::open(path.clone()).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let request = TurnAdmission {
        operator_name: None,
        command_id: uuid::Uuid::new_v4(),
        machine_id: uuid::Uuid::new_v4(),
        principal_id: uuid::Uuid::new_v4(),
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: now + 60000,
        prompt: "fixture".into(),
        parts: vec![],
    };
    let run = journal.admit_turn(&guard, &request, now).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let mut messages = journal.load_session(session.id).unwrap().session.messages;
    let accepted = serde_json::to_vec(&messages).unwrap();
    let mut call = Message::new(Role::Assistant, "");
    call.tool_calls.push(crate::model::ToolCall {
        id: "call".into(),
        name: "shell".into(),
        arguments: json!({"command":"exit 7"}),
    });
    messages.push(call);
    let report = ToolReport::command("exit: 7\nstdout:\n\nstderr:\n".into(), Some(7), false);
    let mut message = Message::tool_result("call", report.output.text_fallback(), false);
    message.tool_outcome = Some(report.outcome);
    message.tool_output = Some(Box::new(report.output));
    messages.push(message);
    journal
        .checkpoint_canonical(&guard, run.id, &messages, &Usage::default())
        .unwrap();
    assert_eq!(serde_json::to_vec(&messages[..1]).unwrap(), accepted);
    assert!(
        old_observer
            .create_session(&crate::session::Session::new(
                root.path().to_path_buf(),
                "fixture".into()
            ))
            .is_err()
    );
    journal
        .finish(&guard, run.id, RunState::Completed, None, Some("fixture"))
        .unwrap();
    drop(guard);
    drop(journal);
    let journal = Journal::open(path).unwrap();
    let saved = journal.load_session(session.id).unwrap().session;
    assert_eq!(
        serde_json::to_vec(&saved.messages[..messages.len()]).unwrap(),
        serde_json::to_vec(&messages).unwrap()
    );
    // A build-only parser dependency must not alter runtime canonical key order.
    assert_eq!(json!({"z":1,"a":2}).to_string(), "{\"a\":2,\"z\":1}");
}
#[tokio::test]
async fn secret_shell_reports_only_fixed_status_and_exit() {
    use crate::workflow::{Document, secrets::SecretInputs};
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path());
    let document:Document=serde_json::from_value(json!({"schema_version":1,"id":"fixture","version":"1","description":"fixture","prompt":"fixture","parameters":{"value":{"type":"string","secret":true}}})).unwrap();
    let bindings = SecretInputs::collect(
        &document,
        vec![("value".into(), "SYNTHETIC-PRIVATE-FIXTURE".into())],
    )
    .unwrap()
    .bind(ctx.execution_id)
    .unwrap();
    let mut registry = ToolRegistry::default();
    let shell = ManagedShell::new();
    registry.register(shell.clone());
    let report=registry.execute_report_with_workflow_secrets("shell",json!({"command":"printf '%s' \"$HELM_WORKFLOW_VALUE\"; exit 6","workflow_secrets":["value"]}),&ctx,Some(&bindings)).await.unwrap();
    assert_eq!(
        report.outcome.command,
        Some(CommandOutcome::Exited { code: 6 })
    );
    let text = report.output.text_fallback();
    assert!(!text.contains("SYNTHETIC"));
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({"status":"exited","code":6})
    );
    assert!(
        shell
            .shutdown(Duration::from_secs(3))
            .await
            .observation_complete
    );
}

#[tokio::test]
async fn retired_denials_allow_harmless_shell_output_and_preserve_access() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = context(root.path());
    let shell = ManagedShell::new();
    let mut registry = ToolRegistry::default();
    registry.register(shell.clone());
    let mut config = crate::Config {
        access: Some(AccessMode::Unrestricted),
        legacy_deny_commands: vec!["printf".into(), "shutdown".into()],
        ..Default::default()
    };
    ctx.policy = Arc::new(Policy::new(&config, root.path().into()).unwrap());
    // Execute only printf; never execute a shutdown, reboot or filesystem tool.
    let args = json!({"command": "printf '%s\\n' shutdown"});
    let report = registry
        .execute_report_with_workflow_secrets("shell", args.clone(), &ctx, None)
        .await
        .unwrap();
    assert!(!report.output.is_error);
    assert!(report.output.text_fallback().contains("shutdown"));
    for access in [AccessMode::ReadOnly, AccessMode::Approval] {
        config.access = Some(access);
        ctx.policy = Arc::new(Policy::new(&config, root.path().into()).unwrap());
        // An unavailable approval interface must not be treated as approval.
        let refused = registry
            .execute_report_with_workflow_secrets(
                "shell",
                json!({"command":"printf shutdown; printf safe"}),
                &ctx,
                None,
            )
            .await;
        assert!(refused.is_err());
    }
    assert!(
        shell
            .shutdown(Duration::from_secs(3))
            .await
            .observation_complete
    );
}
