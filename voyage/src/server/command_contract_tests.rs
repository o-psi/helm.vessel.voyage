//! Route the ordinary command classes through immutable dispatch admission.
//! These cases use owned files, synthetic HTTP and actual journal receipts.
use super::family_fixture::*;
use super::*;
use serde_json::json;
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use voyage_protocol::process::{GrantBinding, RuntimeCommand, WorkspaceChangeScope};

#[derive(Debug)]
struct Authority {
    valid: AtomicBool,
    checks: AtomicUsize,
}
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> Result<()> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        ensure!(
            self.valid.load(Ordering::SeqCst),
            "fixture authority withdrawn"
        );
        Ok(())
    }
}
fn authorized(state: &State, authority: Arc<Authority>) -> authorization::Authorization {
    let mut a = auth(state);
    a.authority = Some(authority);
    a
}
fn scoped(state: &State) -> authorization::Authorization {
    let mut a = auth(state);
    a.grant = Some(GrantBinding {
        grant_id: Uuid::new_v4(),
        principal_id: state.actor.principal_id,
        revision: 1,
    });
    a
}
fn rename(revision: u64, name: &str) -> RuntimeCommand {
    RuntimeCommand::Rename {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        name: name.into(),
    }
}

#[tokio::test]
async fn workspace_routes_apply_policy_and_recheck_authority_before_disclosure() {
    let (root, state, _provider) = family_fixture::fixture().await;
    std::fs::write(root.path().join("owned.txt"), "canonical 世界\n").unwrap();
    let before = state.owner.snapshot().await.unwrap().revision;
    let file = call(
        &state,
        RuntimeCommand::WorkspaceFile {
            path: "owned.txt".into(),
        },
    )
    .await
    .unwrap();
    assert!(file.to_string().contains("canonical"));
    let changes = call(
        &state,
        RuntimeCommand::WorkspaceChanges {
            scope: WorkspaceChangeScope::Status,
            path: None,
        },
    )
    .await
    .unwrap();
    assert!(changes.is_object());
    for command in [
        RuntimeCommand::WorkspaceFile {
            path: "../outside".into(),
        },
        RuntimeCommand::WorkspaceChanges {
            scope: WorkspaceChangeScope::Unstaged,
            path: Some("../outside".into()),
        },
    ] {
        assert!(call(&state, command).await.is_err());
    }
    let authority = Arc::new(Authority {
        valid: AtomicBool::new(false),
        checks: AtomicUsize::new(0),
    });
    assert!(
        commands::dispatch_admitted(
            &state,
            RuntimeCommand::WorkspaceFile {
                path: "owned.txt".into()
            },
            authorized(&state, authority.clone())
        )
        .await
        .is_err()
    );
    assert!(
        commands::dispatch_admitted(
            &state,
            RuntimeCommand::WorkspaceChanges {
                scope: WorkspaceChangeScope::Status,
                path: None
            },
            authorized(&state, authority.clone())
        )
        .await
        .is_err()
    );
    assert_eq!(authority.checks.load(Ordering::SeqCst), 2);
    assert_eq!(state.owner.snapshot().await.unwrap().revision, before);
    assert!(state.active.lock().await.is_none());
}

