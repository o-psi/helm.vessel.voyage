use super::*;
use crate::{
    agent::{AgentEvent, EventSink, SilentSink},
    config::{AccessMode, Config},
    model::{ModelRequest, ModelResponse, Role, ToolCall, ToolDefinition},
    policy::Policy,
    provider::{Provider, ProviderDelta, ProviderError, ProviderStream, ProviderStreamEvent},
    session::Session,
    tools::{
        InteractionMode, Redactor, Tool, ToolContext, ToolError, ToolRegistry, UnattendedApprover,
    },
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

// Fault injection uses the live Journal's journal mode. A second connection in
// DELETE mode cannot unlink the rollback file retained by Windows PERSIST.
fn fixture_database(path: impl AsRef<std::path::Path>) -> rusqlite::Result<rusqlite::Connection> {
    let db = rusqlite::Connection::open(path)?;
    #[cfg(windows)]
    db.pragma_update(None, "journal_mode", "PERSIST")?;
    Ok(db)
}

struct Fixture {
    db: PathBuf,
    requests: Arc<AtomicUsize>,
    mode: &'static str,
}
#[async_trait]
impl Provider for Fixture {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        assert_eq!(request.model, "fixture");
        let index = self.requests.fetch_add(1, Ordering::SeqCst);
        let db = fixture_database(&self.db).unwrap();
        let saved: String = db
            .query_row("SELECT state FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert!(saved.contains("accepted prompt"));
        assert!(!saved.contains("ephemeral guidance"));
        if self.mode == "hang" {
            return std::future::pending().await;
        }
        if self.mode == "provider-failure" {
            return Err(ProviderError::Request("failure sentinel".into()));
        }
        if self.mode == "before-tool-failure" && index == 0 {
            db.execute_batch("CREATE TRIGGER fail_checkpoint BEFORE UPDATE ON sessions BEGIN SELECT RAISE(ABORT,'storage unavailable'); END;").unwrap();
        }
        if self.mode == "final-storage-failure" && index == 1 {
            db.execute_batch("CREATE TRIGGER fail_terminal BEFORE UPDATE ON runs WHEN json_extract(NEW.record,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'terminal storage unavailable'); END;").unwrap();
        }
        let mut message = Message::new(
            Role::Assistant,
            if index == 0 {
                "tool intent"
            } else {
                "final answer"
            },
        );
        message.provider_state = Some(serde_json::json!({"local_only":"continuation-sentinel"}));
        if index == 0 {
            message.tool_calls.push(ToolCall {
                id: "call-1".into(),
                name: "counter".into(),
                arguments: serde_json::json!({}),
            });
        } else {
            assert!(
                request
                    .messages
                    .iter()
                    .any(|m| m.role == Role::Tool && m.content == "effect result")
            );
        }
        Ok(ModelResponse {
            message,
            usage: Usage {
                input_tokens: 3,
                output_tokens: 2,
            },
        })
    }
    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        if matches!(self.mode, "partial-failure" | "partial-storage-failure") {
            self.requests.fetch_add(1, Ordering::SeqCst);
            if self.mode == "partial-storage-failure" {
                fixture_database(&self.db).unwrap().execute_batch("CREATE TRIGGER fail_output BEFORE UPDATE ON runs BEGIN SELECT RAISE(ABORT,'cannot persist output'); END;").unwrap();
            }
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderStreamEvent::Delta(ProviderDelta::Text(
                    "saved partial".into(),
                ))),
                Err(ProviderError::Unavailable("not retried".into())),
            ])));
        }
        let response = self.complete(request).await?;
        if self.mode == "empty-delta" {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderStreamEvent::Delta(ProviderDelta::Text(
                    String::new(),
                ))),
                Ok(ProviderStreamEvent::Completed(response)),
            ])));
        }
        Ok(Box::pin(futures_util::stream::once(async {
            Ok(ProviderStreamEvent::Completed(response))
        })))
    }
}
struct Counter {
    db: PathBuf,
    effects: Arc<AtomicUsize>,
    fail_after: bool,
}
#[async_trait]
impl Tool for Counter {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "counter".into(),
            description: "fixture".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }
    }
    async fn execute(
        &self,
        _: serde_json::Value,
        context: &ToolContext,
    ) -> Result<String, ToolError> {
        let db = fixture_database(&self.db).unwrap();
        let (record, state): (String, String) = db
            .query_row(
                "SELECT record,state FROM runs JOIN sessions ON runs.session_id=sessions.id",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let run: RunRecord = serde_json::from_str(&record).unwrap();
        assert_eq!(run.id, context.execution_id);
        assert!(state.contains("call-1") && state.contains("continuation-sentinel"));
        self.effects.fetch_add(1, Ordering::SeqCst);
        if self.fail_after {
            db.execute_batch("CREATE TRIGGER fail_result BEFORE UPDATE ON sessions BEGIN SELECT RAISE(ABORT,'cannot checkpoint effect result'); END;").unwrap();
        }
        Ok("effect result".into())
    }
}
#[derive(Default)]
struct Observed(Mutex<Vec<AgentEvent>>);
#[async_trait]
impl EventSink for Observed {
    async fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

async fn setup(
    mode: &'static str,
    sink: Arc<dyn EventSink>,
) -> (
    tempfile::TempDir,
    RunOwner,
    Agent,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    TurnAdmission,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("attachment");
    let db = path.join("journal.sqlite3");
    // Persist a valid noncanonical root, as happens with macOS /var aliases
    // and Windows canonical verbatim paths. Policy uses its canonical identity.
    let nested = dir.path().join("workspace-alias");
    std::fs::create_dir(&nested).unwrap();
    let session = Session::new(nested.join(".."), "fixture".into());
    Journal::open(path.clone())
        .unwrap()
        .create_session(&session)
        .unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60_000,
        prompt: "accepted prompt".into(),
    };
    let duplicate = TurnAdmission {
        command_id: request.command_id,
        machine_id: request.machine_id,
        principal_id: request.principal_id,
        session_id: request.session_id,
        expected_revision: request.expected_revision,
        expires_at_ms: request.expires_at_ms,
        prompt: request.prompt.clone(),
    };
    let Admission::New(owner) = RunOwner::admit_at(path, request, 1).await.unwrap() else {
        panic!()
    };
    let requests = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::default();
    tools.register(Counter {
        db: db.clone(),
        effects: effects.clone(),
        fail_after: mode == "after-tool-failure",
    });
    let config = Config {
        access: Some(AccessMode::Unrestricted),
        ..Config::default()
    };
    let context = ToolContext {
        completion: None,
        policy: Arc::new(Policy::new(&config, dir.path().to_path_buf()).unwrap()),
        approver: Arc::new(UnattendedApprover { allow: false }),
        timeout: Duration::from_secs(3),
        max_output_bytes: 4096,
        environment: BTreeMap::new(),
        cancellation: CancellationToken::new(),
        execution_id: Uuid::new_v4(),
        interaction: InteractionMode::Unattended,
        redactor: Arc::new(Redactor::default()),
    };
    let agent = Agent::new(
        Box::new(Fixture {
            db,
            requests: requests.clone(),
            mode,
        }),
        tools,
        context,
        sink,
        "fixture".into(),
        "ephemeral guidance".into(),
        128,
        None,
    );
    (dir, owner, agent, requests, effects, duplicate)
}

