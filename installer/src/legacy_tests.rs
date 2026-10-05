//! Owned SQLite/lock fixtures; no service manager, provider, or live migration.
use super::*;
use crate::fixture_tests::Fixture;
fn setup(f: &Fixture) -> (PathBuf, PathBuf, PathBuf) {
    let state = f.root.join("state");
    let accounts = f.root.join("accounts");
    let stage = f.root.join("stage");
    for path in [&state, &accounts, &stage] {
        files::private_directory(path).unwrap();
    }
    let script = r#"
import os,pathlib,sqlite3,json,sys
os.umask(0o077)
state=pathlib.Path(sys.argv[1]); accounts=pathlib.Path(sys.argv[2])
session='11111111-1111-4111-8111-111111111111'; incarnation='22222222-2222-4222-8222-222222222222'
db=sqlite3.connect(state/'catalogue.sqlite3')
schemas=json.loads(sys.stdin.buffer.read(1024*1024))
db.executescript(schemas['catalogue'])
command='33333333-3333-4333-8333-333333333333'
registration={'protocol':1,'session_id':session,'incarnation':incarnation,'command_id':command,'executable':'/fixture/voyage','workspace':str(state),'state':'suspended','token':'synthetic-private-token','config_path':None,'initialize':None,'restart_from':None}
db.execute('INSERT INTO voyages VALUES(?,?,?,?,?,?,?)',(session,incarnation,str(state).encode(),'suspended',None,json.dumps(registration),0))
db.execute('INSERT INTO incarnations VALUES(?,?,?,?,?)',(incarnation,session,command,json.dumps(registration),0))
db.execute('INSERT INTO catalogue(session_id,incarnation) VALUES(?,?)',(session,incarnation))
db.execute('INSERT INTO legacy_imports VALUES(?,?)',('complete-v1',bytes(32)))
db.commit();db.close()
root=state/'sessions'/session; (root/'journal').mkdir(parents=True)
(root/'registration.json').write_text(json.dumps(registration))
(root/'stopped.json').write_text(json.dumps({'session_id':session,'incarnation':incarnation,'cleanup_observed':True,'suspended':True,'archive':None,'deletion':None}))
(root/('guardian-'+incarnation+'.json')).write_text(json.dumps({'session_id':session,'incarnation':incarnation,'boot_id':'44444444-4444-4444-8444-444444444444','cleanup_observed':True}))
journal=sqlite3.connect(root/'journal/journal.sqlite3');journal.executescript(schemas['journal'])
saved={'id':session,'revision':0,'created_at':'2026-09-30T00:00:00Z','updated_at':'2026-09-30T00:00:00Z','workspace':str(state),'model':'synthetic-model','messages':[],'usage':{'input_tokens':0,'output_tokens':0}}
journal.execute('INSERT INTO sessions VALUES(?,?,?,?)',(session,0,json.dumps(saved),1))
for slot in range(256):journal.execute('INSERT INTO notification_outbox VALUES(?,?,?)',(slot,'00000000000000000000',' '*1024))
journal.commit();journal.close()
(accounts/'accounts.json').write_text('{}')
namespace={'__name__':'fixture_source'};exec(schemas['notification_helper'],namespace)
notifications=state/'notifications';notifications.mkdir(mode=0o700)
notice=sqlite3.connect(notifications/'notifications.sqlite3');notice.execute('PRAGMA journal_mode=PERSIST');notice.executescript(namespace['NOTIFICATION_SCHEMA']);notice.close()

"#;
    crate::service::command::run(
        Path::new("/usr/bin/python3"),
        &[
            "-I",
            "-c",
            script,
            state.to_str().unwrap(),
            accounts.to_str().unwrap(),
        ],
        Some(&serde_json::to_vec(&serde_json::json!({"catalogue":include_str!("fixtures/legacy-v1.0.2-catalogue.sql"),"journal":include_str!("fixtures/legacy-v1.0.2-journal.sql"),"notification_helper":include_str!("legacy_update.py")})).unwrap()),
    )
    .unwrap();
    (state, accounts, stage)
}
fn sql(state: &Path, sql: &str) {
    crate::service::command::run(Path::new("/usr/bin/python3"),&["-I","-c","import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.executescript(sys.argv[2]); c.close()",state.join("catalogue.sqlite3").to_str().unwrap(),sql],None).unwrap();
}
#[test]
fn exact_quiescent_snapshot_restores_original_file_without_editing_live_schema() {
    let f = Fixture::new();
    let (state, accounts, stage) = setup(&f);
    let original = inspect("inspect", &state, &accounts, &stage, None).unwrap();
    let mut guard = begin(&state, &accounts, &stage).unwrap();
    guard.permit_supervisor();
    sql(
        &state,
        "CREATE TABLE execution_identities(id TEXT); UPDATE schema_version SET version=2;",
    );
    guard.verify().unwrap();
    guard.restore().unwrap();
    assert_eq!(
        files::hash(&state.join("catalogue.sqlite3")).unwrap(),
        guard.proof.evidence["backup_sha256"].as_str().unwrap()
    );
    assert_eq!(guard.proof.evidence["session_count"], 1);
    assert!(guard.proof.evidence.get("sessions").is_none());
    // An independent observer cannot borrow the restore helper's guardian lease.
    // Production also releases these locks before observing restarted old services.
    assert!(inspect("inspect", &state, &accounts, &stage, None).is_err());
    drop(guard);
    let restored = inspect("inspect", &state, &accounts, &stage, None).unwrap();
    assert_eq!(restored["canonical_sha256"], original["canonical_sha256"]);
    assert_eq!(restored["session_count"], original["session_count"]);
}
#[test]
fn changed_canonical_admission_authority_or_private_account_prevents_restore_and_preserves_current_file()
 {
    for variant in 0..3 {
        let f = Fixture::new();
        let (state, accounts, stage) = setup(&f);
        let mut guard = begin(&state, &accounts, &stage).unwrap();
        guard.permit_supervisor();
        sql(
            &state,
            match variant {
                0 => {
                    "INSERT INTO lifecycle_commands VALUES('commands','55555555-5555-4555-8555-555555555555',CAST('uncertain' AS BLOB)); UPDATE schema_version SET version=2;"
                }
                1 => {
                    "CREATE TABLE execution_identities(id TEXT); INSERT INTO execution_identities VALUES('new-authority'); UPDATE schema_version SET version=2;"
                }
                _ => "UPDATE schema_version SET version=2;",
            },
        );
        if variant == 2 {
            std::fs::write(accounts.join("accounts.json"), b"changed-private-state").unwrap();
        }
        let current = files::hash(&state.join("catalogue.sqlite3")).unwrap();
        assert!(guard.restore().is_err());
        assert_eq!(
            files::hash(&state.join("catalogue.sqlite3")).unwrap(),
            current
        );
        assert!(stage.join("legacy-catalogue.sqlite3").exists());
    }
}
#[test]
fn held_legacy_execution_owner_is_refused_without_cancellation_or_snapshot() {
    let f = Fixture::new();
    let (state, accounts, stage) = setup(&f);
    let path=state.join("sessions/11111111-1111-4111-8111-111111111111/journal/11111111-1111-4111-8111-111111111111.execution.lock");
    let _live = files::lock(&path).unwrap();
    assert!(begin(&state, &accounts, &stage).is_err());
    assert!(!stage.join("legacy-catalogue.sqlite3").exists());
}
#[test]
fn unobserved_cleanup_and_new_journal_schema_refuse_legacy_eligibility() {
    for variant in 0..2 {
        let f = Fixture::new();
        let (state, accounts, stage) = setup(&f);
        let directory = state.join("sessions/11111111-1111-4111-8111-111111111111");
        if variant == 0 {
            std::fs::write(
                directory.join("stopped.json"),
                b"{\"cleanup_observed\":false}",
            )
            .unwrap();
        } else {
            crate::service::command::run(Path::new("/usr/bin/python3"),&["-I","-c","import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('UPDATE attachment_schema SET version=20'); c.commit()",directory.join("journal/journal.sqlite3").to_str().unwrap()],None).unwrap();
        }
        assert!(eligible(&state, &accounts, &stage).is_err());
    }
}

