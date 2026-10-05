//! Whole ordinary forward transactions with real private SQLite, publication and
//! helper leases. Manager/PID observations use the existing thread-local seam;
//! this is not a claim of native systemd behavior. No host service is reachable.
use super::*;
use crate::fixture_tests::Fixture;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::{fs::PermissionsExt, fs::symlink},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const ORIGINAL: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const OPERATION: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const SESSION: &str = "11111111-1111-4111-8111-111111111111";
const INCARNATION: &str = "22222222-2222-4222-8222-222222222222";
const SUPERVISOR: &str = "voyage-vessel.service";
const GATEWAY: &str = "fixture-gateway.service";
const ORIGIN: &str = "https://owned.invalid";
const OLD_PID: u32 = 701;
const OLD_GATEWAY_PID: u32 = 702;
const NEW_PID: u32 = 703;
const NEW_GATEWAY_PID: u32 = 704;

struct Health {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    reject_target_once: Arc<AtomicBool>,
    child: Option<thread::JoinHandle<()>>,
}
impl Health {
    fn new(root: PathBuf, target: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let ended = stop.clone();
        let reject_target_once = Arc::new(AtomicBool::new(false));
        let rejected = reject_target_once.clone();
        let child = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(45);
            while !ended.load(Ordering::Acquire) {
                assert!(
                    Instant::now() < deadline,
                    "owned health fixture exceeded lifetime"
                );
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut request = [0; 4096];
                        let _ = stream.read(&mut request);
                        if ended.load(Ordering::Acquire) {
                            break;
                        }
                        let current = fs::read_link(root.join("install/current")).ok();
                        let version = if current.as_deref()
                            == Some(root.join("install/releases").join(&target).as_path())
                        {
                            if rejected.swap(false, Ordering::AcqRel) {
                                "0.0.1"
                            } else {
                                "1.0.4"
                            }
                        } else {
                            "1.0.3"
                        };
                        let body = serde_json::json!({"status":"ok","version":version}).to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream.write_all(response.as_bytes()).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("owned health listener: {error}"),
                }
            }
        });
        Self {
            address,
            stop,
            reject_target_once,
            child: Some(child),
        }
    }
}
impl Drop for Health {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(1));
        self.child.take().unwrap().join().unwrap();
    }
}
struct Owned {
    // Retire the loopback observer before Fixture removes its private directory.
    health: Health,
    f: Fixture,
    old: String,
    running: String,
    target: String,
    incoming: PathBuf,
    original_bytes: Vec<u8>,
}
fn quote(value: &Path) -> String {
    format!("'{}'", value.to_str().unwrap().replace('\'', "'\\''"))
}
fn json_file(path: &Path, value: &serde_json::Value) {
    files::atomic_json(path, value).unwrap();
}
fn execute_sql(path: &Path, statement: &str) {
    crate::service::command::run(Path::new("/usr/bin/python3"), &["-I", "-c", "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.executescript(sys.argv[2]);c.close()",path.to_str().unwrap(),statement],None).unwrap();
}
fn notification_sql(path: &Path, statement: &str) {
    crate::service::command::run(Path::new("/usr/bin/python3"), &["-I", "-c", "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA journal_mode=PERSIST');c.executescript(sys.argv[2]);c.close()", path.to_str().unwrap(), statement], None).unwrap();
}
fn seed(f: &Fixture) {
    for name in ["state", "accounts", "units", ".config/systemd/user"] {
        files::private_directory(&f.root.join(name)).unwrap();
    }
    let script = r#"
import os,pathlib,sqlite3,json,sys
os.umask(0o077)
root=pathlib.Path(sys.argv[1]);state=root/'state';schemas=json.load(sys.stdin)
session='11111111-1111-4111-8111-111111111111'; incarnation='22222222-2222-4222-8222-222222222222';command='33333333-3333-4333-8333-333333333333'
db=sqlite3.connect(state/'catalogue.sqlite3');db.executescript(schemas['catalogue'])
registration={'protocol':1,'session_id':session,'incarnation':incarnation,'command_id':command,'executable':'/owned-fixture/voyage','workspace':str(state),'state':'suspended','token':'synthetic-owned-test-only','config_path':None,'initialize':None,'restart_from':None}
db.execute('INSERT INTO voyages VALUES(?,?,?,?,?,?,?)',(session,incarnation,str(state).encode(),'suspended',None,json.dumps(registration),0))
db.execute('INSERT INTO incarnations VALUES(?,?,?,?,?)',(incarnation,session,command,json.dumps(registration),0));db.execute('INSERT INTO catalogue(session_id,incarnation) VALUES(?,?)',(session,incarnation));db.execute('INSERT INTO legacy_imports VALUES(?,?)',('complete-v1',bytes(32)));db.commit();db.executescript(schemas['migration']);db.close()
directory=state/'sessions'/session;(directory/'journal').mkdir(parents=True)
(directory/'registration.json').write_text(json.dumps(registration));(directory/'stopped.json').write_text(json.dumps({'session_id':session,'incarnation':incarnation,'cleanup_observed':True,'suspended':True,'archive':None,'deletion':None}));(directory/('guardian-'+incarnation+'.json')).write_text(json.dumps({'session_id':session,'incarnation':incarnation,'boot_id':'44444444-4444-4444-8444-444444444444','cleanup_observed':True}))
journal=sqlite3.connect(directory/'journal/journal.sqlite3');journal.executescript(schemas['journal']);saved={'id':session,'revision':0,'created_at':'2026-09-30T00:00:00Z','updated_at':'2026-09-30T00:00:00Z','workspace':str(state),'model':'synthetic-model','messages':[],'usage':{'input_tokens':0,'output_tokens':0}};journal.execute('INSERT INTO sessions VALUES(?,?,?,?)',(session,0,json.dumps(saved),1))
for slot in range(256):journal.execute('INSERT INTO notification_outbox VALUES(?,?,?)',(slot,'00000000000000000000',' '*1024))
journal.commit();journal.close();(root/'accounts/accounts.json').write_text('{}')
# A real already-run ordinary owner has materialized every cold lock inode.
# Keep them in the reviewed tree: begin_forward must acquire the actual flocks,
# never add private files after review or skip their evidence.
for lock in (state/'supervisor.lock',directory/'startup.lock',directory/'guardian.lock',directory/'journal'/(session+'.execution.lock')):
 lock.touch(mode=0o600,exist_ok=False)

namespace={'__name__':'fixture_source'};exec(schemas['helper'],namespace);notice=state/'notifications';notice.mkdir(mode=0o700);db=sqlite3.connect(notice/'notifications.sqlite3');db.execute('PRAGMA journal_mode=PERSIST');db.executescript(namespace['NOTIFICATION_SCHEMA']);db.close()
"#;
    let schemas = serde_json::json!({"catalogue":include_str!("../fixtures/legacy-v1.0.2-catalogue.sql"),"migration":include_str!("../../../vessel/src/process/database_v2_migration.sql"),"journal":include_str!("../fixtures/legacy-v1.0.2-journal.sql"),"helper":include_str!("../legacy_update.py")});
    crate::service::command::run(
        Path::new("/usr/bin/python3"),
        &["-I", "-c", script, f.root.to_str().unwrap()],
        Some(&serde_json::to_vec(&schemas).unwrap()),
    )
    .unwrap();
}
fn declared_release(f: &Fixture, name: &str, version: &str, change_on_readiness: u8) -> PathBuf {
    let bin = crate::fixture_tests::release(f, name, version);
    let payload=serde_json::json!([{"session_id":SESSION,"incarnation":INCARNATION,"state":"suspended","cleanup_observed":true}]).to_string();
    let mutation = if change_on_readiness == 1 {
        format!(
            "printf '%s' 'unreviewed-private-writer' > {}\n",
            quote(&f.root.join("accounts/accounts.json"))
        )
    } else if change_on_readiness == 2 {
        format!(
            "/usr/bin/python3 -I -c 'import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute(\"PRAGMA journal_mode=PERSIST\");c.executescript(\"BEGIN IMMEDIATE; PRAGMA user_version=1; COMMIT;\");c.close()' {}\n",
            quote(&f.root.join("state/notifications/notifications.sqlite3"))
        )
    } else {
        String::new()
    };
    let body = format!(
        "if [ \"${{1-}}\" = '--version' ]; then printf 'helm {version}\\n'; else {mutation}printf '%s\\n' '{payload}'; fi"
    );
    f.script(&format!("{name}/bin/helm"), &body);
    let mut manifest: Manifest =
        serde_json::from_slice(&fs::read(bin.parent().unwrap().join("release.json")).unwrap())
            .unwrap();
    manifest.binaries.get_mut("helm").unwrap().sha256 = files::hash(&bin.join("helm")).unwrap();
    manifest.update_compatibility = Some(crate::install::release::UpdateCompatibility {
        schema_version: 1,
        formats: current_forward_formats().unwrap(),
        implementation_sha256: "a".repeat(64),
        build_inputs_sha256: "b".repeat(64),
    });
    fs::write(
        bin.parent().unwrap().join("release.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    bin
}
impl Owned {
    fn new(change_on_readiness: u8) -> Self {
        let f = Fixture::new();
        let oldbin = crate::fixture_tests::release(&f, "published-old", "1.0.2");
        let old = install::run(install::Options {
            bin_dir: oldbin,
            replace_existing: false,
            dry_run: false,
        })
        .unwrap()
        .release;
        let runningbin = declared_release(&f, "approved-running", "1.0.3", 0);
        let running = install::run(install::Options {
            bin_dir: runningbin,
            replace_existing: false,
            dry_run: false,
        })
        .unwrap()
        .release;
        install::rollback(false).unwrap();
        assert_eq!(current().unwrap(), old);
        // Materialize the old pointer before seeding the already-migrated fixture.
        // Never select a legacy binary over an existing schema2 database.
        seed(&f);
        let incoming = declared_release(&f, "qualified-target", "1.0.4", change_on_readiness);
        let target = Manifest::inspect(&incoming).unwrap().id().unwrap();
        let health = Health::new(f.root.clone(), target.clone());
        let original = Record {
            operation_id: ORIGINAL.into(),
            channel: "nightly".into(),
            phase: "unconfirmed".into(),
            message: "original outcome remains unknown".into(),
            created_at: 1,
            updated_at: 1,
            current_release: old.clone(),
            release_id: Some(running.clone()),
            version: Some("1.0.3".into()),
            description: None,
            bin_dir: None,
            staging_root: None,
            gateways: vec![],
            contracts_sha256: None,
            recovered_installed_contract: None,
            supervisor_activation: None,
            legacy_mode: false,
            legacy_proof: None,
            legacy_accounts: None,
        };
        files::atomic_json(&path(ORIGINAL).unwrap(), &original).unwrap();
        let original_bytes = fs::read(path(ORIGINAL).unwrap()).unwrap();
        let owned = Self {
            health,
            f,
            old,
            running,
            target,
            incoming,
            original_bytes,
        };
        for (pid, id, gateway) in [
            (OLD_PID, &owned.running, false),
            (OLD_GATEWAY_PID, &owned.running, true),
            (NEW_PID, &owned.target, false),
            (NEW_GATEWAY_PID, &owned.target, true),
        ] {
            symlink(
                owned
                    .f
                    .root
                    .join("install/releases")
                    .join(id)
                    .join("bin/vessel"),
                owned.f.root.join(format!("pid-{pid}")),
            )
            .unwrap();
            let executable = if gateway && pid == NEW_GATEWAY_PID {
                owned.f.root.join("install/current/bin/vessel")
            } else {
                owned
                    .f
                    .root
                    .join("install/releases")
                    .join(id)
                    .join("bin/vessel")
            };
            let args = if gateway {
                format!(
                    "{}\0--bind\0{}\0--process-directory\0{}\0{}\0{}\0",
                    executable.display(),
                    owned.health.address,
                    owned.f.root.join("state").display(),
                    if pid == NEW_GATEWAY_PID {
                        "--public-origin"
                    } else {
                        ORIGIN
                    },
                    if pid == NEW_GATEWAY_PID { ORIGIN } else { "" }
                )
            } else {
                format!(
                    "{}\0local-serve\0--directory\0{}\0",
                    executable.display(),
                    owned.f.root.join("state").display()
                )
            };
            fs::write(owned.f.root.join(format!("cmdline-{pid}")), args).unwrap();
            fs::write(owned.f.root.join(format!("pid-{pid}-environ")), b"").unwrap();
        }
        let unit = supervisor_unit(&owned.running_bin(), &owned.f.root.join("state"));
        fs::write(owned.f.root.join("units/voyage-vessel.service"), unit).unwrap();
        fs::set_permissions(
            owned.f.root.join("units/voyage-vessel.service"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let gateway = format!(
            "[Service]\nExecStart={} --bind {} --process-directory {} {ORIGIN}\nRestart=on-failure\n",
            owned.running_bin().join("vessel").display(),
            owned.health.address,
            owned.f.root.join("state").display()
        );
        fs::write(owned.gateway_path(), gateway).unwrap();
        fs::set_permissions(owned.gateway_path(), fs::Permissions::from_mode(0o600)).unwrap();
        owned
    }
    fn running_bin(&self) -> PathBuf {
        self.f
            .root
            .join("install/releases")
            .join(&self.running)
            .join("bin")
    }
    fn target_bin(&self) -> PathBuf {
        self.f
            .root
            .join("install/releases")
            .join(&self.target)
            .join("bin")
    }
    fn gateway_path(&self) -> PathBuf {
        self.f.root.join(".config/systemd/user").join(GATEWAY)
    }
    fn remote_query(&self, unit: &str, property: &str, value: &str) {
        self.f.call(
            &["show", unit, &format!("--property={property}"), "--value"],
            value,
        );
    }
    fn plan(&self, active: bool, pid: u32) {
        self.f.effective(true);
        self.f
            .query("ActiveState", if active { "active" } else { "inactive" });
        self.f.query("UnitFileState", "enabled");
        if active {
            self.f.query("MainPID", &pid.to_string());
        }
        self.f.query("InvocationID", "owned-private-invocation");
    }
    fn activation(&self, pid: u32) {
        self.plan(true, pid);
        self.f.effective(true);
    }
    fn verified(&self, unit: &str, pid: u32) {
        self.remote_query(unit, "ActiveState", "active");
        self.remote_query(unit, "MainPID", &pid.to_string());
    }
    fn services(&self) {
        self.f.call(
            &[
                "list-units",
                "--type=service",
                "--state=running",
                "--output=json",
            ],
            &serde_json::json!([{"unit":SUPERVISOR},{"unit":GATEWAY}]).to_string(),
        );
    }
    fn credentials(&self) {
        self.remote_query(GATEWAY, "DropInPaths", "");
        self.remote_query(GATEWAY, "Environment", "");
    }
    fn gateway_check(&self, pid: u32) {
        self.remote_query(
            GATEWAY,
            "FragmentPath",
            self.gateway_path().to_str().unwrap(),
        );
        self.remote_query(GATEWAY, "UnitFileState", "enabled");
        self.verified(GATEWAY, pid);
        self.remote_query(GATEWAY, "MainPID", &pid.to_string());
        self.credentials();
    }
    fn effective_gateway(&self) {
        self.f.call(
            &[
                "show",
                GATEWAY,
                "--property=ExecStart,FragmentPath,DropInPaths",
            ],
            "ExecStart=owned immutable fixture command\nFragmentPath=owned fixture\nDropInPaths=\n",
        );
    }
    fn queue_prepare(&self) {
        self.plan(true, OLD_PID);
        self.activation(OLD_PID);
        self.verified(SUPERVISOR, OLD_PID);
        self.remote_query(SUPERVISOR, "MainPID", &OLD_PID.to_string());
        self.verified(GATEWAY, OLD_GATEWAY_PID);
        self.remote_query(GATEWAY, "MainPID", &OLD_GATEWAY_PID.to_string());
        self.services();
        self.effective_gateway();
        self.credentials();
        self.remote_query(GATEWAY, "UnitFileState", "enabled");
        self.gateway_check(OLD_GATEWAY_PID);
        self.remote_query(GATEWAY, "MainPID", &OLD_GATEWAY_PID.to_string());
    }
    fn prepare(&self) -> Recovery {
        self.queue_prepare();
        crate::remote::recover_user(
            &[
                "prepare",
                OPERATION,
                "--original-operation",
                ORIGINAL,
                "--running-release",
                &self.running,
                "--bin-dir",
                self.incoming.to_str().unwrap(),
                "--gateway-unit",
                GATEWAY,
                "--public-origin",
                ORIGIN,
            ]
            .map(str::to_owned),
        )
        .unwrap();
        self.f.done();
        serde_json::from_slice(&fs::read(record_path(OPERATION).unwrap()).unwrap()).unwrap()
    }
    fn queue_before_effects(&self) {
        self.activation(OLD_PID);
        self.gateway_check(OLD_GATEWAY_PID);
        self.services();
        self.effective_gateway();
        self.remote_query(SUPERVISOR, "MainPID", &OLD_PID.to_string());
    }
    fn queue_quiesce(&self) {
        self.plan(true, OLD_PID);
        self.f.effective(true);
        self.f.query("MainPID", &OLD_PID.to_string());
        self.f.call(&["--no-block", "stop", SUPERVISOR], "");
        self.f.query("ActiveState", "inactive");
    }
    fn queue_target_configuration(&self) {
        self.plan(false, 0);
        self.f.call(&["daemon-reload"], "");
        self.f.effective(true);
        self.f.effective(true);
        self.f.call(&["reset-failed", SUPERVISOR], "");
        self.f.call(&["--no-block", "start", SUPERVISOR], "");
        self.f.query("ActiveState", "active");
        self.f.query("MainPID", &NEW_PID.to_string());
    }
    fn queue_effects(&self) {
        self.f.call(&["stop", GATEWAY], "");
        self.queue_quiesce();
        self.queue_target_configuration();
        self.f.call(&["daemon-reload"], "");
        self.f.call(&["reset-failed", GATEWAY], "");
        self.f.call(&["start", GATEWAY], "");
    }
    fn queue_observe(&self) {
        self.activation(NEW_PID);
        self.remote_query(SUPERVISOR, "MainPID", &NEW_PID.to_string());
        self.gateway_check(NEW_GATEWAY_PID);
        self.remote_query(GATEWAY, "MainPID", &NEW_GATEWAY_PID.to_string());
    }
    fn original_unchanged(&self) {
        assert_eq!(
            fs::read(path(ORIGINAL).unwrap()).unwrap(),
            self.original_bytes
        );
        assert_eq!(load(ORIGINAL).unwrap().phase, "unconfirmed");
    }
    fn no_replay(&self, record: &mut Recovery) {
        let hash = review_hash(&record.review).unwrap();
        let before = fs::read(record_path(OPERATION).unwrap()).unwrap();
        assert!(apply_entry(record, &hash).is_err());
        assert_eq!(fs::read(record_path(OPERATION).unwrap()).unwrap(), before);
    }
}
fn apply_entry(record: &mut Recovery, approved: &str) -> Result<()> {
    let result = crate::remote::recover_user(&[
        "apply".into(),
        record.review.operation_id.clone(),
        "--review".into(),
        approved.into(),
    ]);
    *record = serde_json::from_slice(&fs::read(record_path(&record.review.operation_id)?)?)?;
    result
}
// This is the maintained unit's source template, not a test-created service API.
// The bytes are checked again by production unit::recognized and render on apply.
fn supervisor_unit(bin: &Path, state: &Path) -> String {
    format!(
        "[Unit]\nDescription=Voyage Vessel session supervisor\n\n[Service]\nType=simple\nExecStart=:\"{}\" local-serve --directory \"{}\" --voyage-binary \"{}\"\nWorkingDirectory=%h\nUMask=0077\nRestart=on-failure\nRestartSec=2\nKillMode=process\nTimeoutStopSec=30\nStandardInput=null\nStandardOutput=journal\nStandardError=journal\n\n[Install]\nWantedBy=default.target\n",
        bin.join("vessel").display(),
        state.display(),
        bin.join("voyage").display()
    )
}
#[test]
fn ordinary_forward_publishes_exact_target_keeps_original_unknown_and_all_canonical_bytes() {
    for idle_header_drift in [false, true] {
        let owned = Owned::new(0);
        let mut record = owned.prepare();
        let source = record.review.source_evidence.clone();
        let approved = review_hash(&record.review).unwrap();
        if idle_header_drift {
            notification_sql(
                &owned
                    .f
                    .root
                    .join("state/notifications/notifications.sqlite3"),
                "BEGIN IMMEDIATE; PRAGMA user_version=1; COMMIT;",
            );
        }
        owned.queue_before_effects();
        owned.queue_effects();
        owned.queue_observe();
        apply_entry(&mut record, &approved).unwrap();
        owned.f.done();
        assert_eq!(record.phase, "complete");
        assert_eq!(current().unwrap(), owned.target);
        owned.original_unchanged();
        let proof = record.proof.as_ref().unwrap();
        assert!(proof.stage.join("legacy-catalogue.sqlite3").is_file());
        assert!(!owned.f.root.join("state/update-quarantine.json").exists());
        assert!(supersedes(&load(ORIGINAL).unwrap()).unwrap());
        let current = crate::legacy::forward_evidence(
            &owned.f.root.join("state"),
            &owned.f.root.join("accounts"),
            &owned.f.root.join("install"),
        )
        .unwrap();
        for key in [
            "canonical_sha256",
            "accounts_sha256",
            "sessions",
            "observed_formats",
            "review_state_sha256",
        ] {
            assert_eq!(current[key], source[key], "retained {key}");
        }
        assert_eq!(
            current["observed_formats"],
            serde_json::json!({"catalogue_schema":2,"journal_schemas":[12],"process_protocols":[1]})
        );
        if idle_header_drift {
            assert_ne!(proof.evidence["state_sha256"], source["state_sha256"]);
        }
        owned.no_replay(&mut record);
        crate::remote::recover_user(&["status".into(), OPERATION.into()]).unwrap();
        owned.f.done();
    }
}
#[test]
fn independently_changed_ordinary_context_refuses_before_quarantine_or_publication() {
    for variant in 0..14 {
        let owned = Owned::new(0);
        let mut record = owned.prepare();
        let approved = review_hash(&record.review).unwrap();
        let mut _live_lease = None;
        match variant {
            0 => fs::write(
                path(ORIGINAL).unwrap(),
                b"independent original receipt mutation",
            )
            .unwrap(),
            1 => {
                let p = owned.f.root.join("install/transaction.json");
                let mut j: serde_json::Value =
                    serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
                j["previous"] = serde_json::Value::Null;
                json_file(&p, &j);
            }
            2 => {
                fs::remove_file(owned.f.root.join("install/current")).unwrap();
                symlink(
                    owned.f.root.join("install/releases").join(&owned.running),
                    owned.f.root.join("install/current"),
                )
                .unwrap();
            }
            3 => {
                let p = owned.running_bin().parent().unwrap().join("release.json");
                let mut b = fs::read(&p).unwrap();
                b.push(b'\n');
                fs::write(p, b).unwrap();
            }
            4 => {
                let p = record
                    .review
                    .staged_bin
                    .parent()
                    .unwrap()
                    .join("release.json");
                let mut b = fs::read(&p).unwrap();
                b.push(b'\n');
                fs::write(p, b).unwrap();
            }
            5 => fs::write(
                record.review.staged_bin.join("vessel"),
                b"changed executable",
            )
            .unwrap(),
            6 => fs::write(
                owned.f.root.join("accounts/accounts.json"),
                b"private actor changed accounts",
            )
            .unwrap(),
            7 => files::write_new(
                &owned.f.root.join("state/independent.json"),
                b"new private state",
            )
            .unwrap(),
            8 => notification_sql(
                &owned
                    .f
                    .root
                    .join("state/notifications/notifications.sqlite3"),
                "UPDATE clock SET now=1 WHERE id=1;",
            ),
            9 => json_file(
                &owned
                    .f
                    .root
                    .join(format!("state/sessions/{SESSION}/stopped.json")),
                &serde_json::json!({"session_id":SESSION,"incarnation":INCARNATION,"cleanup_observed":false}),
            ),
            10 => {
                let mut other = load(ORIGINAL).unwrap();
                other.operation_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc".into();
                files::atomic_json(&path(&other.operation_id).unwrap(), &other).unwrap();
            }
            12 => {
                _live_lease = Some(
                    files::lock(
                        &owned
                            .f
                            .root
                            .join(format!("state/sessions/{SESSION}/guardian.lock")),
                    )
                    .unwrap(),
                );
            }
            13 => execute_sql(
                &owned
                    .f
                    .root
                    .join(format!("state/sessions/{SESSION}/journal/journal.sqlite3")),
                "UPDATE attachment_schema SET version=21;",
            ),
            _ => {
                let p = owned.gateway_path();
                let mut b = fs::read(&p).unwrap();
                b.extend_from_slice(b"# independent operator edit\n");
                fs::write(p, b).unwrap();
            }
        }
        let original = fs::read(path(ORIGINAL).unwrap()).unwrap();
        let pointer = fs::read_link(owned.f.root.join("install/current")).unwrap();
        let journal = fs::read(owned.f.root.join("install/transaction.json")).unwrap();
        // Only observation responses are queued. An attempted stop/start cannot
        // consume a matching call and therefore fails the test, not the manager.
        if variant >= 6 {
            owned.queue_before_effects();
        }
        assert!(
            apply_entry(&mut record, &approved).is_err(),
            "variant {variant}"
        );
        assert_eq!(record.phase, "reviewed");
        assert!(!owned.f.root.join("state/update-quarantine.json").exists());
        assert!(record.proof.is_none());
        assert_eq!(fs::read(path(ORIGINAL).unwrap()).unwrap(), original);
        assert_eq!(
            fs::read_link(owned.f.root.join("install/current")).unwrap(),
            pointer
        );
        assert_eq!(
            fs::read(owned.f.root.join("install/transaction.json")).unwrap(),
            journal
        );
        assert!(
            !directory()
                .unwrap()
                .join(OPERATION)
                .join("legacy-catalogue.sqlite3")
                .exists()
        );
    }
}
#[test]
fn approved_gateway_stop_failure_remains_unknown_without_snapshot_or_automatic_rollback() {
    let owned = Owned::new(0);
    let mut record = owned.prepare();
    let approved = review_hash(&record.review).unwrap();
    owned.queue_before_effects();
    owned
        .f
        .fail(&["stop", GATEWAY], "bounded owned manager refusal");
    assert!(apply_entry(&mut record, &approved).is_err());
    owned.f.done();
    assert_eq!(record.phase, "unconfirmed");
    assert_eq!(current().unwrap(), owned.old);
    assert!(record.proof.is_none());
    assert!(owned.f.root.join("state/update-quarantine.json").is_file());
    owned.original_unchanged();
    owned.no_replay(&mut record);
    crate::remote::recover_user(&["status".into(), OPERATION.into()]).unwrap();
    owned.f.done();
    assert!(!supersedes(&load(ORIGINAL).unwrap()).unwrap());
}
#[test]
fn quiescent_target_start_failure_retains_published_target_raw_snapshot_and_original_unknown() {
    let owned = Owned::new(0);
    let mut record = owned.prepare();
    let approved = review_hash(&record.review).unwrap();
    owned.queue_before_effects();
    owned.f.call(&["stop", GATEWAY], "");
    owned.queue_quiesce();
    owned.plan(false, 0);
    owned.f.call(&["daemon-reload"], "");
    owned.f.effective(true);
    owned.f.effective(true);
    owned.f.fail(
        &["reset-failed", SUPERVISOR],
        "owned target activation refused",
    );
    assert!(apply_entry(&mut record, &approved).is_err());
    owned.f.done();
    assert_eq!(record.phase, "unconfirmed");
    assert_eq!(current().unwrap(), owned.target);
    let proof = record.proof.as_ref().unwrap();
    assert!(proof.stage.join("legacy-catalogue.sqlite3").is_file());
    assert_eq!(
        files::hash(&proof.stage.join("legacy-catalogue.sqlite3")).unwrap(),
        proof.evidence["backup_sha256"].as_str().unwrap()
    );
    assert!(owned.f.root.join("state/update-quarantine.json").is_file());
    owned.original_unchanged();
    owned.no_replay(&mut record);
    // Observation of a stopped target must not start it or infer readiness.
    owned.plan(false, 0);
    crate::remote::recover_user(&["status".into(), OPERATION.into()]).unwrap();
    owned.f.done();
    let saved: Recovery =
        serde_json::from_slice(&fs::read(record_path(OPERATION).unwrap()).unwrap()).unwrap();
    assert_eq!(saved.phase, "unconfirmed");
    assert!(!supersedes(&load(ORIGINAL).unwrap()).unwrap());
}
#[test]
fn approved_target_private_writer_breaks_held_proof_and_cannot_be_hidden_by_status() {
    let owned = Owned::new(1);
    let mut record = owned.prepare();
    let approved = review_hash(&record.review).unwrap();
    owned.queue_before_effects();
    owned.queue_effects();
    assert!(apply_entry(&mut record, &approved).is_err());
    owned.f.done();
    assert_eq!(record.phase, "unconfirmed");
    assert_eq!(current().unwrap(), owned.target);
    assert_eq!(
        fs::read(owned.f.root.join("accounts/accounts.json")).unwrap(),
        b"unreviewed-private-writer"
    );
    assert!(
        record
            .proof
            .as_ref()
            .unwrap()
            .stage
            .join("legacy-catalogue.sqlite3")
            .is_file()
    );
    assert!(owned.f.root.join("state/update-quarantine.json").is_file());
    owned.original_unchanged();
    owned.no_replay(&mut record);
    crate::remote::recover_user(&["status".into(), OPERATION.into()]).unwrap();
    owned.f.done();
    let saved: Recovery =
        serde_json::from_slice(&fs::read(record_path(OPERATION).unwrap()).unwrap()).unwrap();
    assert_eq!(saved.phase, "unconfirmed");
}
#[test]
fn completed_recovery_snapshot_and_target_cannot_be_replaced_to_supersede_original_receipt() {
    for change_backup in [false, true] {
        let owned = Owned::new(0);
        let mut record = owned.prepare();
        let approved = review_hash(&record.review).unwrap();
        owned.queue_before_effects();
        owned.queue_effects();
        owned.queue_observe();
        apply_entry(&mut record, &approved).unwrap();
        owned.f.done();
        assert!(supersedes(&load(ORIGINAL).unwrap()).unwrap());
        let p = if change_backup {
            record
                .proof
                .as_ref()
                .unwrap()
                .stage
                .join("legacy-catalogue.sqlite3")
        } else {
            owned.target_bin().join("vessel")
        };
        let mut bytes = fs::read(&p).unwrap();
        bytes.extend_from_slice(b"coordinated fixture tamper");
        fs::write(&p, bytes).unwrap();
        assert!(supersedes(&load(ORIGINAL).unwrap()).is_err());
        owned.original_unchanged();
        owned.f.done();
    }
}

#[test]
fn target_idle_notification_header_write_still_breaks_strict_held_raw_proof() {
    let owned = Owned::new(2);
    let mut record = owned.prepare();
    let approved = review_hash(&record.review).unwrap();
    owned.queue_before_effects();
    owned.queue_effects();
    assert!(apply_entry(&mut record, &approved).is_err());
    owned.f.done();
    assert_eq!(record.phase, "unconfirmed");
    assert_eq!(current().unwrap(), owned.target);
    owned.original_unchanged();
    let proof = record.proof.as_ref().unwrap();
    assert!(proof.stage.join("legacy-catalogue.sqlite3").is_file());
    let current = crate::legacy::forward_evidence(
        &owned.f.root.join("state"),
        &owned.f.root.join("accounts"),
        &owned.f.root.join("install"),
    )
    .unwrap();
    assert_eq!(
        current["review_state_sha256"],
        record.review.source_evidence["review_state_sha256"]
    );
    assert_ne!(
        current["state_sha256"], proof.evidence["state_sha256"],
        "semantic equality must not bypass held raw proof"
    );
    owned.no_replay(&mut record);
    crate::remote::recover_user(&["status".into(), OPERATION.into()]).unwrap();
    owned.f.done();
    let saved: Recovery =
        serde_json::from_slice(&fs::read(record_path(OPERATION).unwrap()).unwrap()).unwrap();
    assert_eq!(saved.phase, "unconfirmed");
    assert!(owned.f.root.join("state/update-quarantine.json").exists());
}
#[test]
fn exact_ready_target_can_reconcile_observation_without_install_start_or_old_apply_replay() {
    let owned = Owned::new(0);
    let mut record = owned.prepare();
    let approved = review_hash(&record.review).unwrap();
    owned
        .health
        .reject_target_once
        .store(true, Ordering::Release);
    owned.queue_before_effects();
    owned.queue_effects();
    owned.queue_observe();
    assert!(apply_entry(&mut record, &approved).is_err());
    owned.f.done();
    assert_eq!(record.phase, "unconfirmed");
    assert_eq!(current().unwrap(), owned.target);
    assert!(owned.f.root.join("state/update-quarantine.json").is_file());
    owned.original_unchanged();
    owned.no_replay(&mut record);
    let pointer = fs::read_link(owned.f.root.join("install/current")).unwrap();
    let journal = fs::read(owned.f.root.join("install/transaction.json")).unwrap();
    let snapshot = files::hash(
        &record
            .proof
            .as_ref()
            .unwrap()
            .stage
            .join("legacy-catalogue.sqlite3"),
    )
    .unwrap();
    // Only current-service observation is admitted. Any publication, stop/start,
    // enabling or old-update attempt mismatches the existing manager seam.
    owned.queue_observe();
    crate::remote::recover_user(&["status".into(), OPERATION.into()]).unwrap();
    owned.f.done();
    let saved: Recovery =
        serde_json::from_slice(&fs::read(record_path(OPERATION).unwrap()).unwrap()).unwrap();
    assert_eq!(saved.phase, "complete");
    assert!(!owned.f.root.join("state/update-quarantine.json").exists());
    assert_eq!(
        fs::read_link(owned.f.root.join("install/current")).unwrap(),
        pointer
    );
    assert_eq!(
        fs::read(owned.f.root.join("install/transaction.json")).unwrap(),
        journal
    );
    assert_eq!(
        files::hash(
            &saved
                .proof
                .as_ref()
                .unwrap()
                .stage
                .join("legacy-catalogue.sqlite3")
        )
        .unwrap(),
        snapshot
    );
    owned.original_unchanged();
    assert!(supersedes(&load(ORIGINAL).unwrap()).unwrap());
}
#[test]
fn ordinary_system_protocol_discovery_does_not_admit_root_update_effects() {
    let f = Fixture::new();
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "ordinary fixture must run as ordinary account"
    );
    crate::remote::run(&["system".into(), "protocol".into()]).unwrap();
    for action in ["prepare", "apply", "status"] {
        let args = vec![
            "system".into(),
            action.into(),
            OPERATION.into(),
            "nightly".into(),
        ];
        let error = crate::remote::run(&args).unwrap_err().to_string();
        assert!(error.contains("requires root"), "{error}");
    }
    f.done();
}

#[test]
fn independently_held_execution_lease_refuses_quiescent_snapshot_without_cancel_or_restore() {
    let owned = Owned::new(0);
    let mut record = owned.prepare();
    let approved = review_hash(&record.review).unwrap();
    let execution = owned.f.root.join(format!(
        "state/sessions/{SESSION}/journal/{SESSION}.execution.lock"
    ));
    let live = files::lock(&execution).unwrap();
    owned.queue_before_effects();
    owned.f.call(&["stop", GATEWAY], "");
    owned.queue_quiesce();
    assert!(apply_entry(&mut record, &approved).is_err());
    owned.f.done();
    assert_eq!(record.phase, "unconfirmed");
    assert_eq!(current().unwrap(), owned.old);
    assert!(record.proof.is_none());
    assert!(owned.f.root.join("state/update-quarantine.json").exists());
    assert!(
        !directory()
            .unwrap()
            .join(OPERATION)
            .join("legacy-catalogue.sqlite3")
            .exists()
    );
    assert!(
        files::lock(&execution).is_err(),
        "independent writer is never cancelled or borrowed"
    );
    owned.original_unchanged();
    owned.no_replay(&mut record);
    crate::remote::recover_user(&["status".into(), OPERATION.into()]).unwrap();
    owned.f.done();
    drop(live);
}