#[tokio::test]
async fn canonical_tool_loop_is_durable_once_with_usage_and_local_provider_state() {
    let (dir, mut owner, agent, requests, effects, retry) =
        setup("success", Arc::new(SilentSink)).await;
    let outcome = owner
        .execute(&agent, CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(outcome.answer, "final answer");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    let record = owner.record().await.unwrap();
    assert_eq!(record.state, RunState::Completed);
    assert_eq!(record.usage.input_tokens, 6);
    assert!(
        owner
            .execute(&agent, CancellationToken::new(), None)
            .await
            .is_err()
    );
    assert!(
        Journal::open(dir.path().join("attachment"))
            .unwrap()
            .acquire_execution(retry.session_id)
            .is_err()
    );
    drop(owner);
    let Admission::Existing(existing) =
        RunOwner::admit_at(dir.path().join("attachment"), retry, 90_000)
            .await
            .unwrap()
    else {
        panic!("retry created execution")
    };
    assert_eq!(existing.id, record.id);
    let journal = Journal::open(dir.path().join("attachment")).unwrap();
    let stored = journal.load_session(record.session_id).unwrap().session;
    assert_eq!(stored.messages.len(), 4);
    assert_eq!(stored.messages[2].role, Role::Tool);
    assert!(stored.messages[1].provider_state.is_some());
    assert_eq!(stored.usage.input_tokens, 6);
    assert_eq!(stored.usage.output_tokens, 4);
}

#[tokio::test]
async fn storage_failure_before_or_after_effect_never_replays_or_accepts_success() {
    for (mode, expected_effects) in [("before-tool-failure", 0), ("after-tool-failure", 1)] {
        let (dir, mut owner, agent, requests, effects, retry) =
            setup(mode, Arc::new(SilentSink)).await;
        assert!(matches!(
            owner.execute(&agent, CancellationToken::new(), None).await,
            Err(AgentError::Checkpoint(_))
        ));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(effects.load(Ordering::SeqCst), expected_effects);
        assert_eq!(owner.record().await.unwrap().state, RunState::Running);
        drop(owner);
        let mut journal = Journal::open(dir.path().join("attachment")).unwrap();
        let db = fixture_database(dir.path().join("attachment/journal.sqlite3")).unwrap();
        db.execute_batch(
            "DROP TRIGGER IF EXISTS fail_checkpoint; DROP TRIGGER IF EXISTS fail_result;",
        )
        .unwrap();
        let guard = journal.acquire_execution(retry.session_id).unwrap();
        assert_eq!(
            journal.recover_interrupted(&guard).unwrap().unwrap().state,
            RunState::Interrupted
        );
        assert!(journal.admit_turn(&guard, &retry, 1).unwrap().duplicate);
        assert_eq!(effects.load(Ordering::SeqCst), expected_effects);
    }
}

#[tokio::test]
async fn partial_failure_is_preserved_without_retry_and_failed_storage_is_not_published() {
    for mode in ["partial-failure", "partial-storage-failure"] {
        let sink = Arc::new(Observed::default());
        let (_dir, mut owner, agent, requests, effects, _) = setup(mode, sink.clone()).await;
        assert!(
            owner
                .execute(&agent, CancellationToken::new(), None)
                .await
                .is_err()
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        let record = owner.record().await.unwrap();
        let advertised = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, AgentEvent::AssistantTextDelta(_)));
        if mode == "partial-failure" {
            assert_eq!(record.partial_text, "saved partial");
            assert_eq!(record.state, RunState::Failed);
            assert!(advertised);
        } else {
            assert_eq!(record.partial_text, "");
            assert_eq!(record.state, RunState::Running);
            assert!(!advertised);
        }
    }
}

