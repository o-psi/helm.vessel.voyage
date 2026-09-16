//! Retained terminal ownership and private-control boundaries, using only PTYs
//! created by the test. Cleanup is explicitly observed before each fixture ends.
use super::*;
use crate::model::{ModelRequest, ModelResponse};
use crate::provider::{ModelInfo, Provider, ProviderError};
use crate::terminal::InteractiveTerminals;
use crate::tools::{InteractionMode, Redactor, ToolContext, ToolRegistry, UnattendedApprover};
use serde_json::{Value, json};
use voyage_protocol::process::TerminalOperation;

struct Offline;
#[async_trait::async_trait]
impl Provider for Offline {
    async fn models(&self) -> std::result::Result<Vec<ModelInfo>, ProviderError> {
        Ok(vec![ModelInfo::minimal("retained-fixture")])
    }
    async fn complete(&self, _: ModelRequest) -> std::result::Result<ModelResponse, ProviderError> {
        panic!("retained controls must not dispatch inference")
    }
}
struct NoChildren;
#[async_trait::async_trait]
impl crate::subagent::SubagentExecutor for NoChildren {
    async fn execute(
        &self,
        _: crate::subagent::ExecutionContext,
    ) -> std::result::Result<crate::subagent::SubagentResult, String> {
        panic!("retained fixture must not execute child agents")
    }
}
struct RetainedFixture {
    _root: tempfile::TempDir,
    state: Arc<State>,
    run: Uuid,
    _run_owner: crate::attachment::runtime::RunOwner,
    agent: Arc<crate::Agent>,
    context: ToolContext,
    manager: crate::tools::ProcessTool,
    cancel: CancellationToken,
    subagents: Arc<crate::subagent::SubagentRuntime>,
}
impl RetainedFixture {
    async fn new(access: crate::config::AccessMode) -> Self {
        let (root, state) = crate::server::tests::fixture().await;
        let config = Config {
            access: Some(access),
            ..Default::default()
        };
        let context = ToolContext {
            policy: Arc::new(
                crate::policy::Policy::new(&config, state.registration.workspace.clone()).unwrap(),
            ),
            approver: Arc::new(UnattendedApprover { allow: false }),
            redactor: Arc::new(Redactor::default()),
            interaction: InteractionMode::Unattended,
            tool_call_id: None,
            artifact_scope: None,
            github: None,
            completion: None,
            timeout: std::time::Duration::from_secs(3),
            max_output_bytes: 8192,
            environment: Default::default(),
            cancellation: CancellationToken::new(),
            execution_id: Uuid::new_v4(),
        };
        let agent = Arc::new(crate::Agent::new(
            Box::new(Offline),
            ToolRegistry::standard(),
            context.clone(),
            Arc::new(crate::agent::SilentSink),
            "retained-fixture".into(),
            "fixture".into(),
            64,
            None,
        ));
        let manager = agent.operator_terminals().unwrap().0;
        let admission = state
            .owner
            .admit(crate::attachment::journal::TurnAdmission {
                coordination: None,
                operator_name: None,
                command_id: Uuid::new_v4(),
                machine_id: state.actor.installation_id,
                principal_id: state.actor.principal_id,
                session_id: state.owner.session_id(),
                expected_revision: 0,
                expires_at_ms: expiry() as i64,
                prompt: "retained fixture ownership".into(),
                parts: vec![],
            })
            .await
            .unwrap();
        let crate::attachment::runtime::Admission::New(run_owner) = admission else {
            panic!("fresh run")
        };
        let run = run_owner.record().await.unwrap().id;
        let cancel = CancellationToken::new();
        let subagents = Arc::new(
            crate::subagent::SubagentRuntime::new(
                Arc::new(NoChildren),
                crate::subagent::RuntimeLimits::default(),
                None,
            )
            .unwrap(),
        );
        state
            .controls
            .open(run, agent.clone(), subagents.clone(), cancel.clone())
            .await;
        state
            .controls
            .retain_root(&state.owner, run, &agent)
            .await
            .unwrap();
        Self {
            _root: root,
            state,
            run,
            _run_owner: run_owner,
            agent,
            context,
            manager,
            cancel,
            subagents,
        }
    }
    async fn start(&self) -> Uuid {
        use crate::tools::Tool;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let result=self.manager.execute(json!({"action":"start","command":"/bin/cat","name":"owned-fixture","cols":80,"rows":24}), &self.context).await;
            match result {
                Err(ref error)
                    if error.to_string().contains("database is locked")
                        && !self.manager.has_owned_work()
                        && std::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await
                }
                result => {
                    result.unwrap();
                    break;
                }
            }
        }
        let metadata = self.manager.metadata().unwrap();
        assert_eq!(metadata.len(), 1);
        metadata[0].id
    }
    async fn terminal(&self, id: Uuid, operation: TerminalOperation) -> Result<Value> {
        self.state.controls.terminal(self.run, id, operation).await
    }
    async fn finish(mut self) {
        assert!(self.state.controls.close().await);
        self.state
            .controls
            .shutdown_retained(&self.state.owner)
            .await
            .unwrap();
        self.subagents.shutdown().await;
        self._run_owner.fail_before_execution().await.unwrap();
        assert!(!self.manager.has_owned_work());
        assert_eq!(
            self.state.owner.session_resources().await.unwrap(),
            json!([])
        );
        assert!(self.state.controls.retained_tool().await.is_none());
    }
}
fn expiry() -> u64 {
    (chrono::Utc::now().timestamp_millis() + 60_000) as u64
}

