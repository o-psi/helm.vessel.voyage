use super::*;
use voyage_protocol::process::{ProcessState, RuntimeCommand};
pub(super) async fn fixture() -> (tempfile::TempDir, Arc<State>) {
    let root = tempfile::tempdir().unwrap();
    let directory =
        crate::attachment::journal::prepare_directory(root.path().join("runtime")).unwrap();
    let session = crate::session::Session::new(root.path().into(), "fixture".into());
    let mut journal = Journal::open(directory.join("journal")).unwrap();
    journal.create_session(&session).unwrap();
    drop(journal);
    let owner = ManagedSessionOwner::open(directory.join("journal"), session.id)
        .await
        .unwrap();
    owner.initialize_process_commands().await.unwrap();
    owner.initialize_session_resources().await.unwrap();
    let actor = LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    };
    owner
        .initialize_command_bindings(actor.principal_id)
        .await
        .unwrap();
    let registration = ProcessRegistration {
        executable: None,
        protocol: 1,
        session_id: session.id,
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        restart_from: None,
        initialize: None,
        config_path: None,
        token: "fixture-token".into(),
        workspace: root.path().to_owned(),
        state: ProcessState::Live,
        name: None,
    };
    let config = Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        ..Default::default()
    };
    let state = Arc::new(State {
        host_browser: crate::host_browser::HostBrowser::new(
            directory.join("host-browser"),
            session.id,
            registration.incarnation,
            None,
        ),
        browser: crate::browser::BrowserBroker::open(
            directory.join("journal"),
            session.id,
            registration.incarnation,
        )
        .unwrap(),
        directory,
        workflows: workflows::Workflows::default(),
        controls: Arc::new(controls::LiveControls::default()),
        cleanup: Arc::new(crate::execution::cleanup::CleanupSlot::default()),
        owner,
        actor,
        config: tokio::sync::RwLock::new(config),
        registration,
        active: Mutex::new(None),
        admission: Mutex::new(()),
        requests: tokio::sync::RwLock::new(()),
        suspend_requested: std::sync::atomic::AtomicBool::new(false),
        shutdown: CancellationToken::new(),
        archive_receipt: Mutex::new(None),
    });
    (root, state)
}
fn auth(state: &State) -> authorization::Authorization {
    authorization::Authorization {
        owner_connection: false,
        authority: None,
        actor: state.actor,
        grant: None,
    }
}
async fn command(state: &Arc<State>, command: RuntimeCommand) -> Result<serde_json::Value> {
    commands::dispatch_admitted(state, command, auth(state)).await
}
#[tokio::test]
async fn idle_read_commands_report_canonical_state_without_starting_a_run() {
    let (_root, state) = fixture().await;
    let health = command(&state, RuntimeCommand::Health).await.unwrap();
    assert_eq!(
        health["session_id"],
        state.registration.session_id.to_string()
    );
    let history = command(
        &state,
        RuntimeCommand::History {
            offset: 0,
            limit: 10,
            expected_revision: Some(0),
        },
    )
    .await
    .unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 0);
    let id = Uuid::new_v4();
    let receipt = command(&state, RuntimeCommand::Receipt { command_id: id })
        .await
        .unwrap();
    assert_eq!(receipt["status"], "unknown");
    let decisions = command(&state, RuntimeCommand::Decisions).await.unwrap();
    assert!(decisions.is_array());
    assert!(state.active.lock().await.is_none());
    assert!(
        command(
            &state,
            RuntimeCommand::MessageChunk {
                index: 0,
                offset: 0,
                limit: 100,
                expected_revision: 0
            }
        )
        .await
        .is_err()
    );
    assert!(
        command(
            &state,
            RuntimeCommand::RunOutput {
                run_id: Uuid::new_v4(),
                offset: 0,
                limit: 100
            }
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn access_configuration_is_durable_deduplicated_and_rejects_stale_changes() {
    let (_root, state) = fixture().await;
    let id = Uuid::new_v4();
    let make = |id, revision, access| RuntimeCommand::SetAccess {
        command_id: id,
        expected_revision: revision,
        expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60000) as u64,
        access,
    };
    let command = make(id, 0, "read-only".into());
    let result = configuration::configure(&state, command.clone(), auth(&state))
        .await
        .unwrap();
    assert_eq!(
        state.config.read().await.access,
        Some(crate::config::AccessMode::ReadOnly)
    );
    let repeated = configuration::configure(&state, command, auth(&state))
        .await
        .unwrap();
    assert_eq!(result, repeated);
    assert!(
        configuration::configure(
            &state,
            make(Uuid::new_v4(), 0, "unrestricted".into()),
            auth(&state)
        )
        .await
        .is_err()
    );
    assert_eq!(
        state.config.read().await.access,
        Some(crate::config::AccessMode::ReadOnly)
    );
    assert!(state.owner.saved_configuration().await.unwrap().is_some());
}

#[tokio::test]
async fn registration_rejects_unsafe_or_invalid_artifacts() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (_root, state) = fixture().await;
    let path = state.directory.join("registration.json");
    let mut registration = state.registration.clone();
    registration.token = "x".repeat(32);
    std::fs::write(&path, serde_json::to_vec(&registration).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        transport::registration(&state.directory)
            .unwrap()
            .session_id,
        registration.session_id
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(transport::registration(&state.directory).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    registration.protocol = 999;
    std::fs::write(&path, serde_json::to_vec(&registration).unwrap()).unwrap();
    assert!(transport::registration(&state.directory).is_err());
    registration.protocol = 1;
    registration.token = "short".into();
    std::fs::write(&path, serde_json::to_vec(&registration).unwrap()).unwrap();
    assert!(transport::registration(&state.directory).is_err());
    let original = state.directory.join("original");
    std::fs::rename(&path, &original).unwrap();
    symlink(original, &path).unwrap();
    assert!(transport::registration(&state.directory).is_err());
}

#[tokio::test]
async fn idle_controls_do_not_manufacture_live_run_or_terminal_authority() {
    let (_root, state) = fixture().await;
    let config = state.config.read().await.clone();
    for section in ["policy", "tools", "terminals", "workflows"] {
        let value = state
            .controls
            .inspect_or_idle(None, section, &config, &state.registration.workspace)
            .await
            .unwrap();
        assert!(!value.is_null(), "{section}");
    }
    assert!(
        state
            .controls
            .inspect(Some(Uuid::new_v4()), "tools")
            .await
            .is_err()
    );
    assert!(
        state
            .controls
            .inspect_or_idle(None, "unknown", &config, &state.registration.workspace)
            .await
            .is_err()
    );
    assert!(state.controls.close().await);
    state
        .controls
        .shutdown_retained(&state.owner)
        .await
        .unwrap();
}

#[tokio::test]
async fn metadata_and_lifecycle_commands_persist_receipts_and_stop_cleanly() {
    let (_root, state) = fixture().await;
    let expires_at_ms = (chrono::Utc::now().timestamp_millis() + 60000) as u64;
    let rename = RuntimeCommand::Rename {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms,
        name: "release fixture".into(),
    };
    let receipt = command(&state, rename.clone()).await.unwrap();
    assert_ne!(receipt["status"], "rejected");
    assert_eq!(command(&state, rename).await.unwrap(), receipt);
    assert_eq!(
        state
            .owner
            .snapshot()
            .await
            .unwrap()
            .session
            .name
            .as_deref(),
        Some("release fixture")
    );
    let revision = state.owner.snapshot().await.unwrap().revision;
    let compact = RuntimeCommand::Compact {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms,
        retain: 1,
        preserve_canonical: false,
    };
    assert!(command(&state, compact).await.is_err());
    let archive = RuntimeCommand::Archive {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms,
        archived: true,
    };
    command(&state, archive).await.unwrap();
    assert_eq!(
        state.owner.process_snapshot().await.unwrap()["lifecycle"]["archived"],
        true
    );
    assert!(state.shutdown.is_cancelled());
    assert!(state.archive_receipt.lock().await.is_some());
    assert_eq!(
        command(&state, RuntimeCommand::Stop).await.unwrap()["cleanup"],
        "pending"
    );
}

#[tokio::test]
async fn dispatch_reserves_rejections_and_prevents_delayed_or_conflicting_admission() {
    let (_root, state) = fixture().await;
    let id = Uuid::new_v4();
    let original = RuntimeCommand::Rename {
        command_id: id,
        expected_revision: 99,
        expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60000) as u64,
        name: "must not appear".into(),
    };
    let error = dispatch::dispatch(&state, original.clone(), auth(&state))
        .await
        .unwrap_err();
    let rejected = error
        .downcast_ref::<dispatch::Rejected>()
        .unwrap()
        .0
        .clone();
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["command_id"], id.to_string());
    let replay = dispatch::dispatch(&state, original.clone(), auth(&state))
        .await
        .unwrap_err();
    assert_eq!(
        replay.downcast_ref::<dispatch::Rejected>().unwrap().0,
        rejected
    );
    let mut foreign = auth(&state);
    foreign.actor.principal_id = Uuid::new_v4();
    assert!(dispatch::dispatch(&state, original, foreign).await.is_err());
    let changed = RuntimeCommand::Rename {
        command_id: id,
        expected_revision: 0,
        expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60000) as u64,
        name: "changed payload".into(),
    };
    assert!(
        dispatch::dispatch(&state, changed, auth(&state))
            .await
            .is_err()
    );
    assert_eq!(state.owner.snapshot().await.unwrap().session.revision, 0);
    assert_eq!(
        state.owner.process_receipt(id).await.unwrap().unwrap(),
        rejected
    );
}

