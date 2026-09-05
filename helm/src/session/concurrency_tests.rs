use super::*;

#[tokio::test]
async fn cas_deletion_branch_and_legacy_paths() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().into());
    let mut original = Session::new(dir.path().into(), "test".into());
    assert_eq!(original.revision, 0);
    store.save(&mut original).await.unwrap();
    assert_eq!(original.revision, 1);
    let mut stale = store.load(original.id).await.unwrap();
    store.save(&mut original).await.unwrap();
    let before = serde_json::to_vec(&stale).unwrap();
    assert!(store.save(&mut stale).await.is_err());
    assert_eq!(before, serde_json::to_vec(&stale).unwrap());
    assert_eq!(store.load(original.id).await.unwrap().revision, 2);
    let branch = store.branch(&original, None).await.unwrap();
    assert_eq!(branch.revision, 1);
    store.delete(original.id).await.unwrap();
    assert!(store.save(&mut original).await.is_err());
    assert!(dir.path().join(format!(".{}.lock", original.id)).exists());

    let legacy = Session::new(dir.path().into(), "old".into());
    let mut json = serde_json::to_value(&legacy).unwrap();
    json.as_object_mut().unwrap().remove("revision");
    std::fs::write(store.path(legacy.id), serde_json::to_vec(&json).unwrap()).unwrap();
    let mut loaded = store
        .load_reference(store.path(legacy.id).to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(loaded.revision, 0);
    store.save(&mut loaded).await.unwrap();
    assert_eq!(loaded.revision, 1);
    let other = tempfile::tempdir().unwrap();
    assert!(
        SessionStore::new(other.path().into())
            .save(&mut loaded)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn independent_handles_wait_and_cancel_without_leaking_lock() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().into());
    let other = SessionStore::new(dir.path().into());
    let mut session = Session::new(dir.path().into(), "test".into());
    let lease = store.acquire_execution(session.id).await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            other.acquire_execution(session.id)
        )
        .await
        .is_err()
    );
    store.save_with_lease(&mut session, &lease).await.unwrap();
    assert!(
        store
            .save_with_lease(&mut Session::new(dir.path().into(), "x".into()), &lease)
            .await
            .is_err()
    );
    let elsewhere = tempfile::tempdir().unwrap();
    assert!(
        SessionStore::new(elsewhere.path().into())
            .save_with_lease(&mut session, &lease)
            .await
            .is_err()
    );
    drop(lease);
    let lease = tokio::time::timeout(Duration::from_secs(2), other.acquire_execution(session.id))
        .await
        .unwrap()
        .unwrap();
    other.delete_with_lease(session.id, &lease).await.unwrap();
}

#[tokio::test]
async fn failed_save_and_invalid_files_leave_snapshot_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().into());
    let mut session = Session::new(dir.path().into(), "test".into());
    store.save(&mut session).await.unwrap();
    let path = store.path(session.id);
    let disk = std::fs::read(&path).unwrap();
    session.revision = u64::MAX;
    let before = serde_json::to_vec(&session).unwrap();
    assert!(store.save(&mut session).await.is_err());
    assert_eq!(before, serde_json::to_vec(&session).unwrap());
    assert_eq!(disk, std::fs::read(&path).unwrap());
    std::fs::write(&path, b"corrupt").unwrap();
    assert!(store.save(&mut session).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt");
    let wrong = Session::new(dir.path().into(), "wrong".into());
    std::fs::write(&path, serde_json::to_vec(&wrong).unwrap()).unwrap();
    assert!(store.load(session.id).await.is_err());
    std::fs::File::create(&path)
        .unwrap()
        .set_len(MAX_SESSION_BYTES + 1)
        .unwrap();
    assert!(store.load(session.id).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_symlinks_including_sidecars_and_parent_paths() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().into());
    let mut session = Session::new(dir.path().into(), "test".into());
    let target = dir.path().join("target");
    std::fs::write(&target, b"untouched").unwrap();
    symlink(&target, store.path(session.id)).unwrap();
    assert!(store.save(&mut session).await.is_err());
    assert!(store.load(session.id).await.is_err());
    assert!(store.delete(session.id).await.is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
    let id = Uuid::new_v4();
    symlink(&target, dir.path().join(format!(".{id}.lock"))).unwrap();
    assert!(store.acquire_execution(id).await.is_err());
    let alias = dir.path().join("alias");
    symlink(dir.path(), &alias).unwrap();
    assert!(
        SessionStore::new(alias)
            .acquire_execution(id)
            .await
            .is_err()
    );
}

// The child is the same test executable, but a fresh process/open-file description.
#[test]
fn subprocess_lock_probe() {
    let Ok(directory) = std::env::var("HELM_SESSION_LOCK_PROBE") else {
        return;
    };
    let id = Uuid::parse_str(&std::env::var("HELM_SESSION_LOCK_ID").unwrap()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let store = SessionStore::new(directory.into());
        assert!(
            tokio::time::timeout(Duration::from_millis(150), store.acquire_execution(id))
                .await
                .is_err()
        );
    });
}

#[tokio::test]
async fn os_lock_excludes_independent_subprocess() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().into());
    let id = Uuid::new_v4();
    let lease = store.acquire_execution(id).await.unwrap();
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session::concurrency_tests::subprocess_lock_probe",
            "--nocapture",
        ])
        .env("HELM_SESSION_LOCK_PROBE", dir.path())
        .env("HELM_SESSION_LOCK_ID", id.to_string())
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    drop(lease);
    store.acquire_execution(id).await.unwrap();
}