#[tokio::test]
async fn private_attach_returns_typed_screen_and_disables_model_capture() {
    use crate::tools::Tool;
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    let frame = fixture
        .terminal(id, TerminalOperation::Attach)
        .await
        .unwrap();
    assert_eq!(frame["terminal_id"], id.to_string());
    assert_eq!(frame["run_id"], fixture.run.to_string());
    assert_eq!(frame["privacy"], "human_only");
    assert!(frame["screen"].is_object());
    assert!(frame["rows"].is_array());
    assert!(
        fixture
            .manager
            .execute(
                json!({"action":"write","id":id,"data":"model-input"}),
                &fixture.context
            )
            .await
            .is_err()
    );
    let read = fixture
        .manager
        .execute(json!({"action":"read","id":id}), &fixture.context)
        .await
        .unwrap();
    assert!(read.contains("privacy") || read.contains("human"));
    fixture.finish().await;
}

#[tokio::test]
async fn private_write_implicitly_attaches_and_has_no_replayable_payload() {
    use crate::tools::Tool;
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    let response = fixture
        .terminal(
            id,
            TerminalOperation::Write {
                bytes: b"private fixture input\n".to_vec(),
            },
        )
        .await
        .unwrap();
    assert_eq!(response["accepted"], true);
    assert_eq!(response["replay"], "never");
    assert!(!response.to_string().contains("private fixture input"));
    let read = fixture
        .manager
        .execute(json!({"action":"read","id":id}), &fixture.context)
        .await
        .unwrap();
    assert!(!read.contains("private fixture input"));
    assert!(
        fixture
            .manager
            .execute(
                json!({"action":"write","id":id,"data":"model input"}),
                &fixture.context
            )
            .await
            .is_err()
    );
    fixture.finish().await;
}

#[tokio::test]
async fn retained_private_resize_sets_screen_dimensions() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    let response = fixture
        .terminal(
            id,
            TerminalOperation::Resize {
                columns: 91,
                rows: 29,
            },
        )
        .await
        .unwrap();
    assert_eq!(response["accepted"], true);
    assert_eq!(response["replay"], "never");
    let snapshot = fixture
        .terminal(id, TerminalOperation::Snapshot)
        .await
        .unwrap();
    assert_eq!(snapshot["screen"]["columns"], 91);
    assert_eq!(snapshot["screen"]["height"], 29);
    assert_eq!(snapshot["rows"].as_array().unwrap().len(), 29);
    fixture.finish().await;
}

#[tokio::test]
async fn zero_terminal_dimensions_are_rejected_before_attach() {
    use crate::tools::Tool;
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    for (columns, rows) in [(0, 24), (80, 0), (0, 0), (u16::MAX, 24), (80, u16::MAX)] {
        let error = fixture
            .terminal(id, TerminalOperation::Resize { columns, rows })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("invalid terminal size"));
    }
    // Invalid controls did not silently switch the terminal into private mode.
    fixture
        .manager
        .execute(
            json!({"action":"write","id":id,"data":"still model-owned\n"}),
            &fixture.context,
        )
        .await
        .unwrap();
    fixture.finish().await;
}