#[tokio::test]
async fn resolve_tombstone_blocks_late_dispatch_without_changing_session() {
    let (_root, state) = fixture().await;
    let id = Uuid::new_v4();
    let original = RuntimeCommand::Rename {
        command_id: id,
        expected_revision: 0,
        expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60000) as u64,
        name: "late delivery".into(),
    };
    let resolution = dispatch::dispatch(
        &state,
        RuntimeCommand::Resolve {
            command_id: id,
            original: Some(Box::new(original.clone())),
        },
        auth(&state),
    )
    .await
    .unwrap();
    assert_eq!(resolution["status"], "not_admitted");
    let error = dispatch::dispatch(&state, original, auth(&state))
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<dispatch::Rejected>().unwrap().0,
        resolution
    );
    assert_eq!(state.owner.snapshot().await.unwrap().session.revision, 0);
}

#[tokio::test]
async fn dispatch_success_is_replayable_but_not_rebindable() {
    let (_root, state) = fixture().await;
    let id = Uuid::new_v4();
    let rename = RuntimeCommand::Rename {
        command_id: id,
        expected_revision: 0,
        expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60000) as u64,
        name: "durable runtime name".into(),
    };
    let first = dispatch::dispatch(&state, rename.clone(), auth(&state))
        .await
        .unwrap();
    let revision = state.owner.snapshot().await.unwrap().session.revision;
    assert!(revision > 0);
    assert_eq!(
        dispatch::dispatch(&state, rename.clone(), auth(&state))
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        state.owner.snapshot().await.unwrap().session.revision,
        revision
    );
    let mut foreign = auth(&state);
    foreign.actor.principal_id = Uuid::new_v4();
    assert!(dispatch::dispatch(&state, rename, foreign).await.is_err());
    assert_eq!(
        state.owner.snapshot().await.unwrap().session.revision,
        revision
    );
}

