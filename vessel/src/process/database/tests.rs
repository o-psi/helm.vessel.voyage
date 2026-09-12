use super::*;
use voyage_protocol::process::{PROCESS_PROTOCOL, ProcessState, VesselCommand};
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("vdb-{}", Uuid::new_v4()));
        registry::private_directory(&p).unwrap();
        registry::private_directory(&p.join("sessions")).unwrap();
        Self(p)
    }
    fn registration(&self) -> ProcessRegistration {
        ProcessRegistration {
            protocol: PROCESS_PROTOCOL,
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            restart_from: None,
            initialize: None,
            config_path: None,
            token: "private-test-token".into(),
            workspace: self.0.clone(),
            state: ProcessState::Starting,
            name: Some("Durable name".into()),
            executable: Some("/fixture/voyage".into()),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn bytes(r: &ProcessRegistration) -> Vec<u8> {
    serde_json::to_vec(&VesselCommand::Start {
        command_id: r.command_id,
        session_id: r.session_id,
        workspace: r.workspace.clone(),
    })
    .unwrap()
}

#[tokio::test]
async fn atomic_creation_and_exact_identity_survive_reopen() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    assert!(
        command(&f.0, "commands", r.command_id, bytes(&r), false)
            .await
            .unwrap()
    );
    assert!(
        command(&f.0, "commands", r.command_id, b"changed".to_vec(), true)
            .await
            .is_err()
    );
    assert!(admit(&f.0, &r, bytes(&r)).await.is_err());
    let restored = initialize(&f.0).await.unwrap();
    assert_eq!(restored[&r.session_id].incarnation, r.incarnation);
    let info = ProcessInfo::from(&r);
    settle_creation(&f.0, r.command_id, &info).await.unwrap();
    assert_eq!(
        creation_receipt(&f.0, r.command_id)
            .await
            .unwrap()
            .unwrap()
            .session_id,
        r.session_id
    );
    let db = open(&f.0).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM voyages", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}
#[tokio::test]
async fn non_admission_fence_blocks_delayed_create_without_allocating_voyage() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    command(
        &f.0,
        "start-resolution/intent",
        r.command_id,
        bytes(&r),
        true,
    )
    .await
    .unwrap();
    assert!(admit(&f.0, &r, bytes(&r)).await.is_err());
    assert!(catalogue(&f.0).await.unwrap().is_empty());
    assert!(
        !command(&f.0, "commands", r.command_id, bytes(&r), false)
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn legacy_import_is_one_time_and_sqlite_is_authoritative() {
    let f = Fixture::new();
    let mut r = f.registration();
    let dir = registry::directory(&f.0, r.session_id);
    registry::private_directory(&dir).unwrap();
    registry::publish(&dir, &r).unwrap();
    let commands = f.0.join("commands");
    registry::private_directory(&commands).unwrap();
    let path = commands.join(format!("{}.json", r.command_id));
    fs::write(&path, bytes(&r)).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    initialize(&f.0).await.unwrap();
    r.name = Some("Database name".into());
    save(&f.0, &r).await.unwrap();
    assert_eq!(initialize(&f.0).await.unwrap()[&r.session_id].name, r.name);
    assert!(
        command(&f.0, "commands", r.command_id, bytes(&r), false)
            .await
            .unwrap()
    );
    assert!(dir.join("registration.json").exists());
}
#[tokio::test]
async fn failed_refresh_preserves_details_and_backs_off() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    let summary = CatalogueSummary {
        session_id: r.session_id,
        revision: 3,
        observation_cursor: 8,
        name: Some("Retained".into()),
        model: "fixture".into(),
        created_at: None,
        last_turn_end: None,
        total_messages: 2,
        run_id: None,
        run_state: None,
        archived: false,
        deleted: false,
        pending_cleanup_run: None,
    };
    let db = open(&f.0).unwrap();
    db.execute(
        "UPDATE catalogue SET summary=?1,observed_at_ms=1",
        [serde_json::to_string(&summary).unwrap()],
    )
    .unwrap();
    drop(db);
    refresh(&f.0, &r).await.unwrap();
    refresh(&f.0, &r).await.unwrap();
    let info = catalogue(&f.0).await.unwrap().pop().unwrap();
    let metadata = info.catalogue.unwrap();
    assert!(metadata.stale);
    assert_eq!(metadata.summary.unwrap(), summary);
    assert_eq!(
        open(&f.0)
            .unwrap()
            .query_row("SELECT failures FROM catalogue", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[tokio::test]
async fn restart_does_not_claim_previous_live_observation() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    let mut info = ProcessInfo::from(&r);
    info.state = ProcessState::Live;
    observe_process(&f.0, &r, &info).await.unwrap();
    assert_eq!(catalogue(&f.0).await.unwrap()[0].state, ProcessState::Live);
    initialize(&f.0).await.unwrap();
    assert_ne!(catalogue(&f.0).await.unwrap()[0].state, ProcessState::Live);
}
#[tokio::test]
async fn rejects_unsafe_database_and_newer_schema() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let target = f.0.join("outside");
    fs::write(&target, b"preserved").unwrap();
    symlink(&target, f.0.join(FILE)).unwrap();
    assert!(initialize(&f.0).await.is_err());
    assert_eq!(fs::read(target).unwrap(), b"preserved");
    fs::remove_file(f.0.join(FILE)).unwrap();
    initialize(&f.0).await.unwrap();
    fs::set_permissions(f.0.join(FILE), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(initialize(&f.0).await.is_err());
    fs::set_permissions(f.0.join(FILE), fs::Permissions::from_mode(0o600)).unwrap();
    open(&f.0)
        .unwrap()
        .execute("UPDATE schema_version SET version=99", [])
        .unwrap();
    assert!(initialize(&f.0).await.is_err());
}

#[tokio::test]
async fn fresh_guards_observe_committed_incarnation_and_reject_stale_publication() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    let store = Registrations::new(f.0.clone());
    assert_eq!(
        store.lock().await.unwrap()[&r.session_id].incarnation,
        r.incarnation
    );
    let mut next = r.clone();
    next.incarnation = Uuid::new_v4();
    next.restart_from = Some(r.incarnation);
    save(&f.0, &next).await.unwrap();
    assert_eq!(
        store.lock().await.unwrap()[&r.session_id].incarnation,
        next.incarnation
    );
    assert!(save(&f.0, &r).await.is_err());
    let mut another = f.registration();
    another.incarnation = next.incarnation;
    assert!(admit(&f.0, &another, bytes(&another)).await.is_err());
    assert_eq!(catalogue(&f.0).await.unwrap().len(), 1);
}

#[tokio::test]
async fn malformed_legacy_record_rolls_back_whole_import() {
    let f = Fixture::new();
    let good = f.registration();
    let dir = registry::directory(&f.0, good.session_id);
    registry::private_directory(&dir).unwrap();
    registry::publish(&dir, &good).unwrap();
    let bad = f.registration();
    let bad_dir = registry::directory(&f.0, bad.session_id);
    registry::private_directory(&bad_dir).unwrap();
    registry::publish(&bad_dir, &bad).unwrap();
    fs::write(bad_dir.join("registration.json"), b"invalid json").unwrap();
    assert!(initialize(&f.0).await.is_err());
    let db = open(&f.0).unwrap();
    for table in ["voyages", "incarnations", "catalogue", "legacy_imports"] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "partial migration in {table}");
    }
    drop(db);
    assert!(dir.join("registration.json").exists());
    registry::publish(&bad_dir, &bad).unwrap();
    assert_eq!(initialize(&f.0).await.unwrap().len(), 2);
}