#[tokio::test]
async fn oversized_private_input_is_rejected_without_taking_over_capture() {
    use crate::tools::Tool;
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    let error = fixture
        .terminal(
            id,
            TerminalOperation::Write {
                bytes: vec![b'x'; 65537],
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("64 KiB"));
    fixture
        .manager
        .execute(
            json!({"action":"write","id":id,"data":"visible\n"}),
            &fixture.context,
        )
        .await
        .unwrap();
    fixture.finish().await;
}

#[tokio::test]
async fn stale_run_cannot_attach_write_resize_or_snapshot_retained_terminal() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    for operation in [
        TerminalOperation::Attach,
        TerminalOperation::Snapshot,
        TerminalOperation::Write {
            bytes: b"wrong generation".to_vec(),
        },
        TerminalOperation::Resize {
            columns: 80,
            rows: 24,
        },
    ] {
        let error = fixture
            .state
            .controls
            .terminal(Uuid::new_v4(), id, operation)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stale terminal run binding"));
    }
    fixture.finish().await;
}

#[tokio::test]
async fn missing_terminal_is_not_confused_with_missing_retained_owner() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = Uuid::new_v4();
    assert!(
        fixture
            .terminal(id, TerminalOperation::Attach)
            .await
            .is_err()
    );
    assert!(
        fixture
            .terminal(id, TerminalOperation::Snapshot)
            .await
            .is_err()
    );
    assert!(
        fixture
            .terminal(id, TerminalOperation::Write { bytes: vec![] })
            .await
            .is_err()
    );
    assert!(
        fixture
            .terminal(
                id,
                TerminalOperation::Resize {
                    columns: 80,
                    rows: 24
                }
            )
            .await
            .is_err()
    );
    assert!(fixture.state.controls.retained_tool().await.is_some());
    fixture.finish().await;
}

#[tokio::test]
async fn retained_terminal_survives_live_controls_close_until_explicit_shutdown() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    assert!(fixture.state.controls.close().await);
    assert!(
        fixture
            .state
            .controls
            .inspect(Some(fixture.run), "terminals")
            .await
            .is_err()
    );
    let snapshot = fixture
        .terminal(id, TerminalOperation::Snapshot)
        .await
        .unwrap();
    assert_eq!(snapshot["run_id"], fixture.run.to_string());
    assert!(fixture.manager.has_owned_work());
    fixture.finish().await;
}

#[tokio::test]
async fn cancelled_live_controls_do_not_invalidate_session_retained_terminal_binding() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    fixture.cancel.cancel();
    assert!(
        fixture
            .state
            .controls
            .inspect(Some(fixture.run), "tools")
            .await
            .is_err()
    );
    assert!(
        fixture
            .terminal(id, TerminalOperation::Snapshot)
            .await
            .is_ok()
    );
    fixture.finish().await;
}

#[tokio::test]
async fn reusing_root_manager_updates_run_binding_without_duplicate_resource() {
    let mut fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    let previous = fixture.run;
    let next = Uuid::new_v4();
    let before = fixture.state.owner.session_resources().await.unwrap();
    fixture
        .state
        .controls
        .retain_root(&fixture.state.owner, next, &fixture.agent)
        .await
        .unwrap();
    fixture.run = next;
    assert_eq!(
        before,
        fixture.state.owner.session_resources().await.unwrap()
    );
    assert!(
        fixture
            .state
            .controls
            .terminal(previous, id, TerminalOperation::Snapshot)
            .await
            .is_err()
    );
    let frame = fixture
        .terminal(id, TerminalOperation::Snapshot)
        .await
        .unwrap();
    assert_eq!(frame["run_id"], next.to_string());
    fixture.finish().await;
}

#[tokio::test]
async fn observed_retained_shutdown_is_idempotent_and_revokes_terminal_controls() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    fixture
        .state
        .controls
        .shutdown_retained(&fixture.state.owner)
        .await
        .unwrap();
    fixture
        .state
        .controls
        .shutdown_retained(&fixture.state.owner)
        .await
        .unwrap();
    let error = fixture
        .terminal(id, TerminalOperation::Snapshot)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no retained terminal manager"));
    assert!(!fixture.manager.has_owned_work());
    assert_eq!(
        fixture.state.owner.session_resources().await.unwrap(),
        json!([])
    );
    fixture.finish().await;
}

#[tokio::test]
async fn read_only_retained_private_write_is_refused_before_terminal_lookup() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::ReadOnly).await;
    let error = fixture
        .terminal(
            Uuid::new_v4(),
            TerminalOperation::Write {
                bytes: b"forbidden".to_vec(),
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("read-only access mode"));
    fixture.finish().await;
}

#[tokio::test]
async fn terminal_transport_response_uses_retained_run_not_current_control_run() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    let command = RuntimeCommand::Terminal {
        run_id: fixture.run,
        terminal_id: id,
        operation: TerminalOperation::Attach,
    };
    let (response, done) = exchange(fixture.state.clone(), request(&fixture.state, command)).await;
    done.unwrap();
    assert!(response.error.is_none(), "{response:?}");
    assert!(!response.outcome_unknown);
    assert_eq!(response.result["privacy"], "human_only");
    assert_eq!(response.result["terminal_id"], id.to_string());
    fixture.finish().await;
}

