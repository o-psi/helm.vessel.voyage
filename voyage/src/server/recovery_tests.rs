//! Offline recovery uses real private journals and OS locks, never an executor.
use super::*;
use recovery::{RecoverArgs, recover};
use serde_json::{Value, json};

async fn fixture() -> (tempfile::TempDir, PathBuf, ProcessRegistration) {
    let (root, state) = tests::fixture().await;
    let directory = state.directory.clone();
    let mut registration = state.registration.clone();
    registration.token = "synthetic-offline-recovery-token-32-bytes".into();
    recovery::persist(
        &directory.join("registration.json"),
        &serde_json::to_value(&registration).unwrap(),
    )
    .unwrap();
    drop(state); // release the execution fence before offline recovery
    (root, directory, registration)
}
fn args(
    directory: &std::path::Path,
    registration: &ProcessRegistration,
    command_id: Uuid,
) -> RecoverArgs {
    RecoverArgs {
        directory: directory.to_owned(),
        session: registration.session_id,
        incarnation: registration.incarnation,
        command_id,
        acknowledge_cleanup: None,
        acknowledge_resources: vec![],
        reconcile_tools: None,
        expected_revision: None,
    }
}
fn saved(path: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[tokio::test]
async fn offline_recovery_publishes_exact_receipt_and_replays_without_execution() {
    let (_root, directory, registration) = fixture().await;
    let command = Uuid::new_v4();
    std::fs::write(directory.join("runtime.sock"), b"stale endpoint").unwrap();
    let result = recover(args(&directory, &registration, command))
        .await
        .unwrap();
    assert_eq!(result["restart_permitted"], true);
    assert_eq!(result["cleanup_disposition"], "observed");
    assert_eq!(result["session_id"], registration.session_id.to_string());
    assert!(!directory.join("runtime.sock").exists());
    assert_eq!(saved(&directory.join("recovered.json")), result);
    assert_eq!(
        saved(&directory.join(format!("recoveries/{command}.json"))),
        result
    );
    std::fs::remove_file(directory.join("recovered.json")).unwrap();
    std::fs::write(directory.join("runtime.sock"), b"another stale endpoint").unwrap();
    assert_eq!(
        recover(args(&directory, &registration, command))
            .await
            .unwrap(),
        result
    );
    assert_eq!(saved(&directory.join("recovered.json")), result);
    assert!(!directory.join("runtime.sock").exists());
    let mut conflict = args(&directory, &registration, command);
    conflict.expected_revision = Some(99);
    assert!(
        recover(conflict)
            .await
            .unwrap_err()
            .to_string()
            .contains("payload conflict")
    );
    assert_eq!(saved(&directory.join("recovered.json")), result);
}

#[tokio::test]
async fn recovery_rejects_wrong_identity_nil_commands_and_relinquished_ownership() {
    let (_root, directory, mut registration) = fixture().await;
    for field in 0..3 {
        let mut request = args(&directory, &registration, Uuid::new_v4());
        match field {
            0 => request.session = Uuid::new_v4(),
            1 => request.incarnation = Uuid::new_v4(),
            _ => request.command_id = Uuid::nil(),
        }
        assert!(
            recover(request)
                .await
                .unwrap_err()
                .to_string()
                .contains("identity mismatch")
        );
    }
    registration.state = voyage_protocol::process::ProcessState::Relinquished;
    recovery::persist(
        &directory.join("registration.json"),
        &serde_json::to_value(&registration).unwrap(),
    )
    .unwrap();
    assert!(
        recover(args(&directory, &registration, Uuid::new_v4()))
            .await
            .unwrap_err()
            .to_string()
            .contains("relinquished")
    );
    assert!(!directory.join("recoveries").exists());
}

#[tokio::test]
async fn live_startup_lock_fences_offline_recovery_before_any_receipt() {
    let (_root, directory, registration) = fixture().await;
    let lock =
        crate::attachment::journal::open_private_file(&directory.join("startup.lock")).unwrap();
    lock.try_lock().unwrap();
    assert!(
        recover(args(&directory, &registration, Uuid::new_v4()))
            .await
            .unwrap_err()
            .to_string()
            .contains("startup owned")
    );
    assert!(!directory.join("recoveries").exists());
}

#[tokio::test]
async fn replay_refuses_forged_receipt_identity_and_malformed_receipts() {
    let (_root, directory, registration) = fixture().await;
    let command = Uuid::new_v4();
    let result = recover(args(&directory, &registration, command))
        .await
        .unwrap();
    let marker = directory.join(format!("recoveries/{command}.json"));
    for field in ["session_id", "incarnation"] {
        let mut forged = result.clone();
        forged[field] = json!(Uuid::new_v4());
        recovery::persist(&marker, &forged).unwrap();
        assert!(
            recover(args(&directory, &registration, command))
                .await
                .unwrap_err()
                .to_string()
                .contains("receipt identity mismatch")
        );
    }
    std::fs::write(&marker, b"not json").unwrap();
    assert!(
        recover(args(&directory, &registration, command))
            .await
            .is_err()
    );
    assert_eq!(saved(&directory.join("recovered.json")), result);
}

#[tokio::test]
async fn tool_reconciliation_requires_revision_and_retains_attempt_receipt() {
    let (_root, directory, registration) = fixture().await;
    let command = Uuid::new_v4();
    let mut request = args(&directory, &registration, command);
    request.reconcile_tools = Some(Uuid::new_v4());
    assert!(
        recover(request)
            .await
            .unwrap_err()
            .to_string()
            .contains("requires expected revision")
    );
    let marker = saved(&directory.join(format!("recoveries/{command}.json")));
    assert_eq!(marker["command_id"], command.to_string());
    assert!(marker.get("restart_permitted").is_none());
    assert!(!directory.join("recovered.json").exists());
}

#[tokio::test]
async fn cleanup_of_stale_endpoint_errors_are_recoverable_from_durable_receipt() {
    let (_root, directory, registration) = fixture().await;
    let command = Uuid::new_v4();
    std::fs::create_dir(directory.join("runtime.sock")).unwrap();
    assert!(
        recover(args(&directory, &registration, command))
            .await
            .is_err()
    );
    let result = saved(&directory.join(format!("recoveries/{command}.json")));
    assert_eq!(result["restart_permitted"], true);
    assert!(
        recover(args(&directory, &registration, command))
            .await
            .is_err()
    );
    std::fs::remove_dir(directory.join("runtime.sock")).unwrap();
    assert_eq!(
        recover(args(&directory, &registration, command))
            .await
            .unwrap(),
        result
    );
}

#[tokio::test]
async fn legacy_recovery_requires_existing_installation_and_handles_idle_journal() {
    let (root, state) = tests::fixture().await;
    let directory = state.directory.clone();
    let session = state.registration.session_id;
    drop(state);
    let make = || legacy_recovery::LegacyRecoverArgs {
        directory: directory.clone(),
        session,
        acknowledge_cleanup: None,
        reconcile_tools: None,
        expected_revision: None,
    };
    assert!(
        legacy_recovery::recover(make())
            .await
            .unwrap_err()
            .to_string()
            .contains("existing legacy")
    );
    LocalActorStore::open(&directory).unwrap();
    let result = legacy_recovery::recover(make()).await.unwrap();
    assert_eq!(
        result,
        json!({"session_id":session,"run":null,"cleanup":"unchanged","reconciliation":null})
    );
    let mut request = make();
    request.reconcile_tools = Some(Uuid::new_v4());
    assert!(
        legacy_recovery::recover(request)
            .await
            .unwrap_err()
            .to_string()
            .contains("revision required")
    );
    let mut request = make();
    request.directory = root.path().join("missing");
    assert!(legacy_recovery::recover(request).await.is_err());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn guardian_evidence_is_identity_bound_and_a_live_guardian_is_not_cleanup() {
    let (_root, directory, registration) = fixture().await;
    assert!(!guardian::observed(&directory, &registration).unwrap());
    let marker = directory.join(format!("guardian-{}.json", registration.incarnation));
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    let mut evidence = json!({"session_id":registration.session_id,"incarnation":registration.incarnation,"boot_id":boot.trim(),"cleanup_observed":false});
    recovery::persist(&marker, &evidence).unwrap();
    assert!(!guardian::observed(&directory, &registration).unwrap());
    evidence["cleanup_observed"] = true.into();
    recovery::persist(&marker, &evidence).unwrap();
    let lock =
        crate::attachment::journal::open_private_file(&directory.join("guardian.lock")).unwrap();
    lock.try_lock().unwrap();
    assert!(!guardian::observed(&directory, &registration).unwrap());
    drop(lock);
    assert!(guardian::observed(&directory, &registration).unwrap());
    evidence["cleanup_observed"] = false.into();
    evidence["boot_id"] = json!(Uuid::nil());
    recovery::persist(&marker, &evidence).unwrap();
    assert!(guardian::observed(&directory, &registration).unwrap());
    evidence["session_id"] = json!(Uuid::new_v4());
    recovery::persist(&marker, &evidence).unwrap();
    assert!(
        guardian::observed(&directory, &registration)
            .unwrap_err()
            .to_string()
            .contains("identity mismatch")
    );
}

#[tokio::test]
async fn interrupted_run_is_retained_without_replay_or_can_be_explicitly_attested() {
    for attest in [false, true] {
        let (_root, directory, registration) = fixture().await;
        let actor = LocalActorStore::open(&directory.join("identity"))
            .unwrap()
            .identity()
            .unwrap();
        let owner = ManagedSessionOwner::open(directory.join("journal"), registration.session_id)
            .await
            .unwrap();
        let admission = crate::attachment::journal::TurnAdmission {
            budget: None,
            coordination: None,
            operator_name: None,
            command_id: Uuid::new_v4(),
            machine_id: actor.installation_id,
            principal_id: actor.principal_id,
            session_id: registration.session_id,
            expected_revision: owner.snapshot().await.unwrap().revision,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 60_000,
            prompt: "synthetic interrupted turn; never dispatch".into(),
            parts: vec![],
        };
        let crate::attachment::runtime::Admission::New(run) = owner.admit(admission).await.unwrap()
        else {
            panic!("expected new turn")
        };
        let run_id = run.record().await.unwrap().id;
        run.register_local_cleanup().await.unwrap();
        drop(run);
        drop(owner);
        let mut request = args(&directory, &registration, Uuid::new_v4());
        if attest {
            request.acknowledge_cleanup = Some(run_id);
        }
        let result = recover(request).await.unwrap();
        assert_eq!(result["restart_permitted"], true);
        assert_eq!(
            result["cleanup_disposition"],
            if attest {
                "operator_attested"
            } else {
                "unresolved_retained"
            }
        );
        let owner = ManagedSessionOwner::open(directory.join("journal"), registration.session_id)
            .await
            .unwrap();
        let snapshot = owner.process_snapshot().await.unwrap();
        let retained = snapshot["retained_cleanup"]["run_ids"].as_array().unwrap();
        if attest {
            assert!(retained.is_empty());
        } else {
            assert_eq!(retained, &vec![json!(run_id)]);
        }
        assert!(
            !owner
                .snapshot()
                .await
                .unwrap()
                .session
                .messages
                .iter()
                .any(|message| message.role == crate::model::Role::Assistant)
        );
    }
}