#[tokio::test]
async fn goal_routes_preserve_human_authority_exact_receipts_and_idle_reconciliation() {
    use voyage_protocol::goals::*;
    let (_root, state, _provider) = family_fixture::fixture().await;
    assert!(call(&state, RuntimeCommand::GoalRead).await.unwrap()["goal"].is_null());
    let command = RuntimeCommand::GoalUpdate {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        action: GoalAction::Set {
            objective: "Owned user-authored fixture objective".into(),
            limits: GoalLimits::default(),
            replace_goal_id: None,
            continue_automatically: false,
        },
    };
    assert!(
        commands::dispatch_admitted(&state, command.clone(), scoped(&state))
            .await
            .is_err()
    );
    let authority = Arc::new(Authority {
        valid: AtomicBool::new(false),
        checks: AtomicUsize::new(0),
    });
    assert!(
        commands::dispatch_admitted(
            &state,
            command.clone(),
            authorized(&state, authority.clone())
        )
        .await
        .is_err()
    );
    let first = call(&state, command.clone()).await.unwrap();
    assert_eq!(call(&state, command).await.unwrap(), first);
    let goal = state.owner.goal().await.unwrap();
    assert_eq!(
        goal.goal.as_ref().unwrap().objective,
        "Owned user-authored fixture objective"
    );
    assert!(!goal.goal.unwrap().continuation_authorized);
    for limit in [0, 129] {
        assert!(
            call(
                &state,
                RuntimeCommand::GoalReconcile {
                    offset: 0,
                    limit,
                    fence_children: false
                }
            )
            .await
            .is_err()
        );
    }
    assert!(
        commands::dispatch_admitted(
            &state,
            RuntimeCommand::GoalReconcile {
                offset: 0,
                limit: 10,
                fence_children: true
            },
            scoped(&state)
        )
        .await
        .is_err()
    );
    assert!(
        commands::dispatch_admitted(
            &state,
            RuntimeCommand::GoalReconcile {
                offset: 0,
                limit: 10,
                fence_children: true
            },
            authorized(&state, authority)
        )
        .await
        .is_err()
    );
    let reconciled = call(
        &state,
        RuntimeCommand::GoalReconcile {
            offset: 0,
            limit: 10,
            fence_children: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(reconciled["effects_replayed"], false);
    assert_eq!(reconciled["continuation_restored"], false);
    let events = call(
        &state,
        RuntimeCommand::NotificationEvents {
            after: 0,
            limit: 10,
        },
    )
    .await
    .unwrap();
    assert!(events.is_object());
    assert!(
        call(
            &state,
            RuntimeCommand::NotificationEvents { after: 0, limit: 0 }
        )
        .await
        .is_err()
    );
    assert!(state.active.lock().await.is_none());
}

#[tokio::test]
async fn browser_routes_fence_socket_policy_revision_and_withdrawn_authority() {
    use voyage_protocol::{
        browser::*,
        host_browser::{HostBrowserOperation, HostBrowserSocket},
    };
    let (_root, state, _provider) = family_fixture::fixture().await;
    let nil = HostBrowserSocket {
        socket_id: Uuid::nil(),
    };
    assert!(
        call(
            &state,
            RuntimeCommand::HostBrowser {
                operation: HostBrowserOperation::Status {},
                socket: nil
            }
        )
        .await
        .is_err()
    );
    let socket = HostBrowserSocket {
        socket_id: Uuid::new_v4(),
    };
    let status = call(
        &state,
        RuntimeCommand::HostBrowser {
            operation: HostBrowserOperation::Status {},
            socket,
        },
    )
    .await
    .unwrap();
    assert!(status.is_object());
    state.config.write().await.access = Some(crate::config::AccessMode::ReadOnly);
    let start = || RuntimeCommand::HostBrowser {
        operation: HostBrowserOperation::Start {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            incarnation: state.registration.incarnation,
        },
        socket,
    };
    assert!(call(&state, start()).await.is_err());
    state.config.write().await.access = Some(crate::config::AccessMode::Unrestricted);
    assert!(
        call(
            &state,
            RuntimeCommand::HostBrowser {
                operation: HostBrowserOperation::Start {
                    command_id: Uuid::new_v4(),
                    expected_revision: 99,
                    incarnation: state.registration.incarnation
                },
                socket
            }
        )
        .await
        .is_err()
    );
    // No actual worker is supplied. The missing executor must fail boundedly.
    assert!(call(&state, start()).await.is_err());
    assert_eq!(
        call(&state, RuntimeCommand::HostBrowserDisconnected { socket })
            .await
            .unwrap()["disconnected"],
        true
    );
    let authority = Arc::new(Authority {
        valid: AtomicBool::new(false),
        checks: AtomicUsize::new(0),
    });
    assert!(
        commands::dispatch_admitted(
            &state,
            RuntimeCommand::PrepareBrowser,
            authorized(&state, authority)
        )
        .await
        .is_err()
    );
    assert_eq!(
        call(&state, RuntimeCommand::PrepareBrowser).await.unwrap()["prepared"],
        true
    );
    let binding = BrowserBinding {
        session_id: state.registration.session_id,
        incarnation: state.registration.incarnation,
        run_id: None,
        browser_id: Uuid::new_v4(),
        resource_id: Uuid::new_v4(),
        executor_id: Uuid::new_v4(),
        controller_epoch: 1,
        capture_epoch: 1,
        expires_at_ms: expiry().min((chrono::Utc::now().timestamp_millis() + 30_000) as u64),
    };
    let offered = call(
        &state,
        RuntimeCommand::Browser {
            operation: BrowserOperation::Offer {
                command_id: Uuid::new_v4(),
                binding: binding.clone(),
            },
        },
    )
    .await
    .unwrap();
    assert!(offered.is_object());
    let pending = call(
        &state,
        RuntimeCommand::Browser {
            operation: BrowserOperation::Pending {
                binding: binding.clone(),
                limit: 10,
            },
        },
    )
    .await
    .unwrap();
    assert!(pending.is_object());
    let rejected = call(
        &state,
        RuntimeCommand::Browser {
            operation: BrowserOperation::Cleanup {
                command_id: Uuid::new_v4(),
                binding,
                request_id: Uuid::new_v4(),
                observed: true,
            },
        },
    )
    .await;
    assert!(rejected.is_err());
    state.browser.finish_run().unwrap();
    assert!(state.active.lock().await.is_none());
}

#[tokio::test]
async fn artifact_route_reads_exact_binary_or_image_and_rejects_deleted_session() {
    use base64::Engine;
    let (_root, state, _provider) = family_fixture::fixture().await;
    let artifact = Uuid::new_v4();
    let bytes = b"\0owned binary\xff";
    crate::artifacts::Store::open(
        &state.directory.join("journal"),
        state.registration.session_id,
    )
    .unwrap()
    .put(artifact, "owned.bin", "application/octet-stream", bytes)
    .unwrap();
    let chunk = call(
        &state,
        RuntimeCommand::ReadArtifact {
            artifact_id: artifact,
            offset: 0,
            limit: 100,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(chunk["data_base64"].as_str().unwrap())
            .unwrap(),
        bytes.to_vec()
    );
    let image = Uuid::new_v4();
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(1, 1)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    crate::images::Store::open(
        &state.directory.join("journal"),
        state.registration.session_id,
    )
    .unwrap()
    .put(
        state.actor.principal_id,
        image,
        "owned.png",
        encoded.get_ref(),
    )
    .unwrap();
    let chunk = call(
        &state,
        RuntimeCommand::ReadArtifact {
            artifact_id: image,
            offset: 0,
            limit: 1000,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(chunk["data_base64"].as_str().unwrap())
            .unwrap(),
        *encoded.get_ref()
    );
    assert!(
        call(
            &state,
            RuntimeCommand::ReadArtifact {
                artifact_id: artifact,
                offset: 99,
                limit: 10
            }
        )
        .await
        .is_err()
    );
    let revision = state.owner.snapshot().await.unwrap().revision;
    call(
        &state,
        RuntimeCommand::Delete {
            command_id: Uuid::new_v4(),
            expected_revision: revision,
            expires_at_ms: expiry(),
            confirm_session_id: state.registration.session_id,
        },
    )
    .await
    .unwrap();
    assert!(
        call(
            &state,
            RuntimeCommand::ReadArtifact {
                artifact_id: artifact,
                offset: 0,
                limit: 10
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn resolve_routes_fence_delayed_delivery_and_never_admit_private_terminal_input() {
    let (_root, state, _provider) = family_fixture::fixture().await;
    let command = rename(0, "must remain unadmitted");
    let id = command.mutation_id().unwrap();
    let receipt = call(
        &state,
        RuntimeCommand::Resolve {
            command_id: id,
            original: Some(Box::new(command.clone())),
        },
    )
    .await
    .unwrap();
    assert_eq!(receipt["status"], "not_admitted");
    let error = call(&state, command).await.unwrap_err();
    let retained = error.downcast_ref::<dispatch::Rejected>().unwrap();
    assert_eq!(retained.0, receipt);
    assert!(format!("{retained}").contains("not admitted"));
    for original in [
        RuntimeCommand::ExecuteTool {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: expiry(),
            run_id: Uuid::new_v4(),
            name: "process".into(),
            arguments: json!({"action":"write","data":"synthetic private terminal input"}),
        },
        RuntimeCommand::OperatorTool {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: expiry(),
            name: "process".into(),
            arguments: json!({"action":"write","data":"synthetic private terminal input"}),
        },
    ] {
        let id = original.mutation_id().unwrap();
        assert!(
            call(
                &state,
                RuntimeCommand::Resolve {
                    command_id: id,
                    original: Some(Box::new(original))
                }
            )
            .await
            .is_err()
        );
        assert!(state.owner.process_receipt(id).await.unwrap().is_none());
    }
    assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
}

#[tokio::test]
async fn long_poll_returns_current_projection_and_shutdown_without_waking_execution() {
    let (_root, state, _provider) = family_fixture::fixture().await;
    for projection in [
        None,
        Some("public-v1"),
        Some(voyage_protocol::live_events::PROJECTION),
    ] {
        let first = observations::observe(&state, 0, 100, 0, projection)
            .await
            .unwrap();
        let cursor = first["latest_cursor"].as_u64().unwrap();
        let reader = {
            let state = state.clone();
            let projection = projection.map(str::to_owned);
            tokio::spawn(async move {
                observations::observe(&state, cursor, 100, 1000, projection.as_deref()).await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        let revision = state.owner.snapshot().await.unwrap().revision;
        call(&state, rename(revision, "long-poll publication"))
            .await
            .unwrap();
        let page = reader.await.unwrap().unwrap();
        assert!(!page["events"].as_array().unwrap().is_empty());
        assert!(page["cursor"].as_u64().unwrap() > cursor);
    }
    assert!(
        observations::observe(&state, 0, 10, 10_001, None)
            .await
            .is_err()
    );
    assert!(
        observations::observe(&state, 0, 10, 0, Some("untrusted-projection"))
            .await
            .is_err()
    );
    let page = observations::observe(&state, 0, 100, 0, None)
        .await
        .unwrap();
    let cursor = page["latest_cursor"].as_u64().unwrap();
    let waiting = {
        let state = state.clone();
        tokio::spawn(async move { observations::observe(&state, cursor, 100, 10_000, None).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    state.shutdown.cancel();
    let observed = tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(observed["events"].as_array().unwrap().is_empty());
    assert!(state.active.lock().await.is_none());
}

#[tokio::test]
async fn private_workflow_input_is_volatile_and_failed_dispatch_discards_only_its_handle() {
    let (root, state, provider) = configured(vec![]).await;
    std::fs::write(root.path().join("owned.toml"),"schema_version=1\nid=\"owned\"\nversion=\"1\"\ndescription=\"Owned fixture\"\nprompt=\"Private {{key}}\"\n[parameters.key]\ntype=\"string\"\nsecret=true\nrequired=true\n").unwrap();
    let preview = call(
        &state,
        RuntimeCommand::WorkflowPreview {
            id: "owned".into(),
            scope: Some("user".into()),
            user_directory: Some(root.path().into()),
            inputs: vec![],
            trust_digest: None,
            optional_secret_names: None,
        },
    )
    .await
    .unwrap();
    assert!(!preview.to_string().contains("synthetic-private-input"));
    let input = Uuid::new_v4();
    let other = Uuid::new_v4();
    for id in [input, other] {
        let value = call(
            &state,
            RuntimeCommand::WorkflowInputs {
                input_id: id,
                values: vec![("key".into(), "synthetic-private-input".into())],
            },
        )
        .await
        .unwrap();
        assert_eq!(value["storage"], "volatile");
        assert_eq!(value["replay"], "never");
    }
    let make = |id| RuntimeCommand::WorkflowSubmit {
        command_id: id,
        expected_revision: 99,
        expires_at_ms: expiry(),
        id: "owned".into(),
        scope: Some("user".into()),
        user_directory: Some(root.path().into()),
        inputs: vec![],
        trust_digest: None,
        private_inputs_id: Some(input),
    };
    assert!(
        commands::dispatch_admitted(&state, make(Uuid::new_v4()), scoped(&state))
            .await
            .is_err()
    );
    let command = make(Uuid::new_v4());
    let id = command.mutation_id().unwrap();
    let error = call(&state, command.clone()).await.unwrap_err();
    assert!(error.downcast_ref::<dispatch::Rejected>().is_some());
    assert!(state.workflows.pending().await);
    assert!(
        state
            .workflows
            .prepare(root.path(), state.actor.principal_id, &make(Uuid::new_v4()))
            .await
            .is_err()
    );
    let retained = call(&state, RuntimeCommand::Receipt { command_id: id })
        .await
        .unwrap();
    assert_eq!(retained["status"], "rejected");
    assert!(!retained.to_string().contains("synthetic-private-input"));
    assert_eq!(
        call(&state, command)
            .await
            .unwrap_err()
            .downcast_ref::<dispatch::Rejected>()
            .unwrap()
            .0,
        retained
    );
    state.workflows.clear().await;
    assert!(!state.workflows.pending().await);
    assert!(provider.requests.lock().await.is_empty());
    assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
}

#[tokio::test]
async fn workflow_translation_replays_completed_admission_without_consuming_private_input_again() {
    let (root, state, provider) = configured(vec![Reply::held("Owned workflow answer")]).await;
    std::fs::write(root.path().join("plain.toml"),"schema_version=1\nid=\"plain\"\nversion=\"1\"\ndescription=\"Owned fixture\"\nprompt=\"Synthetic workflow prompt\"\n").unwrap();
    let command = RuntimeCommand::WorkflowSubmit {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        id: "plain".into(),
        scope: Some("user".into()),
        user_directory: Some(root.path().into()),
        inputs: vec![],
        trust_digest: None,
        private_inputs_id: None,
    };
    let first = call(&state, command.clone()).await.unwrap();
    assert_eq!(first["status"], "accepted");
    provider.wait_requests(1).await;
    let replay = call(&state, command).await.unwrap();
    assert_eq!(replay["run_id"], first["run_id"]);
    assert_eq!(provider.requests.lock().await.len(), 1);
    provider.release.cancel();
    let saved = finish(&state).await;
    assert_eq!(saved["run"]["state"], "completed");
    assert!(saved["pending_cleanup_run"].is_null());
}

#[tokio::test]
async fn configuration_routes_publish_only_durable_current_settings_and_preserve_private_provenance()
 {
    let (root, state, provider) = configured(vec![]).await;
    let access = RuntimeCommand::SetAccess {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        access: "read-only".into(),
    };
    let accepted = call(&state, access.clone()).await.unwrap();
    assert_eq!(call(&state, access).await.unwrap(), accepted);
    assert_eq!(
        state.config.read().await.access,
        Some(crate::config::AccessMode::ReadOnly)
    );
    let revision = state.owner.snapshot().await.unwrap().revision;
    let inference = RuntimeCommand::SetInference {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        model: "fixture-next".into(),
        reasoning_effort: None,
        service_tier: None,
    };
    let accepted = call(&state, inference.clone()).await.unwrap();
    assert_eq!(call(&state, inference).await.unwrap(), accepted);
    assert_eq!(state.config.read().await.model, "fixture-next");
    let config_path = root.path().join("replacement.json");
    let mut replacement = provider.config();
    replacement.model = "fixture".into();
    let launch =
        crate::launch_config::LaunchConfig::capture(&replacement, &state.registration.workspace)
            .unwrap();
    std::fs::write(&config_path, serde_json::to_vec(&launch).unwrap()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let revision = state.owner.snapshot().await.unwrap().revision;
    let configure = RuntimeCommand::Configure {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        config_path: config_path.clone(),
    };
    let accepted = call(&state, configure.clone()).await.unwrap();
    assert_eq!(call(&state, configure).await.unwrap(), accepted);
    assert_eq!(state.config.read().await.model, "fixture");
    assert!(
        state
            .config
            .read()
            .await
            .extension_private_files
            .iter()
            .any(|entry| entry.path == config_path)
    );
    assert!(provider.requests.lock().await.is_empty());
    assert!(state.active.lock().await.is_none());
}

#[tokio::test]
async fn supported_idle_observations_and_missing_live_controls_never_start_a_run() {
    let (_root, state, provider) = configured(vec![]).await;
    for command in [
        RuntimeCommand::Health,
        RuntimeCommand::Snapshot,
        RuntimeCommand::ProviderAttempts {
            run_id: None,
            offset: 0,
            limit: 10,
            expected_revision: Some(0),
        },
        RuntimeCommand::History {
            offset: 0,
            limit: 10,
            expected_revision: Some(0),
        },
        RuntimeCommand::Controls {
            run_id: None,
            section: "policy".into(),
        },
        RuntimeCommand::Controls {
            run_id: None,
            section: "files".into(),
        },
        RuntimeCommand::Controls {
            run_id: None,
            section: "models".into(),
        },
        RuntimeCommand::Decisions,
    ] {
        assert!(call(&state, command).await.is_ok());
    }
    for command in [
        RuntimeCommand::RunOutput {
            run_id: Uuid::new_v4(),
            offset: 0,
            limit: 10,
        },
        RuntimeCommand::MessageChunk {
            index: 0,
            offset: 0,
            limit: 10,
            expected_revision: 0,
        },
        RuntimeCommand::Terminal {
            run_id: Uuid::new_v4(),
            terminal_id: Uuid::new_v4(),
            operation: voyage_protocol::process::TerminalOperation::Attach,
        },
        RuntimeCommand::ExecuteTool {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: expiry(),
            run_id: Uuid::new_v4(),
            name: "read_file".into(),
            arguments: json!({"path":"owned.txt"}),
        },
        RuntimeCommand::Controls {
            run_id: None,
            section: "unknown".into(),
        },
        RuntimeCommand::AssignmentObserve {
            run_id: Uuid::new_v4(),
            assignment_id: Uuid::new_v4(),
            participant: "missing".into(),
            cancel: false,
        },
    ] {
        assert!(call(&state, command).await.is_err());
    }
    assert_eq!(state.owner.snapshot().await.unwrap().revision, 0);
    assert!(state.active.lock().await.is_none());
    assert!(provider.requests.lock().await.is_empty());
}

#[tokio::test]
async fn operator_and_live_tool_routes_use_the_actual_registry_with_one_exact_effect_receipt() {
    let (root, state, provider) = configured(vec![]).await;
    std::fs::write(root.path().join("owned.txt"), "Owned operator file 世界\n").unwrap();
    let command = RuntimeCommand::OperatorTool {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        name: "read_file".into(),
        arguments: json!({"path":"owned.txt"}),
    };
    let first = call(&state, command.clone()).await.unwrap();
    assert_eq!(first["status"], "accepted");
    let replay = call(&state, command).await.unwrap();
    assert_eq!(replay["run_id"], first["run_id"]);
    let snapshot = finish(&state).await;
    assert_eq!(snapshot["run"]["state"], "completed");
    assert!(snapshot["pending_cleanup_run"].is_null());
    assert!(provider.requests.lock().await.is_empty());
    let (root, state, provider) = configured(vec![Reply::held("controlled inference")]).await;
    std::fs::write(root.path().join("owned.txt"), "Owned live-control file\n").unwrap();
    let accepted = call(
        &state,
        submit(0, "Held activity for exact operator controls"),
    )
    .await
    .unwrap();
    let run: Uuid = accepted["run_id"].as_str().unwrap().parse().unwrap();
    provider.wait_requests(1).await;
    let revision = state.owner.snapshot().await.unwrap().revision;
    let execute = RuntimeCommand::ExecuteTool {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        run_id: run,
        name: "read_file".into(),
        arguments: json!({"path":"owned.txt"}),
    };
    let id = execute.mutation_id().unwrap();
    let accepted = call(&state, execute.clone()).await.unwrap();
    assert_eq!(accepted["outcome"], "pending_or_unknown");
    let first = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let receipt = state.owner.process_receipt(id).await.unwrap().unwrap();
            if receipt["outcome"].is_object() {
                break receipt;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(first["outcome"]["status"], "completed");
    assert!(first.to_string().contains("Owned live-control file"));
    assert_eq!(call(&state, execute).await.unwrap(), first);
    let cancel = RuntimeCommand::Cancel {
        command_id: Uuid::new_v4(),
        expected_revision: state.owner.snapshot().await.unwrap().revision,
        expires_at_ms: expiry(),
        run_id: run,
    };
    call(&state, cancel).await.unwrap();
    provider.wait_disconnected().await;
    let snapshot = finish(&state).await;
    assert!(snapshot["pending_cleanup_run"].is_null());
    assert_eq!(provider.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn github_route_refuses_scope_bounds_and_runs_owner_local_references_without_provider() {
    let (_root, state, _provider) = family_fixture::fixture().await;
    let make = |words| RuntimeCommand::Github {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        words,
    };
    assert!(
        commands::dispatch_admitted(&state, make(vec!["references".into()]), scoped(&state))
            .await
            .is_err()
    );
    for words in [vec![], vec!["x".into(); 65], vec!["x".repeat(65537)]] {
        assert!(call(&state, make(words)).await.is_err());
    }
    let command = make(vec!["references".into()]);
    let first = call(&state, command.clone()).await.unwrap();
    assert_eq!(first["status"], "accepted");
    let replay = call(&state, command).await.unwrap();
    assert_eq!(replay["run_id"], first["run_id"]);
    let completed = finish(&state).await;
    assert_eq!(completed["run"]["state"], "completed");
    assert!(completed["pending_cleanup_run"].is_null());
}
