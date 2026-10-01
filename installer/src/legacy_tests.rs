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
db.executescript('CREATE TABLE schema_version(id INTEGER,version INTEGER); INSERT INTO schema_version VALUES(1,1); CREATE TABLE voyages(session_id TEXT,registration TEXT); CREATE TABLE incarnations(id TEXT,session_id TEXT); CREATE TABLE lifecycle_commands(id TEXT,request TEXT); CREATE TABLE creation_receipts(id TEXT,record TEXT); CREATE TABLE legacy_imports(source TEXT,digest BLOB);')
registration={'session_id':session,'incarnation':incarnation,'workspace':str(state),'token':'synthetic-private-token','config_path':None,'initialize':None,'restart_from':None}
db.execute('INSERT INTO voyages VALUES(?,?)',(session,json.dumps(registration)));db.commit();db.close()
root=state/'sessions'/session; (root/'journal').mkdir(parents=True)
(root/'registration.json').write_text(json.dumps(registration))
for name in ['stopped.json','guardian-'+incarnation+'.json']:
 (root/name).write_text(json.dumps({'session_id':session,'incarnation':incarnation,'cleanup_observed':True}))
journal=sqlite3.connect(root/'journal/journal.sqlite3'); journal.executescript('CREATE TABLE attachment_schema(id INTEGER,version INTEGER); INSERT INTO attachment_schema VALUES(1,12); CREATE TABLE runs(active INTEGER); CREATE TABLE sessions(id TEXT);');journal.execute('INSERT INTO sessions VALUES(?)',(session,));journal.commit();journal.close()
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
        None,
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
    let original = files::hash(&state.join("catalogue.sqlite3")).unwrap();
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
        original
    );
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
                    "INSERT INTO lifecycle_commands VALUES('new-effect','uncertain'); UPDATE schema_version SET version=2;"
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
