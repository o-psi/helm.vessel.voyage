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
        Some(&serde_json::to_vec(&serde_json::json!({"catalogue":include_str!("fixtures/legacy-v1.0.2-catalogue.sql"),"journal":include_str!("fixtures/legacy-v1.0.2-journal.sql")})).unwrap()),
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
    let restored = inspect("inspect", &state, &accounts, &stage, None).unwrap();
    assert_eq!(restored["canonical_sha256"], original["canonical_sha256"]);
    assert_eq!(restored["session_count"], original["session_count"]);
    assert_eq!(guard.proof.evidence["session_count"], 1);
    assert!(guard.proof.evidence.get("sessions").is_none());
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
