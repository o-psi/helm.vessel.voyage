use super::*;

fn relinquish(revision: u64) -> RuntimeCommand {
    RuntimeCommand::Relinquish {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: 61000,
        transfer_id: Uuid::new_v4(),
        destination_vessel_id: Uuid::new_v4(),
        prepare_digest: "a".repeat(64),
    }
}
fn initialization(receipt: &Value) -> RuntimeInitialization {
    RuntimeInitialization::Transfer {
        transfer_id: serde_json::from_value(receipt["transfer_id"].clone()).unwrap(),
        artifact_path: "artifact.json".into(),
        sha256: receipt["artifact_sha256"].as_str().unwrap().into(),
        prepare_digest: "a".repeat(64),
        generation: receipt["generation"].as_u64().unwrap(),
    }
}

#[test]
fn transfer_round_trip_is_irreversible_idempotent_and_rehomes_workspace() {
    let (root, mut j, s, g) = fixture();
    j.retain_initial_configuration(&g, "host-private-settings".into())
        .unwrap();
    let command = relinquish(0);
    let (receipt, artifact) = j.relinquish(&g, &command, 1000).unwrap();
    assert_eq!(receipt["status"], "relinquished");
    assert_eq!(receipt["artifact_bytes"], artifact.len());
    assert!(!String::from_utf8_lossy(&artifact).contains("host-private-settings"));
    assert_eq!(j.lifecycle_status(s.id).unwrap()["archived"], true);
    assert_eq!(
        j.relinquish(&g, &command, 999999).unwrap(),
        (receipt.clone(), artifact.clone())
    );
    assert!(j.relinquish(&g, &relinquish(1), 1000).is_err());
    let mut destination = Journal::open(root.path().join("destination")).unwrap();
    let workspace = root.path().join("new-workspace");
    fs::create_dir(&workspace).unwrap();
    let init = initialization(&receipt);
    destination
        .import_transfer(&init, &artifact, s.id, &workspace)
        .unwrap();
    destination
        .import_transfer(&init, &artifact, s.id, &workspace)
        .unwrap();
    let restored = destination.load_session(s.id).unwrap();
    assert_eq!(restored.session.workspace, workspace);
    assert_eq!(restored.session.model, s.model);
    assert!(destination.initial_configuration(s.id).unwrap().is_none());
}

#[test]
fn transfer_import_rejects_digest_binding_and_existing_identity() {
    let (root, mut j, s, g) = fixture();
    let (receipt, artifact) = j.relinquish(&g, &relinquish(0), 1000).unwrap();
    let init = initialization(&receipt);
    let mut destination = Journal::open(root.path().join("destination")).unwrap();
    assert!(
        destination
            .import_transfer(&init, b"corrupt", s.id, root.path())
            .is_err()
    );
    assert!(
        destination
            .import_transfer(&init, &artifact, Uuid::new_v4(), root.path())
            .is_err()
    );
    let mut wrong = init.clone();
    if let RuntimeInitialization::Transfer { generation, .. } = &mut wrong {
        *generation += 1;
    }
    assert!(
        destination
            .import_transfer(&wrong, &artifact, s.id, root.path())
            .is_err()
    );
    assert!(destination.load_session(s.id).is_err());
    destination.create_session(&s).unwrap();
    assert!(
        destination
            .import_transfer(&init, &artifact, s.id, root.path())
            .is_err()
    );
    assert_eq!(revision(&destination, s.id), 0);
}

#[test]
fn relinquishment_requires_valid_binding_revision_idle_and_cleanup() {
    let (_root, mut j, s, g) = fixture();
    for command in [RuntimeCommand::Health, relinquish(1)] {
        assert!(j.relinquish(&g, &command, 1000).is_err());
    }
    assert!(j.relinquish(&g, &relinquish(0), 61000).is_err());
    let mut invalid = relinquish(0);
    if let RuntimeCommand::Relinquish { prepare_digest, .. } = &mut invalid {
        prepare_digest.clear();
    }
    assert!(j.relinquish(&g, &invalid, 1000).is_err());
    let run = admit(&mut j, &g);
    j.register_local_cleanup(&g, run.id).unwrap();
    assert!(
        j.relinquish(&g, &relinquish(revision(&j, s.id)), 1000)
            .is_err()
    );
    j.finish(
        &g,
        run.id,
        RunState::Cancelled,
        Some("test cancellation"),
        None,
    )
    .unwrap();
    assert!(
        j.relinquish(&g, &relinquish(revision(&j, s.id)), 1000)
            .is_err()
    );
    j.confirm_local_cleanup_observed(&g, run.id).unwrap();
    j.relinquish(&g, &relinquish(revision(&j, s.id)), 1000)
        .unwrap();
}