#[tokio::test]
async fn live_inventory_lists_owned_terminal_then_observed_shutdown_removes_it() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    let inventory = fixture
        .state
        .controls
        .inspect(Some(fixture.run), "terminals")
        .await
        .unwrap();
    assert!(
        inventory["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry.to_string().contains(&id.to_string()))
    );
    fixture
        .state
        .controls
        .shutdown_retained(&fixture.state.owner)
        .await
        .unwrap();
    let terminals = fixture.manager.list().await.unwrap();
    assert!(
        terminals
            .iter()
            .all(|entry| entry.state != crate::terminal::TerminalState::Running)
    );
    fixture.finish().await;
}

#[tokio::test]
async fn teardown_bad_confirmation_preserves_owned_terminal_and_resource() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    fixture.start().await;
    for delete in [false, true] {
        let command = if delete {
            RuntimeCommand::Delete {
                command_id: Uuid::new_v4(),
                expected_revision: fixture.state.owner.snapshot().await.unwrap().revision,
                expires_at_ms: expiry(),
                confirm_session_id: Uuid::new_v4(),
            }
        } else {
            RuntimeCommand::Clear {
                command_id: Uuid::new_v4(),
                expected_revision: fixture.state.owner.snapshot().await.unwrap().revision,
                expires_at_ms: expiry(),
                confirm_session_id: Uuid::new_v4(),
            }
        };
        let error = fixture
            .state
            .controls
            .close_for_command(&fixture.state.owner, &command)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("confirmation mismatch"));
        assert!(fixture.manager.has_owned_work());
        assert!(fixture.state.controls.retained_tool().await.is_some());
    }
    fixture.finish().await;
}

#[tokio::test]
async fn teardown_stale_revision_preserves_owned_terminal() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    fixture.start().await;
    let command = RuntimeCommand::Archive {
        command_id: Uuid::new_v4(),
        expected_revision: 99,
        expires_at_ms: expiry(),
        archived: true,
    };
    assert!(
        fixture
            .state
            .controls
            .close_for_command(&fixture.state.owner, &command)
            .await
            .is_err()
    );
    assert!(fixture.manager.has_owned_work());
    fixture.finish().await;
}

#[tokio::test]
async fn teardown_expired_command_preserves_owned_terminal() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    fixture.start().await;
    let command = RuntimeCommand::SetAccess {
        command_id: Uuid::new_v4(),
        expected_revision: fixture.state.owner.snapshot().await.unwrap().revision,
        expires_at_ms: 0,
        access: "read-only".into(),
    };
    assert!(
        fixture
            .state
            .controls
            .close_for_command(&fixture.state.owner, &command)
            .await
            .is_err()
    );
    assert!(fixture.manager.has_owned_work());
    fixture.finish().await;
}

#[tokio::test]
async fn teardown_noncanonical_compaction_cannot_destroy_retained_terminal() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    fixture.start().await;
    let command = RuntimeCommand::Compact {
        command_id: Uuid::new_v4(),
        expected_revision: fixture.state.owner.snapshot().await.unwrap().revision,
        expires_at_ms: expiry(),
        retain: 0,
        preserve_canonical: false,
    };
    let error = fixture
        .state
        .controls
        .close_for_command(&fixture.state.owner, &command)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("preserve_canonical"));
    assert!(fixture.manager.has_owned_work());
    fixture.finish().await;
}

#[tokio::test]
async fn teardown_invalid_compaction_retain_cannot_destroy_retained_terminal() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    fixture.start().await;
    let command = RuntimeCommand::Compact {
        command_id: Uuid::new_v4(),
        expected_revision: fixture.state.owner.snapshot().await.unwrap().revision,
        expires_at_ms: expiry(),
        retain: 0,
        preserve_canonical: true,
    };
    assert!(
        fixture
            .state
            .controls
            .close_for_command(&fixture.state.owner, &command)
            .await
            .is_err()
    );
    assert!(fixture.manager.has_owned_work());
    fixture.finish().await;
}

#[tokio::test]
async fn valid_clear_teardown_observes_only_fixture_owned_terminal() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    fixture.start().await;
    let command = RuntimeCommand::Clear {
        command_id: Uuid::new_v4(),
        expected_revision: fixture.state.owner.snapshot().await.unwrap().revision,
        expires_at_ms: expiry(),
        confirm_session_id: fixture.state.owner.session_id(),
    };
    fixture
        .state
        .controls
        .close_for_command(&fixture.state.owner, &command)
        .await
        .unwrap();
    assert!(!fixture.manager.has_owned_work());
    assert_eq!(
        fixture.state.owner.session_resources().await.unwrap(),
        json!([])
    );
    // Teardown itself is not the metadata mutation.
    assert!(fixture.state.owner.snapshot().await.unwrap().revision > 0);
    fixture.finish().await;
}