#[tokio::test]
async fn retained_teardown_validates_confirmation_deadline_revision_and_compaction() {
    let (_root, state) = fixture().await;
    let deadline = (chrono::Utc::now().timestamp_millis() + 60000) as u64;
    let clear = |confirm, revision, expiry| RuntimeCommand::Clear {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry,
        confirm_session_id: confirm,
    };
    for (command, reason) in [
        (clear(Uuid::new_v4(), 0, deadline), "confirmation"),
        (clear(state.owner.session_id(), 99, deadline), "revision"),
        (clear(state.owner.session_id(), 0, 0), "deadline"),
        (clear(state.owner.session_id(), 0, u64::MAX), "deadline"),
    ] {
        let error = state
            .controls
            .close_for_command(&state.owner, &command)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
    }
    for (retain, preserve, valid) in [
        (0, true, false),
        (100001, true, false),
        (1, false, false),
        (1, true, true),
        (100000, true, true),
    ] {
        let compact = RuntimeCommand::Compact {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: deadline,
            retain,
            preserve_canonical: preserve,
        };
        assert_eq!(
            state
                .controls
                .close_for_command(&state.owner, &compact)
                .await
                .is_ok(),
            valid
        );
    }
    state
        .controls
        .close_for_command(&state.owner, &clear(state.owner.session_id(), 0, deadline))
        .await
        .unwrap();
    assert!(
        state
            .controls
            .close_for_command(&state.owner, &RuntimeCommand::Health)
            .await
            .is_err()
    );
    assert!(state.controls.retained_tool().await.is_none());
    state
        .controls
        .shutdown_retained(&state.owner)
        .await
        .unwrap();
    assert_eq!(state.owner.snapshot().await.unwrap().session.revision, 0);
}