#[tokio::test]
async fn cancellation_before_and_during_provider_wait_preserves_accepted_input() {
    for precancel in [false, true] {
        let (_dir, mut owner, agent, requests, effects, _) =
            setup("hang", Arc::new(SilentSink)).await;
        let cancel = CancellationToken::new();
        if precancel {
            cancel.cancel();
        } else {
            let token = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                token.cancel();
            });
        }
        assert!(matches!(
            owner.execute(&agent, cancel, None).await,
            Err(AgentError::Cancelled)
        ));
        assert_eq!(owner.record().await.unwrap().state, RunState::Cancelled);
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        if precancel {
            assert_eq!(requests.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn failed_final_commit_never_returns_success_or_repeats_final_text() {
    let (dir, mut owner, agent, requests, effects, retry) =
        setup("final-storage-failure", Arc::new(SilentSink)).await;
    assert!(matches!(
        owner.execute(&agent, CancellationToken::new(), None).await,
        Err(AgentError::Checkpoint(_))
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(owner.record().await.unwrap().state, RunState::Running);
    assert!(owner.record().await.unwrap().final_checkpointed);
    let stored = Journal::open(dir.path().join("attachment"))
        .unwrap()
        .load_session(retry.session_id)
        .unwrap()
        .session;
    assert_eq!(
        stored
            .messages
            .iter()
            .filter(|m| m.content == "final answer")
            .count(),
        1
    );
}

#[tokio::test]
async fn corrupt_storage_and_wrong_local_model_prevent_provider_dispatch() {
    for corrupt in [false, true] {
        let (dir, mut owner, agent, requests, effects, _) =
            setup("success", Arc::new(SilentSink)).await;
        if corrupt {
            fixture_database(dir.path().join("attachment/journal.sqlite3"))
                .unwrap()
                .execute("UPDATE sessions SET state='corrupt'", [])
                .unwrap();
        } else {
            agent.set_model("wrong local model").unwrap();
        }
        assert!(
            owner
                .execute(&agent, CancellationToken::new(), None)
                .await
                .is_err()
        );
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert!(
            owner
                .execute(&agent, CancellationToken::new(), None)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn steering_is_checkpointed_in_fifo_order_before_provider_dispatch() {
    let (dir, mut owner, agent, _, _, retry) = setup("success", Arc::new(SilentSink)).await;
    let sender = owner
        .enable_steering_with_clock(Arc::new(steering::allow_actor), Arc::new(|| Ok(1)))
        .unwrap();
    sender
        .submit(steering::request(&owner, "first steering").await)
        .await
        .unwrap();
    sender
        .submit(steering::request(&owner, "second steering").await)
        .await
        .unwrap();
    owner
        .execute(&agent, CancellationToken::new(), None)
        .await
        .unwrap();
    let stored = Journal::open(dir.path().join("attachment"))
        .unwrap()
        .load_session(retry.session_id)
        .unwrap()
        .session;
    let inputs: Vec<_> = stored
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(
        inputs,
        vec!["accepted prompt", "first steering", "second steering"]
    );
    assert!(
        sender
            .submit(steering::request(&owner, "too late").await)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn duplicate_outcomes_are_readable_while_execution_ownership_is_held() {
    let (dir, mut owner, agent, _, _, mut retry) = setup("success", Arc::new(SilentSink)).await;
    let duplicate = TurnAdmission {
        command_id: retry.command_id,
        machine_id: retry.machine_id,
        principal_id: retry.principal_id,
        session_id: retry.session_id,
        expected_revision: retry.expected_revision,
        expires_at_ms: retry.expires_at_ms,
        prompt: retry.prompt.clone(),
    };
    let Admission::Existing(run) = RunOwner::admit_at(dir.path().join("attachment"), duplicate, 1)
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(run.state, RunState::Accepted);
    owner
        .execute(&agent, CancellationToken::new(), None)
        .await
        .unwrap();
    retry.prompt.push_str("changed");
    assert!(
        RunOwner::admit_at(dir.path().join("attachment"), retry, 1)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn empty_text_deltas_are_noops_not_storage_failures() {
    let (_dir, mut owner, agent, _, effects, _) = setup("empty-delta", Arc::new(SilentSink)).await;
    assert!(
        owner
            .execute(&agent, CancellationToken::new(), None)
            .await
            .is_ok()
    );
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(owner.record().await.unwrap().partial_text, "");
}

#[tokio::test]
async fn checkpointed_run_uses_pinned_model_instead_of_mutable_next_turn_model() {
    let (_dir, owner, agent, _, effects, _) = setup("success", Arc::new(SilentSink)).await;
    owner
        .storage(|store| store.journal.mark_running(&store.guard, store.run_id))
        .await
        .unwrap();
    agent.set_model("next-turn-model").unwrap();
    let (history, prompt) = owner.input.clone().unwrap();
    agent
        .run_checkpointed(
            history,
            prompt,
            CancellationToken::new(),
            None,
            &owner,
            "fixture".into(),
        )
        .await
        .unwrap();
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(agent.model(), "next-turn-model");
}

#[tokio::test]
async fn failed_steering_checkpoint_never_announces_durable_application() {
    let sink = Arc::new(Observed::default());
    let (dir, mut owner, agent, requests, effects, _) = setup("success", sink.clone()).await;
    let sender = owner
        .enable_steering_with_clock(Arc::new(steering::allow_actor), Arc::new(|| Ok(1)))
        .unwrap();
    sender
        .submit(steering::request(&owner, "steering-sentinel").await)
        .await
        .unwrap();
    fixture_database(dir.path().join("attachment/journal.sqlite3")).unwrap()
        .execute_batch("CREATE TRIGGER fail_steering BEFORE UPDATE ON sessions WHEN NEW.state LIKE '%steering-sentinel%' BEGIN SELECT RAISE(ABORT,'injected steering failure'); END;").unwrap();
    assert!(matches!(
        owner.execute(&agent, CancellationToken::new(), None).await,
        Err(AgentError::Checkpoint(_))
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert!(
        !sink
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentEvent::SteeringApplied { .. }))
    );
    let record = owner.record().await.unwrap();
    assert_eq!(record.state, RunState::Failed);
    let saved = Journal::open(dir.path().join("attachment"))
        .unwrap()
        .load_session(record.session_id)
        .unwrap();
    assert_eq!(saved.session.messages.len(), 1);
    assert_eq!(saved.session.messages[0].content, "accepted prompt");
}

#[tokio::test]
async fn admitted_workspace_uses_canonical_identity_and_rejects_another_root() {
    let (_dir, mut owner, agent, requests, effects, _) =
        setup("success", Arc::new(SilentSink)).await;
    assert_eq!(owner.workspace, agent.workspace());
    let other = tempfile::tempdir().unwrap();
    owner.workspace = other.path().canonicalize().unwrap();
    assert!(matches!(
        owner.execute(&agent, CancellationToken::new(), None).await,
        Err(AgentError::Checkpoint(_))
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert_eq!(owner.record().await.unwrap().state, RunState::Failed);
}

struct NoChildren;
#[async_trait]
impl crate::subagent::SubagentExecutor for NoChildren {
    async fn execute(
        &self,
        _: crate::subagent::ExecutionContext,
    ) -> Result<crate::subagent::SubagentResult, String> {
        Err("unexpected child execution".into())
    }
}

#[tokio::test]
async fn scoped_checkpoint_uses_admitted_identity_and_seals_only_accepted_work() {
    use crate::{
        completion::{
            FinalOutcome,
            runtime::{Coordinator, RunHandle},
        },
        subagent::{AgentTreeStore, RuntimeLimits, SubagentRuntime},
        todo::{TodoScope, TodoStore},
    };
    for mode in ["success", "provider-failure"] {
        let (dir, mut owner, agent, requests, _, _) = setup(mode, Arc::new(SilentSink)).await;
        let coordinator =
            Coordinator::open(dir.path().join("completion"), agent.workspace()).unwrap();
        let todos = Arc::new(
            TodoStore::new(
                dir.path().join("todos/list.json"),
                TodoScope::workspace(agent.workspace().to_owned()),
            )
            .with_coordinator(coordinator.clone()),
        );
        let agents = AgentTreeStore::new(dir.path().join("agents/tree.json"))
            .with_coordinator(coordinator.clone());
        let runtime = Arc::new(
            SubagentRuntime::new(
                Arc::new(NoChildren),
                RuntimeLimits::default(),
                Some(agents.clone()),
            )
            .unwrap(),
        );
        let agent = agent
            .with_completion_coordinator(coordinator.clone())
            .with_completion_gate(todos, agents, runtime);
        let outcome = owner.execute(&agent, CancellationToken::new(), None).await;
        let record = owner.record().await.unwrap();
        let session = Journal::open(dir.path().join("attachment"))
            .unwrap()
            .load_session(record.session_id)
            .unwrap()
            .session;
        assert_eq!(session.completion_runs.len(), 1);
        assert_eq!(session.completion_runs[0].run_id, record.id);
        assert_eq!(session.completion_runs[0].session_id, record.session_id);
        assert_eq!(session.run_summaries.len(), 1);
        assert_eq!(
            session.run_summaries[0].phase,
            if mode == "success" {
                crate::agent::CompletionPhase::Completed
            } else {
                crate::agent::CompletionPhase::Interrupted
            }
        );
        let handle = RunHandle::resume(coordinator, record.session_id, record.id)
            .await
            .unwrap();
        let decision = handle.decision().await.unwrap().unwrap();
        if mode == "success" {
            assert_eq!(outcome.unwrap().stop_reason, StopReason::Completed);
            assert_eq!(record.state, RunState::Completed);
            assert!(record.final_checkpointed);
            assert_eq!(decision.outcome, FinalOutcome::Completed);
            assert_eq!(requests.load(Ordering::SeqCst), 2);
        } else {
            assert!(outcome.is_err());
            assert_eq!(record.state, RunState::Failed);
            assert!(!record.final_checkpointed);
            assert_eq!(decision.outcome, FinalOutcome::Interrupted);
            assert_eq!(requests.load(Ordering::SeqCst), 1);
        }
    }
}

mod owner;

mod steering;