#[tokio::test]
async fn unsupported_teardown_is_side_effect_free() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    fixture.start().await;
    let error = fixture
        .state
        .controls
        .close_for_command(&fixture.state.owner, &RuntimeCommand::Health)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unsupported terminal teardown"));
    assert!(fixture.manager.has_owned_work());
    fixture.finish().await;
}

#[tokio::test]
async fn durable_receipt_short_circuits_teardown_even_when_live_terminal_exists() {
    let fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    fixture.start().await;
    let id = Uuid::new_v4();
    fixture
        .state
        .owner
        .resolve_process_command(id, fixture.state.actor.principal_id, None)
        .await
        .unwrap();
    let command = RuntimeCommand::Clear {
        command_id: id,
        expected_revision: 99,
        expires_at_ms: 0,
        confirm_session_id: Uuid::nil(),
    };
    fixture
        .state
        .controls
        .close_for_command(&fixture.state.owner, &command)
        .await
        .unwrap();
    assert!(fixture.manager.has_owned_work());
    fixture.finish().await;
}

#[tokio::test]
async fn automatic_suspension_preserves_background_terminal_until_explicit_close() {
    let mut fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let id = fixture.start().await;
    // Terminalize the synthetic turn without dispatching inference; the PTY is
    // separately session-owned and must survive both model and human detachment.
    fixture._run_owner.fail_before_execution().await.unwrap();
    assert!(fixture.state.controls.close().await);
    fixture
        .terminal(id, TerminalOperation::Attach)
        .await
        .unwrap();
    super::suspension::suspend(&fixture.state).await.unwrap();
    assert!(!fixture.state.shutdown.is_cancelled());
    assert!(
        !fixture
            .state
            .suspend_requested
            .load(std::sync::atomic::Ordering::Acquire)
    );
    assert!(fixture.manager.has_owned_work());
    assert!(
        !fixture
            .state
            .owner
            .session_resources()
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    fixture
        .terminal(id, TerminalOperation::Snapshot)
        .await
        .unwrap();
    use crate::tools::Tool;
    fixture
        .manager
        .execute(json!({"action":"terminate", "id":id}), &fixture.context)
        .await
        .unwrap();
    super::suspension::suspend(&fixture.state).await.unwrap();
    assert!(fixture.state.shutdown.is_cancelled());
    assert!(
        fixture
            .state
            .suspend_requested
            .load(std::sync::atomic::Ordering::Acquire)
    );
    assert!(!fixture.manager.has_owned_work());
    assert_eq!(
        fixture.state.owner.session_resources().await.unwrap(),
        json!([])
    );
    assert!(fixture.state.controls.retained_tool().await.is_none());
}

#[tokio::test]
async fn automatic_suspension_retires_empty_terminal_manager() {
    let mut fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    fixture._run_owner.fail_before_execution().await.unwrap();
    assert!(fixture.state.controls.close().await);
    super::suspension::suspend(&fixture.state).await.unwrap();
    assert!(fixture.state.shutdown.is_cancelled());
    assert!(!fixture.manager.has_owned_work());
    assert_eq!(
        fixture.state.owner.session_resources().await.unwrap(),
        json!([])
    );
    assert!(fixture.state.controls.retained_tool().await.is_none());
}

#[tokio::test]
async fn automatic_suspension_preserves_unresolved_other_session_resources() {
    let mut fixture = RetainedFixture::new(crate::config::AccessMode::Unrestricted).await;
    let resource = Uuid::new_v4();
    fixture
        .state
        .owner
        .session_resource_adopt(resource, fixture.run, "background-fixture".into())
        .await
        .unwrap();
    fixture._run_owner.fail_before_execution().await.unwrap();
    assert!(fixture.state.controls.close().await);
    assert!(super::suspension::suspend(&fixture.state).await.is_err());
    assert!(!fixture.state.shutdown.is_cancelled());
    assert_eq!(
        fixture
            .state
            .owner
            .session_resources()
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fixture
        .state
        .owner
        .session_resource_closed(resource)
        .await
        .unwrap();
    super::suspension::suspend(&fixture.state).await.unwrap();
    assert!(fixture.state.shutdown.is_cancelled());
}
