use super::*;
use std::num::NonZeroU64;
use voyage_protocol::execution_identity::{
    AccountContextRef, AdministratorGrant, AuthorityClass, ConfiguredExecutionIdentity,
    EXECUTION_SCHEMA, ExecutionBinding, IdentityRef,
};
use voyage_protocol::process::ProcessPeerUids;
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
            peer_uids: None,
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
async fn v1_catalogue_migrates_without_reinterpreting_legacy_voyages() {
    let fixture = Fixture::new();
    initialize(&fixture.0).await.unwrap();
    let registration = fixture.registration();
    admit(&fixture.0, &registration, bytes(&registration))
        .await
        .unwrap();
    let database = open(&fixture.0).unwrap();
    database
        .execute_batch(
            "DROP TABLE execution_bindings; DROP TABLE execution_identities; \
             DROP TABLE administrator_revocations; DROP TABLE administrator_grants; \
             UPDATE schema_version SET version=1 WHERE id=1;",
        )
        .unwrap();
    drop(database);
    let restored = initialize(&fixture.0).await.unwrap();
    assert_eq!(
        restored[&registration.session_id].incarnation,
        registration.incarnation
    );
    assert!(
        execution_binding(&fixture.0, registration.session_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        open(&fixture.0)
            .unwrap()
            .query_row("SELECT version FROM schema_version WHERE id=1", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn identity_binding_is_atomic_with_admission_and_requires_exact_identity() {
    let fixture = Fixture::new();
    initialize(&fixture.0).await.unwrap();
    let supervisor_uid = unsafe { libc::geteuid() };
    let uid = if supervisor_uid == 0 {
        1000
    } else {
        supervisor_uid
    };
    let identity = ConfiguredExecutionIdentity {
        identity: IdentityRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        label: "Ordinary fixture".into(),
        user_name: "fixture".into(),
        uid,
        gid: unsafe { libc::getegid() },
        supplementary_groups: vec![],
        home: fixture.0.clone(),
        account_context: AccountContextRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        authority: AuthorityClass::Ordinary,
        enabled: true,
    };
    store_identity(&fixture.0, &identity).await.unwrap();
    let mut registration = fixture.registration();
    let peers = ProcessPeerUids {
        supervisor: supervisor_uid,
        runtime: uid,
    };
    registration.peer_uids = Some(peers.clone());
    let binding = ExecutionBinding {
        session_id: registration.session_id,
        incarnation: registration.incarnation,
        identity: identity.identity.clone(),
        account_context: identity.account_context.clone(),
        peer_uids: peers,
        administrator_grant_id: None,
        host_identity_digest: "a".repeat(64),
        policy_digest: "b".repeat(64),
    };
    admit_with_binding(
        &fixture.0,
        &registration,
        bytes(&registration),
        Some(&binding),
    )
    .await
    .unwrap();
    assert_eq!(
        execution_binding(&fixture.0, registration.session_id)
            .await
            .unwrap(),
        Some(binding.clone())
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        bound_observer_identity(&fixture.0, &registration)
            .await
            .unwrap(),
        identity
    );
    let mut changed_peer = registration.clone();
    changed_peer.peer_uids.as_mut().unwrap().runtime = uid.wrapping_add(1);
    #[cfg(target_os = "linux")]
    assert!(
        bound_observer_identity(&fixture.0, &changed_peer)
            .await
            .is_err()
    );
    assert!(save(&fixture.0, &changed_peer).await.is_err());
    let mut next = registration.clone();
    next.restart_from = Some(registration.incarnation);
    next.incarnation = Uuid::new_v4();
    #[cfg(target_os = "linux")]
    assert!(bound_observer_identity(&fixture.0, &next).await.is_err());
    assert!(save(&fixture.0, &next).await.is_err());
    let mut altered = identity.clone();
    altered.uid = uid.wrapping_add(1);
    assert!(store_identity(&fixture.0, &altered).await.is_err());
    let mut unbound = fixture.registration();
    unbound.peer_uids = registration.peer_uids.clone();
    assert!(save(&fixture.0, &unbound).await.is_err());
    assert!(admit(&fixture.0, &unbound, bytes(&unbound)).await.is_err());
    let mut superseded = identity;
    superseded.identity.revision = NonZeroU64::new(2).unwrap();
    store_identity(&fixture.0, &superseded).await.unwrap();
    #[cfg(target_os = "linux")]
    assert!(
        bound_observer_identity(&fixture.0, &registration)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn administrator_binding_requires_current_protected_voyage_grant() {
    let fixture = Fixture::new();
    initialize(&fixture.0).await.unwrap();
    let identity = ConfiguredExecutionIdentity {
        identity: IdentityRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        label: "Administrator fixture".into(),
        user_name: "root".into(),
        uid: 0,
        gid: 0,
        supplementary_groups: vec![],
        home: fixture.0.clone(),
        account_context: AccountContextRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        authority: AuthorityClass::Administrator,
        enabled: true,
    };
    store_identity(&fixture.0, &identity).await.unwrap();
    let mut registration = fixture.registration();
    registration.peer_uids = Some(ProcessPeerUids {
        supervisor: unsafe { libc::geteuid() },
        runtime: 0,
    });
    let grant = AdministratorGrant {
        schema: EXECUTION_SCHEMA,
        grant_id: Uuid::new_v4(),
        vessel_id: Uuid::new_v4(),
        session_id: registration.session_id,
        administrative_owner_id: Uuid::new_v4(),
        authority_revision: NonZeroU64::new(1).unwrap(),
        identity: identity.identity.clone(),
        account_context: identity.account_context.clone(),
        host_identity_digest: "a".repeat(64),
        policy_digest: "b".repeat(64),
        created_at_ms: 1,
        revoked_at_ms: None,
    };
    let binding = ExecutionBinding {
        session_id: registration.session_id,
        incarnation: registration.incarnation,
        identity: identity.identity.clone(),
        account_context: identity.account_context.clone(),
        peer_uids: registration.peer_uids.clone().unwrap(),
        administrator_grant_id: Some(grant.grant_id),
        host_identity_digest: grant.host_identity_digest.clone(),
        policy_digest: grant.policy_digest.clone(),
    };
    assert!(
        admit_with_binding(
            &fixture.0,
            &registration,
            bytes(&registration),
            Some(&binding)
        )
        .await
        .is_err()
    );
    issue_administrator_grant(&fixture.0, &grant).await.unwrap();
    let mut forged = binding.clone();
    forged.policy_digest = "c".repeat(64);
    assert!(
        admit_with_binding(
            &fixture.0,
            &registration,
            bytes(&registration),
            Some(&forged)
        )
        .await
        .is_err()
    );
    let revocation = Uuid::new_v4();
    revoke_administrator_grant(&fixture.0, grant.grant_id, revocation, 2)
        .await
        .unwrap();
    revoke_administrator_grant(&fixture.0, grant.grant_id, revocation, 2)
        .await
        .unwrap();
    assert!(
        revoke_administrator_grant(&fixture.0, grant.grant_id, Uuid::new_v4(), 2)
            .await
            .is_err()
    );
    assert_eq!(
        administrator_grant(&fixture.0, grant.grant_id)
            .await
            .unwrap()
            .revoked_at_ms,
        Some(2)
    );
    assert!(
        admit_with_binding(
            &fixture.0,
            &registration,
            bytes(&registration),
            Some(&binding)
        )
        .await
        .is_err()
    );
    let next = AdministratorGrant {
        grant_id: Uuid::new_v4(),
        ..grant
    };
    issue_administrator_grant(&fixture.0, &next).await.unwrap();
    let binding = ExecutionBinding {
        administrator_grant_id: Some(next.grant_id),
        ..binding
    };
    admit_with_binding(
        &fixture.0,
        &registration,
        bytes(&registration),
        Some(&binding),
    )
    .await
    .unwrap();
    #[cfg(target_os = "linux")]
    assert_eq!(
        bound_observer_identity(&fixture.0, &registration)
            .await
            .unwrap(),
        identity
    );
    revoke_administrator_grant(&fixture.0, next.grant_id, Uuid::new_v4(), 3)
        .await
        .unwrap();
    #[cfg(target_os = "linux")]
    assert!(
        bound_observer_identity(&fixture.0, &registration)
            .await
            .is_err()
    );
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

#[tokio::test]
async fn process_invalidation_is_bounded_to_transitions_and_current_incarnation() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    let count = || {
        open(&f.0)
            .unwrap()
            .query_row("SELECT count(*) FROM catalogue_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    let mut info = ProcessInfo::from(&r);
    observe_process(&f.0, &r, &info).await.unwrap();
    let initial = count();
    observe_process(&f.0, &r, &info).await.unwrap();
    assert_eq!(count(), initial);
    info.state = ProcessState::Live;
    observe_process(&f.0, &r, &info).await.unwrap();
    assert_eq!(count(), initial + 1);
    let mut stale = r.clone();
    stale.incarnation = Uuid::new_v4();
    observe_process(&f.0, &stale, &info).await.unwrap();
    assert_eq!(count(), initial + 1);
}

#[tokio::test]
async fn restart_invalidates_once_and_ignores_stale_process_publication() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let first = f.registration();
    admit(&f.0, &first, bytes(&first)).await.unwrap();
    let mut next = first.clone();
    next.restart_from = Some(first.incarnation);
    next.incarnation = Uuid::new_v4();
    next.command_id = Uuid::new_v4();
    save(&f.0, &next).await.unwrap();
    let db = open(&f.0).unwrap();
    let changes: i64 = db
        .query_row(
            "SELECT count(*) FROM catalogue_events WHERE kind='owner_changed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(changes, 1);
    drop(db);
    save(&f.0, &next).await.unwrap();
    let mut old_info = ProcessInfo::from(&first);
    old_info.state = ProcessState::Live;
    observe_process(&f.0, &first, &old_info).await.unwrap();
    let db = open(&f.0).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM catalogue_events WHERE kind='owner_changed'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        catalogue(&f.0).await.unwrap()[0].incarnation,
        next.incarnation
    );
}