#[tokio::test]
async fn serialization_limit_failure_preserves_disk_and_caller() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().into());
    let mut session = Session::new(dir.path().into(), "test".into());
    store.save(&mut session).await.unwrap();
    let disk = std::fs::read(store.path(session.id)).unwrap();
    session.model = "x".repeat(MAX_SESSION_BYTES as usize);
    let timestamp = session.updated_at;
    let revision = session.revision;
    session.name = None;
    assert!(store.save(&mut session).await.is_err());
    assert_eq!(session.updated_at, timestamp);
    assert_eq!(session.revision, revision);
    assert!(session.name.is_none());
    assert_eq!(std::fs::read(store.path(session.id)).unwrap(), disk);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn foreign_path_resume_is_rejected_before_execution_without_importing() {
    let source_dir = tempfile::tempdir().unwrap();
    let source = SessionStore::new(source_dir.path().into());
    let mut session = Session::new(source_dir.path().into(), "test".into());
    source.save(&mut session).await.unwrap();
    let path = source.path(session.id);
    let before = std::fs::read(&path).unwrap();
    let target_dir = tempfile::tempdir().unwrap();
    // A missing destination directory must also reject the foreign path without
    // implicitly creating a store or copying a session.
    for target in [
        target_dir.path().to_path_buf(),
        target_dir.path().join("missing"),
    ] {
        let target = SessionStore::new(target.clone());
        let error = target
            .load_reference(path.to_str().unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("another store"), "{error:#}");
    }
    assert_eq!(std::fs::read_dir(target_dir.path()).unwrap().count(), 0);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut resumed = source.load_reference(path.to_str().unwrap()).await.unwrap();
    source.save(&mut resumed).await.unwrap();
    assert_eq!(resumed.revision, session.revision + 1);
}

#[tokio::test]
async fn save_replace_and_delete_work_on_supported_platforms() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let mut session = Session::new(dir.path().into(), "first".into());
    store.save(&mut session).await.unwrap();
    session.model = "replacement".into();
    store.save(&mut session).await.unwrap();
    let restored = store.load(session.id).await.unwrap();
    assert_eq!(restored.model, "replacement");
    assert_eq!(restored.revision, 2);
    store.delete(session.id).await.unwrap();
    assert!(store.load(session.id).await.is_err());
    assert!(
        store
            .directory
            .join(format!(".{}.lock", session.id))
            .is_file()
    );
}

#[tokio::test]
async fn completion_run_references_survive_resume_but_do_not_transfer_to_branches() {
    use crate::completion::runtime::RunReference;
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().into());
    let mut original = Session::new(dir.path().into(), "test".into());
    original.completion_runs.push(RunReference {
        session_id: original.id,
        run_id: Uuid::new_v4(),
    });
    store.save(&mut original).await.unwrap();
    let loaded = store.load(original.id).await.unwrap();
    assert_eq!(loaded.completion_runs, original.completion_runs);
    assert!(
        store
            .branch(&loaded, None)
            .await
            .unwrap()
            .completion_runs
            .is_empty()
    );
    let valid = serde_json::to_value(&loaded).unwrap();
    for invalid in [
        serde_json::json!([{"session_id": Uuid::new_v4(), "run_id": Uuid::new_v4()}]),
        serde_json::json!([{"session_id": loaded.id, "run_id": Uuid::nil()}]),
        serde_json::json!([loaded.completion_runs[0], loaded.completion_runs[0]]),
    ] {
        let mut value = valid.clone();
        value["completion_runs"] = invalid;
        std::fs::write(store.path(loaded.id), serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(store.load(loaded.id).await.is_err());
    }
    let mut legacy = valid;
    legacy.as_object_mut().unwrap().remove("completion_runs");
    std::fs::write(store.path(loaded.id), serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(
        store
            .load(loaded.id)
            .await
            .unwrap()
            .completion_runs
            .is_empty()
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn root_owned_macos_temp_alias_supports_session_lifecycle_without_allowing_user_links() {
    use std::os::unix::fs::symlink;
    for temp_root in ["/var/tmp", "/tmp"] {
        let dir = tempfile::tempdir_in(temp_root).unwrap();
        let store = SessionStore::new(dir.path().join("sessions"));
        let mut session = Session::new(dir.path().into(), "test".into());
        store.save(&mut session).await.unwrap();
        store.save(&mut session).await.unwrap();
        assert_eq!(store.load(session.id).await.unwrap().revision, 2);
        let alias = dir.path().join("alias");
        symlink(dir.path().join("sessions"), &alias).unwrap();
        assert!(SessionStore::new(alias).load(session.id).await.is_err());
        store.delete(session.id).await.unwrap();
        assert!(store.load(session.id).await.is_err());
    }
}

#[cfg(windows)]
#[tokio::test]
async fn canonical_windows_store_path_supports_leases_and_session_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().canonicalize().unwrap();
    assert!(matches!(
        canonical.components().next(),
        Some(std::path::Component::Prefix(prefix)) if prefix.kind().is_verbatim()
    ));
    // The root remains checked, but its incomplete drive prefix must never be
    // queried as an ancestor. Missing descendants are valid for initial stores.
    reject_symlinks(&canonical).unwrap();
    let store = SessionStore::new(canonical.join("sessions"));
    let mut session = Session::new(canonical, "test".into());
    let lease = store.acquire_execution(session.id).await.unwrap();
    store.save_with_lease(&mut session, &lease).await.unwrap();
    store.save_with_lease(&mut session, &lease).await.unwrap();
    assert_eq!(store.load(session.id).await.unwrap().revision, 2);
    store.delete_with_lease(session.id, &lease).await.unwrap();
    assert!(store.load(session.id).await.is_err());
}