#[test]
fn managed_import_copies_only_selected_session_and_retires_source() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().canonicalize().unwrap();
    let source_path = root.path().join("source");
    let mut source = Journal::open(source_path.clone()).unwrap();
    let session = Session::new(workspace.clone(), "legacy".into());
    let other = Session::new(workspace.clone(), "unrelated".into());
    source.create_session(&session).unwrap();
    source.create_session(&other).unwrap();
    let guard = source.acquire_execution(session.id).unwrap();
    let run = admit(&mut source, &guard);
    source
        .finish(
            &guard,
            run.id,
            RunState::Cancelled,
            Some("test cancellation"),
            None,
        )
        .unwrap();
    let rev = revision(&source, session.id);
    drop(guard);
    let transfer = Uuid::new_v4();
    let mut destination = Journal::open(root.path().join("destination")).unwrap();
    destination
        .import_managed(&source_path, session.id, transfer, rev, &workspace)
        .unwrap();
    assert_eq!(
        destination.load_session(session.id).unwrap().session.model,
        "legacy"
    );
    assert_eq!(destination.run(run.id).unwrap().state, RunState::Cancelled);
    assert!(destination.load_session(other.id).is_err());
    assert!(source.load_session(session.id).is_err());
    assert_eq!(
        source.list_session_summaries(None, 10).unwrap().sessions[0].id,
        other.id
    );
    destination
        .import_managed(&source_path, session.id, transfer, rev, &workspace)
        .unwrap();
    assert!(
        destination
            .import_managed(&source_path, session.id, transfer, rev + 1, &workspace)
            .is_err()
    );
    assert!(
        destination
            .import_managed(&source_path, session.id, Uuid::new_v4(), rev, &workspace)
            .is_err()
    );
    destination.remove_managed_originals(session.id).unwrap();
    destination.remove_managed_originals(session.id).unwrap();
}

#[test]
fn managed_import_refuses_owned_coincident_and_supervised_sources() {
    let (root, mut source, s, guard) = fixture();
    let mut destination = Journal::open(root.path().join("destination")).unwrap();
    let source_path = root.path().join("journal");
    let transfer = Uuid::new_v4();
    assert!(
        destination
            .import_managed(&source_path, s.id, transfer, 0, &s.workspace)
            .is_err()
    );
    assert!(
        source
            .import_managed(&source_path, s.id, transfer, 0, &s.workspace)
            .is_err()
    );
    source
        .process_metadata(&guard, actor(), &rename(0, "supervised"), 1000)
        .unwrap();
    drop(guard);
    assert!(
        destination
            .import_managed(&source_path, s.id, transfer, 1, &s.workspace)
            .is_err()
    );
    assert!(destination.load_session(s.id).is_err());
    assert_eq!(revision(&source, s.id), 1);
}

#[test]
fn transferred_command_receipts_are_tombstones_not_executable_work() {
    let (root, mut source, s, guard) = fixture();
    let run = admit(&mut source, &guard);
    source
        .finish(
            &guard,
            run.id,
            RunState::Cancelled,
            Some("test cancellation"),
            None,
        )
        .unwrap();
    let (receipt, bytes) = source
        .relinquish(&guard, &relinquish(revision(&source, s.id)), 1000)
        .unwrap();
    let mut destination = Journal::open(root.path().join("destination")).unwrap();
    destination
        .import_transfer(&initialization(&receipt), &bytes, s.id, &s.workspace)
        .unwrap();
    let receipt = destination
        .process_receipt(run.command_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt["status"], "transferred");
    assert_eq!(receipt["effects_replayed"], false);
    assert!(destination.process_latest_run(s.id).unwrap().is_none());
}

#[test]
fn transfer_import_validates_snapshot_identity_even_with_matching_digest() {
    let (root, mut source, s, guard) = fixture();
    let (receipt, bytes) = source.relinquish(&guard, &relinquish(0), 1000).unwrap();
    let mut portable: Value = serde_json::from_slice(&bytes).unwrap();
    portable["session"]["id"] = json!(Uuid::new_v4());
    let bytes = serde_json::to_vec(&portable).unwrap();
    let mut init = initialization(&receipt);
    if let RuntimeInitialization::Transfer { sha256, .. } = &mut init {
        *sha256 = hex::encode(Sha256::digest(&bytes));
    }
    let mut destination = Journal::open(root.path().join("destination")).unwrap();
    assert!(
        destination
            .import_transfer(&init, &bytes, s.id, &s.workspace)
            .is_err()
    );
    assert!(destination.load_session(s.id).is_err());
}

#[test]
fn managed_import_rejects_stale_workspace_and_active_run_without_retiring_source() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().canonicalize().unwrap();
    let source_path = root.path().join("source");
    let mut source = Journal::open(source_path.clone()).unwrap();
    let s = Session::new(workspace.clone(), "legacy".into());
    source.create_session(&s).unwrap();
    let mut destination = Journal::open(root.path().join("destination")).unwrap();
    assert!(
        destination
            .import_managed(&source_path, s.id, Uuid::new_v4(), 1, &workspace)
            .is_err()
    );
    assert!(
        destination
            .import_managed(
                &source_path,
                s.id,
                Uuid::new_v4(),
                0,
                &root.path().join("wrong")
            )
            .is_err()
    );
    let guard = source.acquire_execution(s.id).unwrap();
    let run = admit(&mut source, &guard);
    drop(guard);
    assert!(
        destination
            .import_managed(
                &source_path,
                s.id,
                Uuid::new_v4(),
                revision(&source, s.id),
                &workspace
            )
            .is_err()
    );
    assert_eq!(source.run(run.id).unwrap().state, RunState::Accepted);
    assert!(destination.load_session(s.id).is_err());
    assert_eq!(source.load_session(s.id).unwrap().session.id, s.id);
}