#[tokio::test]
async fn retained_teardown_replay_does_not_revalidate_expired_confirmation() {
    let (_root, state) = fixture().await;
    let id = Uuid::new_v4();
    state
        .owner
        .resolve_process_command(id, state.actor.principal_id, None)
        .await
        .unwrap();
    state
        .controls
        .close_for_command(
            &state.owner,
            &RuntimeCommand::Clear {
                command_id: id,
                expected_revision: 999,
                expires_at_ms: 0,
                confirm_session_id: Uuid::nil(),
            },
        )
        .await
        .unwrap();
    assert!(state.controls.retained_tool().await.is_none());
}

// Offline controls fixture: provider inference and subagent execution are forbidden.
mod live_controls_batch {
    use super::*;
    use crate::model::{ModelRequest, ModelResponse};
    use crate::provider::{ModelInfo, Provider, ProviderError};
    use crate::tools::{InteractionMode, Redactor, ToolContext, ToolRegistry, UnattendedApprover};
    use serde_json::{Value, json};

    struct Offline;
    #[async_trait::async_trait]
    impl Provider for Offline {
        async fn models(&self) -> std::result::Result<Vec<ModelInfo>, ProviderError> {
            Ok(vec![ModelInfo::minimal("fixture")])
        }
        async fn complete(
            &self,
            _: ModelRequest,
        ) -> std::result::Result<ModelResponse, ProviderError> {
            panic!("controls must not perform inference")
        }
    }
    struct NeverExecute;
    #[async_trait::async_trait]
    impl crate::subagent::SubagentExecutor for NeverExecute {
        async fn execute(
            &self,
            _: crate::subagent::ExecutionContext,
        ) -> std::result::Result<crate::subagent::SubagentResult, String> {
            panic!("controls must not launch subagents")
        }
    }
    fn agent(state: &State, terminals: bool) -> Arc<crate::Agent> {
        let config = Config {
            access: Some(crate::config::AccessMode::Unrestricted),
            ..Default::default()
        };
        let context = ToolContext {
            policy: Arc::new(
                crate::policy::Policy::new(&config, state.registration.workspace.clone()).unwrap(),
            ),
            approver: Arc::new(UnattendedApprover { allow: false }),
            interaction: InteractionMode::Unattended,
            redactor: Arc::new(Redactor::default()),
            tool_call_id: None,
            artifact_scope: None,
            github: None,
            completion: None,
            timeout: std::time::Duration::from_secs(3),
            max_output_bytes: 4096,
            environment: Default::default(),
            cancellation: CancellationToken::new(),
            execution_id: Uuid::new_v4(),
        };
        Arc::new(crate::Agent::new(
            Box::new(Offline),
            if terminals {
                ToolRegistry::standard()
            } else {
                ToolRegistry::default()
            },
            context,
            Arc::new(crate::agent::SilentSink),
            "fixture".into(),
            "fixture".into(),
            64,
            None,
        ))
    }
    async fn open(state: &State, terminals: bool) -> (Uuid, Arc<crate::Agent>, CancellationToken) {
        let agent = agent(state, terminals);
        let run = Uuid::new_v4();
        let cancel = CancellationToken::new();
        let subagents = Arc::new(
            crate::subagent::SubagentRuntime::new(
                Arc::new(NeverExecute),
                crate::subagent::RuntimeLimits::default(),
                None,
            )
            .unwrap(),
        );
        state
            .controls
            .open(run, agent.clone(), subagents, cancel.clone())
            .await;
        (run, agent, cancel)
    }
    async fn section(section: &str) -> Value {
        let (_root, state) = fixture().await;
        let (run, _, cancel) = open(&state, true).await;
        let value = state.controls.inspect(Some(run), section).await.unwrap();
        assert_eq!(value["run_id"], run.to_string());
        assert_eq!(value["section"], section);
        assert!(!cancel.is_cancelled());
        assert!(state.controls.close().await);
        assert!(cancel.is_cancelled());
        value["value"].clone()
    }
    #[tokio::test]
    async fn tools_are_the_live_registry() {
        let tools = section("tools").await;
        assert!(
            tools
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "process")
        );
    }
    #[tokio::test]
    async fn models_use_mock_provider_without_inference() {
        let models = section("models").await;
        assert_eq!(models[0]["id"], "fixture");
    }
    #[tokio::test]
    async fn policy_is_a_live_serializable_snapshot() {
        assert!(!section("policy").await.is_null());
    }
    #[tokio::test]
    async fn subagents_start_empty() {
        assert_eq!(section("subagents").await, json!([]));
    }
    #[tokio::test]
    async fn archive_starts_empty() {
        assert_eq!(section("subagents_archive").await["agents"], json!([]));
    }
    #[tokio::test]
    async fn terminal_inventory_starts_empty() {
        assert_eq!(section("terminals").await, json!([]));
    }
    #[tokio::test]
    async fn workflows_are_discovered_without_execution() {
        assert!(!section("workflows").await.is_null());
    }
    #[tokio::test]
    async fn stale_run_cannot_inspect_live_tools() {
        let (_root, state) = fixture().await;
        open(&state, false).await;
        assert!(
            state
                .controls
                .inspect(Some(Uuid::new_v4()), "tools")
                .await
                .unwrap_err()
                .to_string()
                .contains("stale control run")
        );
        assert!(state.controls.close().await);
    }
    #[tokio::test]
    async fn cancelled_run_denies_controls_before_close() {
        let (_root, state) = fixture().await;
        let (run, _, cancel) = open(&state, false).await;
        cancel.cancel();
        assert!(
            state
                .controls
                .inspect(Some(run), "tools")
                .await
                .unwrap_err()
                .to_string()
                .contains("run controls closed")
        );
        assert!(state.controls.close().await);
    }
    #[tokio::test]
    async fn unknown_section_does_not_close_executor() {
        let (_root, state) = fixture().await;
        let (run, _, _) = open(&state, false).await;
        assert!(
            state
                .controls
                .inspect(Some(run), "not_a_section")
                .await
                .unwrap_err()
                .to_string()
                .contains("unknown control section")
        );
        assert_eq!(
            state.controls.inspect(None, "tools").await.unwrap()["value"],
            json!([])
        );
        assert!(state.controls.close().await);
    }
    #[tokio::test]
    async fn close_is_idempotent_and_removes_live_authority() {
        let (_root, state) = fixture().await;
        let (run, _, _) = open(&state, false).await;
        assert!(state.controls.close().await);
        assert!(state.controls.close().await);
        assert!(
            state
                .controls
                .inspect(Some(run), "tools")
                .await
                .unwrap_err()
                .to_string()
                .contains("no live executor")
        );
    }
    #[tokio::test]
    async fn absent_terminal_registry_reports_empty_inventory() {
        let (_root, state) = fixture().await;
        let (run, _, _) = open(&state, false).await;
        assert_eq!(
            state
                .controls
                .inspect(Some(run), "terminals")
                .await
                .unwrap()["value"],
            json!([])
        );
        assert!(state.controls.close().await);
    }
    #[tokio::test]
    async fn absent_todo_registry_reports_error() {
        let (_root, state) = fixture().await;
        let (run, _, _) = open(&state, false).await;
        assert!(state.controls.inspect(Some(run), "todos").await.is_err());
        assert!(state.controls.close().await);
    }
    #[tokio::test]
    async fn retaining_agent_without_terminals_is_noop() {
        let (_root, state) = fixture().await;
        state
            .controls
            .retain_root(&state.owner, Uuid::new_v4(), &agent(&state, false))
            .await
            .unwrap();
        assert!(state.controls.retained_tool().await.is_none());
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn invalid_tool_variant_never_creates_receipt() {
        let (_root, state) = fixture().await;
        assert!(
            state
                .controls
                .execute(
                    state.owner.clone(),
                    state.owner.session_id(),
                    RuntimeCommand::Health
                )
                .await
                .is_err()
        );
        assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    }
    async fn rejected_tool(name: String, arguments: Value, stale: bool) {
        let (_root, state) = fixture().await;
        let (run, _, _) = open(&state, true).await;
        let id = Uuid::new_v4();
        let result = state
            .controls
            .execute(
                state.owner.clone(),
                state.owner.session_id(),
                RuntimeCommand::ExecuteTool {
                    command_id: id,
                    expected_revision: 0,
                    expires_at_ms: u64::MAX,
                    run_id: if stale { Uuid::new_v4() } else { run },
                    name,
                    arguments,
                },
            )
            .await;
        assert!(result.is_err());
        assert!(state.owner.process_receipt(id).await.unwrap().is_none());
        assert!(state.controls.close().await);
    }
    #[tokio::test]
    async fn unregistered_tool_is_not_admitted() {
        rejected_tool("not_registered".into(), json!({}), false).await;
    }
    #[tokio::test]
    async fn oversized_tool_name_is_not_admitted() {
        rejected_tool("x".repeat(129), json!({}), false).await;
    }
    #[tokio::test]
    async fn oversized_tool_arguments_are_not_admitted() {
        rejected_tool("process".into(), json!({"data":"x".repeat(65537)}), false).await;
    }
    #[tokio::test]
    async fn stale_tool_run_is_not_admitted() {
        rejected_tool("process".into(), json!({"action":"list"}), true).await;
    }
    async fn retained_fixture(
        state: &State,
    ) -> (
        crate::attachment::runtime::RunOwner,
        Uuid,
        Arc<crate::Agent>,
    ) {
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
                expires_at_ms: chrono::Utc::now().timestamp_millis() + 60000,
                prompt: "retained resources".into(),
                parts: vec![],
            })
            .await
            .unwrap();
        let crate::attachment::runtime::Admission::New(run) = admission else {
            panic!("fresh run")
        };
        let id = run.record().await.unwrap().id;
        let agent = agent(state, true);
        state
            .controls
            .retain_root(&state.owner, id, &agent)
            .await
            .unwrap();
        (run, id, agent)
    }
    #[tokio::test]
    async fn retained_manager_survives_control_close_until_explicit_shutdown() {
        let (_root, state) = fixture().await;
        let (_run, _, _) = retained_fixture(&state).await;
        assert!(state.controls.retained_tool().await.is_some());
        assert!(state.controls.close().await);
        assert!(state.controls.retained_tool().await.is_some());
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
        assert!(state.controls.retained_tool().await.is_none());
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn same_root_manager_can_be_retained_again() {
        let (_root, state) = fixture().await;
        let (_run, id, agent) = retained_fixture(&state).await;
        state
            .controls
            .retain_root(&state.owner, id, &agent)
            .await
            .unwrap();
        assert!(state.controls.retained_tool().await.is_some());
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn distinct_root_manager_cannot_replace_unobserved_owner() {
        let (_root, state) = fixture().await;
        let (_run, id, _) = retained_fixture(&state).await;
        let error = state
            .controls
            .retain_root(&state.owner, id, &agent(&state, true))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("root terminal manager changed"));
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn stale_terminal_binding_is_rejected_before_lookup() {
        let (_root, state) = fixture().await;
        let (_run, _, _) = retained_fixture(&state).await;
        let error = state
            .controls
            .terminal(
                Uuid::new_v4(),
                Uuid::new_v4(),
                voyage_protocol::process::TerminalOperation::Attach,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stale terminal run binding"));
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn retained_terminal_rejects_oversized_private_input() {
        let (_root, state) = fixture().await;
        let (_run, id, _) = retained_fixture(&state).await;
        let error = state
            .controls
            .terminal(
                id,
                Uuid::new_v4(),
                voyage_protocol::process::TerminalOperation::Write {
                    bytes: vec![0; 65537],
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("private input frame exceeds"));
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn retained_terminal_rejects_invalid_dimensions_before_attach() {
        let (_root, state) = fixture().await;
        let (_run, id, _) = retained_fixture(&state).await;
        for (columns, rows) in [(0, 1), (1, 0), (u16::MAX, 1), (1, u16::MAX)] {
            let error = state
                .controls
                .terminal(
                    id,
                    Uuid::new_v4(),
                    voyage_protocol::process::TerminalOperation::Resize { columns, rows },
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("invalid terminal size"));
        }
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn missing_terminal_does_not_discard_retained_manager() {
        let (_root, state) = fixture().await;
        let (_run, id, _) = retained_fixture(&state).await;
        assert!(
            state
                .controls
                .terminal(
                    id,
                    Uuid::new_v4(),
                    voyage_protocol::process::TerminalOperation::Snapshot
                )
                .await
                .is_err()
        );
        assert!(state.controls.retained_tool().await.is_some());
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
    }
    async fn executed_tool(arguments: Value) -> (Value, Value) {
        let (_root, state) = fixture().await;
        let (_run, id, agent) = retained_fixture(&state).await;
        let subagents = Arc::new(
            crate::subagent::SubagentRuntime::new(
                Arc::new(NeverExecute),
                crate::subagent::RuntimeLimits::default(),
                None,
            )
            .unwrap(),
        );
        state
            .controls
            .open(id, agent, subagents, CancellationToken::new())
            .await;
        let command_id = Uuid::new_v4();
        let command = RuntimeCommand::ExecuteTool {
            command_id,
            expected_revision: state.owner.snapshot().await.unwrap().revision,
            expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 60000,
            run_id: id,
            name: "process".into(),
            arguments,
        };
        let accepted = state
            .controls
            .execute(
                state.owner.clone(),
                state.owner.session_id(),
                command.clone(),
            )
            .await
            .unwrap();
        assert_eq!(accepted["outcome"], "pending_or_unknown");
        let completed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let receipt = state
                    .owner
                    .process_receipt(command_id)
                    .await
                    .unwrap()
                    .unwrap();
                if receipt["outcome"].is_object() {
                    break receipt;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(state.controls.close().await);
        // Durable retry must work even after live control authority is removed.
        let replay = state
            .controls
            .execute(state.owner.clone(), state.owner.session_id(), command)
            .await
            .unwrap();
        assert_eq!(replay, completed);
        state
            .controls
            .shutdown_retained(&state.owner)
            .await
            .unwrap();
        (accepted, completed)
    }
    #[tokio::test]
    async fn live_process_list_completes_and_replays_after_close() {
        let (_, receipt) = executed_tool(json!({"action":"list"})).await;
        assert_eq!(receipt["outcome"]["status"], "completed");
        assert!(receipt["outcome"]["output"].is_string());
    }
    #[tokio::test]
    async fn live_private_write_is_durably_failed_not_executed() {
        let (_, receipt) =
            executed_tool(json!({"action":"write", "id":Uuid::new_v4(), "data":"not input"})).await;
        assert_eq!(receipt["outcome"]["status"], "failed");
        assert!(
            receipt["outcome"]["error"]
                .as_str()
                .unwrap()
                .contains("private terminal channel")
        );
    }
    #[tokio::test]
    async fn live_invalid_process_action_records_failure_receipt() {
        let (_, receipt) = executed_tool(json!({"action":"not_an_action"})).await;
        assert_eq!(receipt["outcome"]["status"], "failed");
        assert!(receipt["outcome"]["error"].is_string());
    }
    #[tokio::test]
    async fn idle_catalog_has_no_invented_model_support() {
        let (_root, state) = fixture().await;
        assert!(
            state
                .controls
                .known_model(&Config::default())
                .await
                .is_none()
        );
    }
}

mod submission_configuration_batch {
    use super::*;
    use serde_json::json;
    fn authorization(state: &State) -> authorization::Authorization {
        authorization::Authorization {
            owner_connection: false,
            authority: None,
            actor: state.actor,
            grant: None,
        }
    }
    fn submit(id: Uuid, expiry: u64) -> RuntimeCommand {
        RuntimeCommand::Submit {
            coordination: None,
            command_id: id,
            expected_revision: 0,
            expires_at_ms: expiry,
            prompt: "offline admission".into(),
        }
    }
    #[tokio::test]
    async fn unsupported_submission_is_refused_before_configuration() {
        let (_root, state) = fixture().await;
        assert!(
            submission::submit(&state, authorization(&state), RuntimeCommand::Health)
                .await
                .unwrap_err()
                .to_string()
                .contains("not submit")
        );
        assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    }
    #[tokio::test]
    async fn stopping_runtime_refuses_submission() {
        let (_root, state) = fixture().await;
        state.shutdown.cancel();
        let id = Uuid::new_v4();
        assert!(
            submission::submit(&state, authorization(&state), submit(id, 1))
                .await
                .unwrap_err()
                .to_string()
                .contains("runtime stopping")
        );
        assert!(state.owner.process_receipt(id).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn overflowing_submission_deadline_does_not_admit() {
        let (_root, state) = fixture().await;
        assert!(
            submission::submit(
                &state,
                authorization(&state),
                submit(Uuid::new_v4(), u64::MAX)
            )
            .await
            .is_err()
        );
        assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    }
    #[tokio::test]
    async fn excessive_content_is_refused_before_account_loading() {
        let (_root, state) = fixture().await;
        let command = RuntimeCommand::SubmitContent {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: 1,
            content: (0..17)
                .map(|_| voyage_protocol::content::ContentPart::Text {
                    text: "part".into(),
                })
                .collect(),
        };
        assert!(
            submission::submit(&state, authorization(&state), command)
                .await
                .unwrap_err()
                .to_string()
                .contains("too many content parts")
        );
        assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    }
    #[tokio::test]
    async fn accepted_submission_replay_never_needs_provider_configuration() {
        let (_root, state) = fixture().await;
        let id = Uuid::new_v4();
        let expiry = chrono::Utc::now().timestamp_millis() + 60000;
        let admitted = state
            .owner
            .admit(crate::attachment::journal::TurnAdmission {
                coordination: None,
                operator_name: None,
                command_id: id,
                machine_id: state.actor.installation_id,
                principal_id: state.actor.principal_id,
                session_id: state.owner.session_id(),
                expected_revision: 0,
                expires_at_ms: expiry,
                prompt: "offline admission".into(),
                parts: vec![],
            })
            .await
            .unwrap();
        let crate::attachment::runtime::Admission::New(run) = admitted else {
            panic!("fresh admission")
        };
        let record = run.record().await.unwrap();
        let before = state.owner.snapshot().await.unwrap().revision;
        let value = submission::submit(&state, authorization(&state), submit(id, expiry as u64))
            .await
            .unwrap();
        assert_eq!(value["status"], "accepted");
        assert_eq!(value["duplicate"], true);
        assert_eq!(value["run_id"], record.id.to_string());
        assert_eq!(state.owner.snapshot().await.unwrap().revision, before);
    }
    #[tokio::test]
    async fn unsupported_configuration_variant_is_refused() {
        let (_root, state) = fixture().await;
        assert!(
            configuration::configure(&state, RuntimeCommand::Health, authorization(&state))
                .await
                .unwrap_err()
                .to_string()
                .contains("not configure")
        );
        assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    }
    #[tokio::test]
    async fn relative_configuration_path_is_not_loaded() {
        let (_root, state) = fixture().await;
        let command = RuntimeCommand::Configure {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: 1,
            config_path: "relative.toml".into(),
        };
        assert!(
            configuration::configure(&state, command, authorization(&state))
                .await
                .unwrap_err()
                .to_string()
                .contains("configuration path must be absolute")
        );
        assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    }
    #[tokio::test]
    async fn missing_absolute_configuration_preserves_live_model() {
        let (root, state) = fixture().await;
        let before = state.config.read().await.model.clone();
        let command = RuntimeCommand::Configure {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: 1,
            config_path: root.path().join("missing.toml"),
        };
        assert!(
            configuration::configure(&state, command, authorization(&state))
                .await
                .is_err()
        );
        assert_eq!(state.config.read().await.model, before);
        assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    }
    fn account_command(revision: u64, deadline: u64) -> RuntimeCommand {
        RuntimeCommand::SetAccountInference { command_id: Uuid::new_v4(), expected_revision: revision, expires_at_ms: deadline,
            model: "fixture".into(), reasoning_effort: None, service_tier: None,
            account: serde_json::from_value(json!({"account_id":Uuid::new_v4(),"connection_id":Uuid::new_v4(),"identity_generation":0,"connection_revision":0,"transport":"openai_responses"})).unwrap() }
    }
    #[tokio::test]
    async fn expired_account_selection_is_refused_before_lookup() {
        let (_root, state) = fixture().await;
        assert!(
            configuration::configure(&state, account_command(0, 0), authorization(&state))
                .await
                .unwrap_err()
                .to_string()
                .contains("invalid account selection deadline")
        );
        assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    }
    #[tokio::test]
    async fn far_future_account_selection_is_refused_before_lookup() {
        let (_root, state) = fixture().await;
        assert!(
            configuration::configure(&state, account_command(0, u64::MAX), authorization(&state))
                .await
                .unwrap_err()
                .to_string()
                .contains("invalid account selection deadline")
        );
    }
    #[tokio::test]
    async fn stale_account_selection_is_refused_before_lookup() {
        let (_root, state) = fixture().await;
        let expiry = chrono::Utc::now().timestamp_millis() as u64 + 60000;
        assert!(
            configuration::configure(&state, account_command(1, expiry), authorization(&state))
                .await
                .unwrap_err()
                .to_string()
                .contains("session revision conflict")
        );
        assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    }
}
