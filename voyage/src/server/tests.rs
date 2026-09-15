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
