//! Exercise owner mutex/spawn-blocking adapters without providers or executors.
use super::*;
use serde_json::json;
use voyage_protocol::process::RuntimeCommand;

async fn fixture() -> (tempfile::TempDir, ManagedSessionOwner, Uuid) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("journal");
    let mut journal = Journal::open(directory.clone()).unwrap();
    let session = crate::session::Session::new(root.path().into(), "fixture".into());
    journal.create_session(&session).unwrap();
    drop(journal);
    let owner = ManagedSessionOwner::open(directory, session.id)
        .await
        .unwrap();
    owner.initialize_process_commands().await.unwrap();
    owner.initialize_session_resources().await.unwrap();
    owner
        .initialize_command_bindings(Uuid::new_v4())
        .await
        .unwrap();
    (root, owner, session.id)
}
fn expiry() -> u64 {
    (chrono::Utc::now().timestamp_millis() + 60000) as u64
}

#[tokio::test]
async fn owner_metadata_and_history_observe_the_same_revision() {
    let (_root, owner, session) = fixture().await;
    let command = RuntimeCommand::Rename {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        name: "through owner".into(),
    };
    let id = match &command {
        RuntimeCommand::Rename { command_id, .. } => *command_id,
        _ => unreachable!(),
    };
    assert!(owner.process_receipt(id).await.unwrap().is_none());
    let receipt = owner
        .process_metadata(actor(), command.clone())
        .await
        .unwrap();
    assert_eq!(receipt["revision"], 1);
    assert_eq!(
        owner.process_receipt(id).await.unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        owner.process_metadata(actor(), command).await.unwrap(),
        receipt
    );
    let snapshot = owner.process_snapshot().await.unwrap();
    assert_eq!(snapshot["revision"], 1);
    let history = owner.process_history(0, 128, Some(1)).await.unwrap();
    assert_eq!(history["session_id"], session.to_string());
    assert_eq!(history["total_messages"], 0);
    assert_eq!(history["has_more"], false);
    assert!(owner.process_history(0, 1, Some(0)).await.is_err());
    assert!(owner.process_history(1, 1, None).await.is_err());
    assert!(owner.process_history(0, 0, None).await.is_err());
    assert!(owner.process_history(0, 129, None).await.is_err());
    assert!(owner.process_message_chunk(0, 0, 100, 1).await.is_err());
    let attempts = owner
        .process_provider_attempts(None, 0, 32, Some(1))
        .await
        .unwrap();
    assert_eq!(attempts["total"], 0);
    assert!(
        owner
            .process_provider_attempts(None, 0, 0, None)
            .await
            .is_err()
    );
    assert!(
        owner
            .process_provider_attempts(None, 0, 33, None)
            .await
            .is_err()
    );
    assert!(
        owner
            .process_provider_attempts(None, 0, 1, Some(0))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn owner_configuration_is_durable_and_access_revision_is_checked() {
    let (root, owner, session) = fixture().await;
    assert!(owner.saved_configuration().await.unwrap().is_none());
    owner.check_access_revision(0).await.unwrap();
    assert!(owner.check_access_revision(1).await.is_err());
    let command = RuntimeCommand::Configure {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        config_path: root.path().join("private.toml"),
    };
    let receipt = owner
        .configure(
            command.clone(),
            "private serialized settings".into(),
            "new-model".into(),
        )
        .await
        .unwrap();
    assert_eq!(receipt["revision"], 1);
    assert_eq!(
        owner.saved_configuration().await.unwrap().as_deref(),
        Some("private serialized settings")
    );
    assert!(owner.check_access_revision(0).await.is_err());
    owner.check_access_revision(1).await.unwrap();
    assert_eq!(
        owner
            .configure(command, "retry must not overwrite".into(), "other".into())
            .await
            .unwrap(),
        receipt
    );
    drop(owner);
    let reopened = ManagedSessionOwner::open(root.path().join("journal"), session)
        .await
        .unwrap();
    assert_eq!(
        reopened.saved_configuration().await.unwrap().as_deref(),
        Some("private serialized settings")
    );
}

#[tokio::test]
async fn owner_transfer_exposes_exact_durable_artifact_on_retry() {
    let (_root, owner, _session) = fixture().await;
    let command = RuntimeCommand::Relinquish {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        transfer_id: Uuid::new_v4(),
        destination_vessel_id: Uuid::new_v4(),
        prepare_digest: "b".repeat(64),
    };
    let (receipt, artifact) = owner.relinquish(command.clone()).await.unwrap();
    assert_eq!(receipt["status"], "relinquished");
    assert_eq!(receipt["artifact_bytes"], artifact.len());
    assert_eq!(
        owner.relinquish(command).await.unwrap(),
        (receipt, artifact)
    );
}

#[tokio::test]
async fn owner_controls_and_message_reads_do_not_replay_work() {
    let (_root, owner, run) = super::checkpoint_tests::fixture_authorized(None).await;
    owner.initialize_session_resources().await.unwrap();
    let snapshot = owner.process_snapshot().await.unwrap();
    let revision = snapshot["revision"].as_u64().unwrap();
    let run_id = run.run_id;
    let command = RuntimeCommand::ExecuteTool {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        run_id,
        name: "read_file".into(),
        arguments: json!({"path":"local"}),
    };
    let id = match &command {
        RuntimeCommand::ExecuteTool { command_id, .. } => *command_id,
        _ => unreachable!(),
    };
    assert!(
        owner
            .control_receipt(command.clone())
            .await
            .unwrap()
            .is_none()
    );
    let (receipt, dispatch) = owner.admit_control(command.clone()).await.unwrap();
    assert!(dispatch);
    assert_eq!(
        owner.admit_control(command.clone()).await.unwrap(),
        (receipt, false)
    );
    owner
        .complete_control(id, json!({"text":"saved"}))
        .await
        .unwrap();
    assert_eq!(
        owner.control_receipt(command).await.unwrap().unwrap()["outcome"]["text"],
        "saved"
    );
    assert!(owner.complete_control(id, json!("replace")).await.is_err());
    let history = owner.process_history(0, 1, Some(revision)).await.unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 1);
    let chunk = owner
        .process_message_chunk(0, 0, 4096, revision)
        .await
        .unwrap();
    assert!(chunk.to_string().contains("checkpoint fixture"));
    assert!(
        owner
            .process_message_chunk(0, 0, 4096, revision + 1)
            .await
            .is_err()
    );
    assert!(
        owner
            .process_run_output(Uuid::new_v4(), 0, 100)
            .await
            .is_err()
    );
    // This fixture has no executor/resources; terminalize explicitly rather than
    // leaving its synthetic active run to owner-drop recovery.
    run.storage(|store| {
        store.journal.finish(
            &store.guard,
            store.run_id,
            RunState::Cancelled,
            Some("test cancellation"),
            None,
        )
    })
    .await
    .unwrap();
}

fn actor() -> crate::attachment::local_actor::LocalActor {
    crate::attachment::local_actor::LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    }
}

#[tokio::test]
async fn owner_materializes_legacy_account_once_without_changing_other_settings() {
    let (_root, owner, _session) = fixture().await;
    let command = RuntimeCommand::Configure {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        config_path: "private".into(),
    };
    owner
        .configure(
            command,
            r#"{"config":{"model":"fixture"},"workspace":"unchanged"}"#.into(),
            "fixture".into(),
        )
        .await
        .unwrap();
    let account = voyage_protocol::accounts::AccountBinding {
        account_id: Uuid::new_v4(),
        connection_id: Uuid::new_v4(),
        identity_generation: 1,
        connection_revision: 2,
        transport: voyage_protocol::accounts::Transport::OpenaiResponses,
    };
    owner
        .materialize_account_configuration(&account)
        .await
        .unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&owner.saved_configuration().await.unwrap().unwrap()).unwrap();
    assert_eq!(saved["workspace"], "unchanged");
    assert_eq!(saved["config"]["model"], "fixture");
    assert_eq!(
        saved["config"]["account"],
        serde_json::to_value(&account).unwrap()
    );
    assert!(
        owner
            .materialize_account_configuration(&account)
            .await
            .is_err()
    );
    owner.check_access_revision(1).await.unwrap();
}
