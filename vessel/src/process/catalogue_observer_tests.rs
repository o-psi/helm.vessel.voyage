use super::*;
use serde_json::json;
use uuid::Uuid;

fn summary(session: Uuid) -> serde_json::Value {
    json!({"session_id":session,"revision":7,"observation_cursor":9,"name":"Scoped catalogue",
        "model":"fixture","created_at":null,"last_turn_end":null,"total_messages":3,
        "run_id":null,"run_state":null,"archived":false,"deleted":false,"pending_cleanup_run":null})
}

#[test]
fn helper_metadata_is_bounded_and_cannot_name_another_session() {
    let session = Uuid::new_v4();
    let value = summary(session);
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(decode(&bytes, session).unwrap().total_messages, 3);
    assert!(decode(&bytes, Uuid::new_v4()).is_err());
    assert!(decode(&vec![b' '; 8193], session).is_err());
    assert!(decode(b"not JSON", session).is_err());
    for (field, limit) in [
        ("name", 512),
        ("model", 512),
        ("created_at", 128),
        ("last_turn_end", 128),
        ("run_state", 128),
    ] {
        let mut bad = value.clone();
        bad[field] = json!("x".repeat(limit + 1));
        assert!(
            decode(&serde_json::to_vec(&bad).unwrap(), session).is_err(),
            "{field}"
        );
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires an explicitly designated disposable native-root fixture and Voyage executable"]
async fn native_bound_catalogue_uses_ordinary_helper_and_protected_layout() {
    use crate::process::{database, registry, runtime_storage};
    use std::{
        fs,
        num::NonZeroU64,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::PathBuf,
    };
    use voyage_protocol::{execution_identity::*, process::*};
    use voyage_storage::protected_linux::{RootDirectory, RuntimeRoot};
    assert_eq!(
        std::env::var("VOYAGE_DISPOSABLE_ROOT_FIXTURE").as_deref(),
        Ok("1")
    );
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let program = PathBuf::from(std::env::var_os("VOYAGE_TEST_CATALOGUE_EXECUTABLE").unwrap());
    super::super::launch::protected_binary(&program).unwrap();
    let uid: u32 = String::from_utf8(
        std::process::Command::new("/usr/bin/id")
            .args(["-u", "voyageordinary"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .parse()
    .unwrap();
    let gid: u32 = String::from_utf8(
        std::process::Command::new("/usr/bin/id")
            .args(["-g", "voyageordinary"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .parse()
    .unwrap();
    assert_ne!(uid, 0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let fixture = Fixture(
        std::env::current_dir()
            .unwrap()
            .join(format!("catalogue-native-{}", Uuid::new_v4())),
    );
    fs::create_dir(&fixture.0).unwrap();
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o755)).unwrap();
    let control = fixture.0.join("control");
    let runtime = fixture.0.join("runtime");
    registry::private_directory(&control).unwrap();
    registry::private_directory(&control.join("sessions")).unwrap();
    fs::create_dir(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o711)).unwrap();
    database::initialize(&control).await.unwrap();
    let identity = ConfiguredExecutionIdentity {
        identity: IdentityRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        label: "Native ordinary".into(),
        user_name: "voyageordinary".into(),
        uid,
        gid,
        supplementary_groups: vec![],
        home: "/home/voyageordinary".into(),
        account_context: AccountContextRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        authority: AuthorityClass::Ordinary,
        enabled: true,
    };
    database::store_identity(&control, &identity).await.unwrap();
    let session = Uuid::new_v4();
    let directory = runtime.join(session.to_string());
    RuntimeRoot::open(&runtime)
        .unwrap()
        .create_session(session.to_string().as_ref(), uid, gid)
        .unwrap();
    let layout = serde_json::to_vec(&json!({"version":1,"runtime_root":runtime})).unwrap();
    let protected = RootDirectory::open(&control).unwrap();
    protected
        .publish_new("runtime-layout.json".as_ref(), &layout, 4096)
        .unwrap();
    protected
        .publish_new("secret".as_ref(), b"control-fixture-secret", 4096)
        .unwrap();
    let helper = fixture.0.join("helper");
    let source = format!(
        r#"#!/usr/bin/python3
import json,os,pathlib,sys
assert os.getuid()=={uid} and os.geteuid()=={uid}
assert os.getgid()=={gid} and os.getgroups()==[]
assert os.environ['HOME']=='/home/voyageordinary'
assert 'VOYAGE_DISPOSABLE_ROOT_FIXTURE' not in os.environ
try: pathlib.Path({secret}).read_bytes(); raise AssertionError('control leaked')
except PermissionError: pass
status=pathlib.Path('/proc/self/status').read_text()
for field in ['CapEff','CapPrm','CapAmb']:
 assert next(line.split(':')[1].strip() for line in status.splitlines() if line.startswith(field+':'))=='0000000000000000'
assert next(line.split(':')[1].strip() for line in status.splitlines() if line.startswith('NoNewPrivs:'))=='1'
pathlib.Path({marker}).write_text(str(os.getuid()))
os.execv({program}, [{program}]+sys.argv[1:])
"#,
        secret = serde_json::to_string(&control.join("secret")).unwrap(),
        marker = serde_json::to_string(&directory.join("helper-identity")).unwrap(),
        program = serde_json::to_string(&program).unwrap()
    );
    fs::write(&helper, &source).unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    let registration = ProcessRegistration {
        protocol: PROCESS_PROTOCOL,
        session_id: session,
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        restart_from: None,
        initialize: None,
        config_path: None,
        token: "fixture-token".into(),
        peer_uids: Some(ProcessPeerUids {
            supervisor: 0,
            runtime: uid,
        }),
        workspace: identity.home.clone(),
        state: ProcessState::Starting,
        name: None,
        executable: Some(helper.clone()),
    };
    let binding = ExecutionBinding {
        session_id: session,
        incarnation: registration.incarnation,
        identity: identity.identity.clone(),
        account_context: identity.account_context.clone(),
        peer_uids: registration.peer_uids.clone().unwrap(),
        administrator_grant_id: None,
        host_identity_digest: "a".repeat(64),
        policy_digest: "b".repeat(64),
    };
    database::admit_with_binding(&control, &registration, vec![1], Some(&binding))
        .await
        .unwrap();
    // A minimal schema exercises the real bounded Voyage SQLite reader. Setup is
    // fixture-owned; parsing later must happen under the recorded ordinary UID.
    let journal = directory.join("journal");
    fs::create_dir(&journal).unwrap();
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o700)).unwrap();
    let file = journal.join("journal.sqlite3");
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute_batch("CREATE TABLE attachment_schema(id,version); INSERT INTO attachment_schema VALUES(1,2); CREATE TABLE sessions(id,revision,state); CREATE TABLE process_observations(session_id,cursor); CREATE TABLE runs(id,session_id,record); CREATE TABLE process_lifecycle(session_id,archived,deleted); CREATE TABLE local_cleanup_obligations(session_id,run_id,confirmation);").unwrap();
    db.execute("INSERT INTO sessions VALUES(?1,7,?2)", rusqlite::params![session.to_string(), json!({"name":"Scoped catalogue","model":"fixture","messages":[1,2,3],"run_summaries":[]}).to_string()]).unwrap();
    drop(db);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    for path in [&journal, &file] {
        std::os::unix::fs::chown(path, Some(uid), Some(gid)).unwrap();
    }
    assert_eq!(
        runtime_storage::directory(&control, &registration)
            .await
            .unwrap(),
        directory
    );
    assert_eq!(
        read(&control, &registration).await.unwrap().total_messages,
        3
    );
    assert_eq!(
        fs::metadata(directory.join("helper-identity"))
            .unwrap()
            .uid(),
        uid
    );
    database::refresh_now(&control, &registration)
        .await
        .unwrap();
    let metadata = database::catalogue(&control)
        .await
        .unwrap()
        .pop()
        .unwrap()
        .catalogue
        .unwrap();
    assert!(!metadata.stale);
    assert_eq!(
        metadata.summary.unwrap().name.as_deref(),
        Some("Scoped catalogue")
    );
    // Child output is never trusted as identity or bounded simply because the
    // child had the intended UID. Exercise rejection and bounded termination.
    fs::write(
        &helper,
        format!(
            "#!/usr/bin/python3\nprint({:?})\n",
            summary(Uuid::new_v4()).to_string()
        ),
    )
    .unwrap();
    assert!(read(&control, &registration).await.is_err());
    fs::write(&helper, "#!/usr/bin/python3\nprint('x'*8193)\n").unwrap();
    assert!(read(&control, &registration).await.is_err());
    fs::write(&helper, "#!/usr/bin/python3\nimport time\ntime.sleep(30)\n").unwrap();
    let started = std::time::Instant::now();
    assert!(read(&control, &registration).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(6));
    fs::write(&helper, &source).unwrap();
    let hardlink = fixture.0.join("helper-hardlink");
    fs::hard_link(&helper, &hardlink).unwrap();
    assert!(read(&control, &registration).await.is_err());
    fs::remove_file(hardlink).unwrap();
    let alias = fixture.0.join("helper-alias");
    std::os::unix::fs::symlink(&helper, &alias).unwrap();
    let mut aliased = registration.clone();
    aliased.executable = Some(alias);
    assert!(read(&control, &aliased).await.is_err());
    // A forged runtime projection cannot locate control or change the helper UID.
    fs::write(
        directory.join("registration.json"),
        b"forged runtime authority",
    )
    .unwrap();
    assert_eq!(
        read(&control, &registration).await.unwrap().total_messages,
        3
    );
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(read(&control, &registration).await.is_err());
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o711)).unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(read(&control, &registration).await.is_err());
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    // Root must not fall back to parsing even a valid current-UID legacy journal.
    fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(control.join("secret"), &file).unwrap();
    assert!(read(&control, &registration).await.is_err());
    database::refresh_now(&control, &registration)
        .await
        .unwrap();
    let stale = database::catalogue(&control)
        .await
        .unwrap()
        .pop()
        .unwrap()
        .catalogue
        .unwrap();
    assert!(stale.stale);
    assert_eq!(stale.summary.unwrap().total_messages, 3);
    let mut changed = identity;
    changed.identity.revision = NonZeroU64::new(2).unwrap();
    database::store_identity(&control, &changed).await.unwrap();
    fs::remove_file(directory.join("helper-identity")).unwrap();
    assert!(read(&control, &registration).await.is_err());
    assert!(!directory.join("helper-identity").exists());
}