#[test]
fn coordinated_staged_backup_and_proof_changes_cannot_replace_the_pinned_snapshot() {
    let f = Fixture::new();
    let (state, accounts, stage) = setup(&f);
    let mut guard = begin(&state, &accounts, &stage).unwrap();
    guard.permit_supervisor();
    let original = files::hash(&state.join("catalogue.sqlite3")).unwrap();
    let backup = stage.join("legacy-catalogue.sqlite3");
    let mut bytes = std::fs::read(&backup).unwrap();
    bytes.extend_from_slice(b"coordinated-staged-change");
    std::fs::write(&backup, bytes).unwrap();
    let path = stage.join("legacy-proof.json");
    let mut changed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    changed["backup_sha256"] = files::hash(&backup).unwrap().into();
    std::fs::write(path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(guard.verify().is_err());
    assert!(guard.restore().is_err());
    assert_eq!(
        files::hash(&state.join("catalogue.sqlite3")).unwrap(),
        original
    );
}

#[test]
fn any_registration_projection_authority_extra_is_refused_without_catalogue_restore() {
    for unknown in [false, true] {
        let f = Fixture::new();
        let (state, accounts, stage) = setup(&f);
        let mut guard = begin(&state, &accounts, &stage).unwrap();
        guard.permit_supervisor();
        let path = state.join("sessions/11111111-1111-4111-8111-111111111111/registration.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        if unknown {
            value["unreviewed_authority"] = true.into();
        } else {
            value["peer_uids"] = serde_json::json!({"supervisor":1,"runtime":0});
        }
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        let original = files::hash(&state.join("catalogue.sqlite3")).unwrap();
        assert!(guard.verify().is_err());
        assert!(guard.restore().is_err());
        assert_eq!(
            files::hash(&state.join("catalogue.sqlite3")).unwrap(),
            original
        );
    }
}

#[test]
fn previous_pointer_observation_requires_pinned_restored_marker_and_original_namespace() {
    let f = Fixture::new();
    let (state, accounts, stage) = setup(&f);
    let mut guard = begin(&state, &accounts, &stage).unwrap();
    assert!(restored(&guard.proof).is_err()); // Eligible schema1 alone is insufficient.
    guard.permit_supervisor();
    sql(
        &state,
        "CREATE TABLE execution_identities(id TEXT); UPDATE schema_version SET version=2;",
    );
    guard.restore().unwrap();
    let proof = guard.proof.clone();
    drop(guard);
    restored(&proof).unwrap();
    let previous = files::hash(&state.join("catalogue.sqlite3")).unwrap();
    let marker = stage.join("legacy-restored.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
    value["backup_sha256"] = "f".repeat(64).into();
    std::fs::write(marker, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(restored(&proof).is_err());
    assert_eq!(
        files::hash(&state.join("catalogue.sqlite3")).unwrap(),
        previous
    );
}

#[test]
fn positively_empty_failed_startup_is_eligible_but_unknown_empty_journal_is_refused() {
    for proved in [false, true] {
        let f = Fixture::new();
        let (state, accounts, stage) = setup(&f);
        let directory = state.join("sessions/11111111-1111-4111-8111-111111111111");
        crate::service::command::run(Path::new("/usr/bin/python3"),&["-I","-c","import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('DELETE FROM sessions'); c.commit()",directory.join("journal/journal.sqlite3").to_str().unwrap()],None).unwrap();
        if proved {
            let path = directory.join("stopped.json");
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            value["startup_failed"] = true.into();
            std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        assert_eq!(eligible(&state, &accounts, &stage).is_ok(), proved);
    }
}

#[test]
fn forward_recovery_proves_existing_schema2_but_never_restores_or_downgrades_it() {
    let f = Fixture::new();
    let (state, accounts, stage) = setup(&f);
    assert!(forward_evidence(&state, &accounts, &stage).is_err());
    sql(
        &state,
        include_str!("../../vessel/src/process/database_v2_migration.sql"),
    );
    let before = files::hash(&state.join("catalogue.sqlite3")).unwrap();
    let mut guard = begin_forward(&state, &accounts, &stage).unwrap();
    guard.verify().unwrap();
    let mut leases = guard._sessions.iter().collect::<Vec<_>>();
    leases.push(guard.supervisor.as_ref().unwrap());
    assert!(
        inspect_leased(
            "restore",
            &state,
            &accounts,
            &stage,
            Some(&guard.proof.evidence),
            &leases
        )
        .is_err()
    );
    assert!(guard.restore().is_err());
    assert_eq!(
        files::hash(&state.join("catalogue.sqlite3")).unwrap(),
        before
    );
    let proof = guard.proof.clone();
    drop(guard);
    hold_forward(&proof).unwrap().verify().unwrap();
    std::fs::write(accounts.join("accounts.json"), b"changed account").unwrap();
    assert!(hold_forward(&proof).is_err());
    assert_eq!(
        files::hash(&state.join("catalogue.sqlite3")).unwrap(),
        before
    );
}
#[test]
fn forward_recovery_refuses_new_authority_and_unobserved_owner_cleanup() {
    for variant in 0..2 {
        let f = Fixture::new();
        let (state, accounts, stage) = setup(&f);
        sql(
            &state,
            include_str!("../../vessel/src/process/database_v2_migration.sql"),
        );
        if variant == 0 {
            sql(
                &state,
                "INSERT INTO execution_identities VALUES('new-owner',1,'{}');",
            );
        } else {
            std::fs::write(
                state.join("sessions/11111111-1111-4111-8111-111111111111/stopped.json"),
                b"{\"cleanup_observed\":false}",
            )
            .unwrap();
        }
        assert!(begin_forward(&state, &accounts, &stage).is_err());
        assert!(!stage.join("legacy-catalogue.sqlite3").exists());
    }
}

fn notification_sql(state: &Path, statement: &str) {
    crate::service::command::run(Path::new("/usr/bin/python3"),&["-I","-c","import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('PRAGMA journal_mode=PERSIST'); c.executescript(sys.argv[2]); c.close()",state.join("notifications/notifications.sqlite3").to_str().unwrap(),statement],None).unwrap();
}
#[test]
fn live_review_normalizes_real_idle_sqlite_physical_drift_but_held_proof_stays_raw() {
    let f = Fixture::new();
    let (state, accounts, stage) = setup(&f);
    sql(
        &state,
        include_str!("../../vessel/src/process/database_v2_migration.sql"),
    );
    let before = forward_evidence(&state, &accounts, &stage).unwrap();
    // Exactly the courier's repeated idempotent header transaction; no row changes.
    notification_sql(&state, "BEGIN IMMEDIATE; PRAGMA user_version=1; COMMIT;");
    let after = forward_evidence(&state, &accounts, &stage).unwrap();
    assert_ne!(before["state_sha256"], after["state_sha256"]);
    assert_eq!(before["review_state_sha256"], after["review_state_sha256"]);
    assert_eq!(before["canonical_sha256"], after["canonical_sha256"]);
    assert_eq!(before["accounts_sha256"], after["accounts_sha256"]);
    let guard = begin_forward(&state, &accounts, &stage).unwrap();
    guard.verify().unwrap();
    notification_sql(&state, "BEGIN IMMEDIATE; PRAGMA user_version=1; COMMIT;");
    assert!(guard.verify().is_err());
}
#[test]
fn notification_review_pins_clock_complete_rows_and_every_other_private_file() {
    use std::os::unix::fs::PermissionsExt;
    for variant in 0..3 {
        let f = Fixture::new();
        let (state, accounts, stage) = setup(&f);
        sql(
            &state,
            include_str!("../../vessel/src/process/database_v2_migration.sql"),
        );
        let before = forward_evidence(&state, &accounts, &stage).unwrap();
        match variant {
            0 => notification_sql(&state, "UPDATE clock SET now=1 WHERE id=1;"),
            1 => notification_sql(
                &state,
                "INSERT INTO destinations VALUES('destination','grant','source','{}',100,NULL,NULL,0,NULL);",
            ),
            _ => {
                let path = state.join("private-other.json");
                std::fs::write(&path, b"new private effect").unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        let after = forward_evidence(&state, &accounts, &stage).unwrap();
        assert_ne!(before["review_state_sha256"], after["review_state_sha256"]);
    }
}
#[test]
fn notification_review_refuses_changed_schema_hot_missing_or_nonprivate_pairs() {
    use std::os::unix::fs::PermissionsExt;
    for variant in 0..15 {
        let f = Fixture::new();
        let (state, accounts, stage) = setup(&f);
        sql(
            &state,
            include_str!("../../vessel/src/process/database_v2_migration.sql"),
        );
        let directory = state.join("notifications");
        let db = directory.join("notifications.sqlite3");
        let journal = directory.join("notifications.sqlite3-journal");
        match variant {
            0 => notification_sql(&state, "PRAGMA user_version=2;"),
            1 => notification_sql(&state, "CREATE TABLE unreviewed_authority(value TEXT);"),
            2 => notification_sql(
                &state,
                "CREATE TRIGGER unreviewed AFTER INSERT ON destinations BEGIN DELETE FROM clock; END;",
            ),
            3 => notification_sql(
                &state,
                "ALTER TABLE destinations ADD COLUMN hidden_authority TEXT;",
            ),
            4 => std::fs::remove_file(journal).unwrap(),
            5 => std::fs::remove_file(db).unwrap(),
            6 => std::fs::write(journal, b"hot journal header must refuse").unwrap(),
            7 => std::fs::set_permissions(db, std::fs::Permissions::from_mode(0o644)).unwrap(),
            8 => {
                std::fs::hard_link(&db, directory.join("other-link")).unwrap();
            }
            9 => {
                let moved = directory.join("original.sqlite3");
                std::fs::rename(&db, &moved).unwrap();
                std::os::unix::fs::symlink(moved, db).unwrap();
            }
            10 => {
                std::fs::write(directory.join("notifications.sqlite3-wal"), b"").unwrap();
            }
            11 => {
                std::fs::write(directory.join("notifications.sqlite3-shm"), b"").unwrap();
            }
            12 => notification_sql(&state, "PRAGMA application_id=1;"),
            13 => notification_sql(&state, "UPDATE clock SET now='unsupported';"),
            _ => {
                let moved = state.join("retained-notifications");
                std::fs::rename(&directory, &moved).unwrap();
                std::os::unix::fs::symlink(moved, directory).unwrap();
            }
        }
        assert!(
            forward_evidence(&state, &accounts, &stage).is_err(),
            "variant {variant}"
        );
    }
}

#[test]
fn forward_evidence_pins_actual_catalogue_journal_and_registration_protocol_formats() {
    let f = Fixture::new();
    let (state, accounts, stage) = setup(&f);
    sql(
        &state,
        include_str!("../../vessel/src/process/database_v2_migration.sql"),
    );
    let evidence = forward_evidence(&state, &accounts, &stage).unwrap();
    assert_eq!(
        evidence["observed_formats"],
        serde_json::json!({"catalogue_schema":2,
        "journal_schemas":[12],"process_protocols":[1]})
    );
    let guard = begin_forward(&state, &accounts, &stage).unwrap();
    assert_eq!(
        guard.proof.evidence["observed_formats"],
        evidence["observed_formats"]
    );
    guard.verify().unwrap();
    let journal =
        state.join("sessions/11111111-1111-4111-8111-111111111111/journal/journal.sqlite3");
    crate::service::command::run(Path::new("/usr/bin/python3"), &["-I","-c",
        "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('UPDATE attachment_schema SET version=21 WHERE id=1');c.commit()",
        journal.to_str().unwrap()],None).unwrap();
    assert!(guard.verify().is_err());
    assert!(forward_evidence(&state, &accounts, &stage).is_err());
}

#[test]
fn restored_legacy_pointer_uses_exact_typed_held_proof_without_opening_an_owner_lease_gap() {
    let f = Fixture::new();
    let (state, accounts, stage) = setup(&f);
    let old = crate::fixture_tests::release(&f, "guard-old", "1.0.2");
    let original = crate::install::run(crate::install::Options {
        bin_dir: old,
        replace_existing: false,
        dry_run: false,
    })
    .unwrap()
    .release;
    let candidate = crate::fixture_tests::release(&f, "guard-candidate", "1.0.3");
    let mut held = begin(&state, &accounts, &stage).unwrap();
    let next = crate::install::run(crate::install::Options {
        bin_dir: candidate,
        replace_existing: false,
        dry_run: false,
    })
    .unwrap()
    .release;
    held.permit_supervisor();
    sql(
        &state,
        include_str!("../../vessel/src/process/database_v2_migration.sql"),
    );
    held.restore().unwrap();
    held.verify_restored_held().unwrap();
    // The ordinary observer deliberately cannot borrow the held guardian lock.
    // It must keep refusing, rather than dropping locks to select an old reader.
    assert!(crate::install::rollback(false).is_err());
    let before = std::fs::read(f.root.join("install/transaction.json")).unwrap();
    assert!(crate::install::rollback_restored_legacy(&held, &"f".repeat(64), &original).is_err());
    assert!(crate::install::rollback_restored_legacy(&held, &next, &"e".repeat(64)).is_err());
    assert_eq!(
        std::fs::read(f.root.join("install/transaction.json")).unwrap(),
        before
    );
    let changed = crate::install::rollback_restored_legacy(&held, &next, &original).unwrap();
    assert_eq!(changed.release, original);
    assert_eq!(
        std::fs::read_link(f.root.join("install/current")).unwrap(),
        f.root.join("install/releases").join(&original)
    );
    held.verify_restored_held().unwrap();
    assert!(eligible(&state, &accounts, &stage).is_err());
    assert!(
        crate::install::rollback_restored_legacy(&held, &next, &original).is_err(),
        "duplicate pointer selection cannot replay"
    );
    drop(held);
    eligible(&state, &accounts, &stage).unwrap();
    f.done();
}
#[test]
fn held_restored_pointer_refuses_snapshot_mutation_foreign_namespace_and_forward_proof() {
    for variant in 0..3 {
        let f = Fixture::new();
        let (state, accounts, stage) = setup(&f);
        let old = crate::fixture_tests::release(&f, "refuse-old", "1.0.2");
        let previous = crate::install::run(crate::install::Options {
            bin_dir: old,
            replace_existing: false,
            dry_run: false,
        })
        .unwrap()
        .release;
        let candidate = crate::fixture_tests::release(&f, "refuse-candidate", "1.0.3");
        let current = crate::install::run(crate::install::Options {
            bin_dir: candidate,
            replace_existing: false,
            dry_run: false,
        })
        .unwrap()
        .release;
        let mut held = if variant == 2 {
            sql(
                &state,
                include_str!("../../vessel/src/process/database_v2_migration.sql"),
            );
            begin_forward(&state, &accounts, &stage).unwrap()
        } else {
            let mut h = begin(&state, &accounts, &stage).unwrap();
            h.permit_supervisor();
            sql(
                &state,
                include_str!("../../vessel/src/process/database_v2_migration.sql"),
            );
            h.restore().unwrap();
            h
        };
        if variant == 0 {
            let p = stage.join("legacy-catalogue.sqlite3");
            let mut b = std::fs::read(&p).unwrap();
            b.push(0);
            std::fs::write(p, b).unwrap();
        }
        if variant == 1 {
            held.proof.state = f.root.join("foreign-state");
        }
        let before = std::fs::read(f.root.join("install/transaction.json")).unwrap();
        assert!(crate::install::rollback_restored_legacy(&held, &current, &previous).is_err());
        assert_eq!(
            std::fs::read(f.root.join("install/transaction.json")).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read_link(f.root.join("install/current")).unwrap(),
            f.root.join("install/releases").join(&current)
        );
        f.done();
    }
}

#[test]
fn adapted_legacy_rollback_refuses_migrated_state_without_changing_the_pointer() {
    let f = Fixture::new();
    let (state, _, _) = setup(&f);
    let old = crate::fixture_tests::release(&f, "adapted-old", "1.0.2");
    let mut manifest = crate::install::release::Manifest::inspect(&old).unwrap();
    manifest.version = "v1.0.2-debian12-isolated-glibc".into();
    std::fs::write(
        old.parent().unwrap().join("release.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    crate::install::run(crate::install::Options {
        bin_dir: old,
        replace_existing: false,
        dry_run: false,
    })
    .unwrap();
    let candidate = crate::fixture_tests::release(&f, "adapted-next", "1.0.3");
    let next = crate::install::run(crate::install::Options {
        bin_dir: candidate,
        replace_existing: false,
        dry_run: false,
    })
    .unwrap()
    .release;
    sql(
        &state,
        include_str!("../../vessel/src/process/database_v2_migration.sql"),
    );
    for dry_run in [true, false] {
        assert!(
            crate::install::rollback(dry_run)
                .err()
                .unwrap()
                .to_string()
                .contains("migrated or active state")
        );
        assert_eq!(
            std::fs::read_link(f.root.join("install/current")).unwrap(),
            f.root.join("install/releases").join(&next)
        );
    }
    f.done();
}

#[test]
fn pre_guardian_journal9_requires_original_observed_receipts_and_never_fabricates_a_witness() {
    for variant in 0..21 {
        let f = Fixture::new();
        let (state, accounts, stage) = setup(&f);
        let session = state.join("sessions/11111111-1111-4111-8111-111111111111");
        let marker = session.join("guardian-22222222-2222-4222-8222-222222222222.json");
        std::fs::remove_file(&marker).unwrap();
        let script = r#"
import json,pathlib,sqlite3,sys
root=pathlib.Path(sys.argv[1]); variant=int(sys.argv[2]); session=root.name
run='55555555-5555-4555-8555-555555555555'
db=sqlite3.connect(root/'journal/journal.sqlite3')
db.executescript('UPDATE attachment_schema SET version=9; CREATE TABLE remote_cleanup_attestations(run_id TEXT PRIMARY KEY,installation_id TEXT NOT NULL,principal_id TEXT NOT NULL); CREATE TABLE process_assignment_local_cleanup(run_id TEXT PRIMARY KEY); CREATE TABLE process_cleanup_progress(session_id TEXT PRIMARY KEY,run_id TEXT NOT NULL,record TEXT NOT NULL); CREATE TABLE process_session_resources(id TEXT PRIMARY KEY,session_id TEXT NOT NULL,run_id TEXT NOT NULL,kind TEXT NOT NULL,state TEXT);')
db.execute('INSERT INTO runs VALUES(?,?,?,?)',(run,session,'{}',0))
db.execute('INSERT INTO local_cleanup_obligations VALUES(?,?,?,?,?)',(run,session,'local','actor','observed'))
db.execute('INSERT INTO process_session_resources VALUES(?,?,?,?,?)',(run,session,run,'root_terminals','observed'))
record={'run_id':run,'phase':'observed','pending':[],'reason':None,'retryable':False}
db.execute('INSERT INTO process_cleanup_progress VALUES(?,?,?)',(session,run,json.dumps(record)))
stopped_path=root/'stopped.json';stopped=json.loads(stopped_path.read_text())
if variant==1:db.execute("UPDATE process_session_resources SET state='pending'")
elif variant==2:stopped['cleanup_observed']=False
elif variant==3:db.execute('UPDATE attachment_schema SET version=12')
elif variant==4:db.execute("UPDATE local_cleanup_obligations SET confirmation='operator_attested'")
elif variant==5:record['phase']='blocked'
elif variant==6:record['pending']=['unknown resource']
elif variant==7:db.execute('INSERT INTO remote_cleanup_attestations VALUES(?,?,?)',(run,'local','actor'))
elif variant==8:db.execute('INSERT INTO process_assignment_local_cleanup VALUES(?)',(run,))
elif variant==9:stopped['unknown']=True
elif variant==10:stopped['incarnation']=run
elif variant==11:db.execute('UPDATE process_session_resources SET state=NULL')
elif variant==12:db.execute('DROP TABLE process_session_resources')
elif variant==13:db.execute('UPDATE process_session_resources SET session_id=?',(run,))
elif variant==14:db.execute('UPDATE local_cleanup_obligations SET confirmation=NULL')
elif variant==15:record['retryable']=True
elif variant==16:record['run_id']=session
elif variant==17:
 marker=root/'guardian-22222222-2222-4222-8222-222222222222.json'
 marker.symlink_to(root/'missing')
elif variant==19:db.execute('UPDATE runs SET active=1')
elif variant==20:db.execute('DELETE FROM runs')
elif variant==18:
 (root/'guardian-22222222-2222-4222-8222-222222222222.json').write_text(json.dumps({'session_id':session,'incarnation':stopped['incarnation'],'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'cleanup_observed':False}))
db.execute('UPDATE process_cleanup_progress SET record=?',(json.dumps(record),));db.commit();db.close()
stopped_path.write_text(json.dumps(stopped))
"#;
        crate::service::command::run(
            Path::new("/usr/bin/python3"),
            &[
                "-I",
                "-c",
                script,
                session.to_str().unwrap(),
                &variant.to_string(),
            ],
            None,
        )
        .unwrap();
        if variant == 0 {
            let original = files::read(&session.join("stopped.json"), 65536).unwrap();
            eligible(&state, &accounts, &stage).unwrap();
            let held = begin(&state, &accounts, &stage).unwrap();
            held.verify().unwrap();
            assert!(!marker.exists());
            assert_eq!(
                files::read(&session.join("stopped.json"), 65536).unwrap(),
                original
            );
            drop(held);
            sql(
                &state,
                include_str!("../../vessel/src/process/database_v2_migration.sql"),
            );
            assert!(forward_evidence(&state, &accounts, &stage).is_err());
        } else {
            assert!(
                eligible(&state, &accounts, &stage).is_err(),
                "variant {variant}"
            );
        }
        f.done();
    }
}
