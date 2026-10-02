//! Cold production HostBrowser APIs in an owned child data scope. The pipe peer
//! is a fixed Python fixture, never Chromium or a native guardian qualification.
use super::*;
use std::os::unix::fs::PermissionsExt;

const GUARDIAN: &str = r#"
import sys,json,os,uuid,shutil
root=os.environ['HOME']; temporary=sys.argv[3]; mode='normal'
status={'browser':str(uuid.uuid4()),'tab':str(uuid.uuid4()),'open':True,'mode':'agent','epochs':{'tab':1,'document':1,'viewport':1,'control':1,'capture':1},'tabs':[]}
with open('/proc/self/stat') as f:start=int(f.read().rsplit(')',1)[1].split()[19])
with open(os.path.join(root,'fixture-witness.json'),'w') as f:json.dump({'pid':os.getpid(),'start_ticks':start,'parent':os.getppid(),'uid':os.geteuid(),'group':os.getpgrp(),'temporary':temporary},f)
try:
 for line in sys.stdin:
  q=json.loads(line);op=q['op']
  with open(os.path.join(root,'fixture-requests.jsonl'),'a') as f:f.write(json.dumps(q)+'\n')
  if op=='init':mode=q['config'].get('fixture_mode','normal')
  if (op=='init' and mode=='init_failure') or (op=='open' and mode=='open_failure'):
   out={'id':q['id'],'ok':False,'error':{'state':'uncertain','code':'synthetic-failure'}}
  elif op=='open' and mode=='malformed_open':print('not-json',flush=True);break
  else:
   result={'status':status,'value':{'fixture_only':True}}
   if op=='init' or (op=='open' and mode=='refresh_after_open'):result={'accepted':True}
   out={'id':q['id'],'ok':True,'result':result}
  print(json.dumps(out),flush=True)
  if op=='shutdown':break
finally:
 shutil.rmtree(temporary)
 with open(os.path.join(root,'guardian-cleanup.json'),'w') as f:json.dump({'observed':mode!='unconfirmed_cleanup','fixture_only':True},f)
"#;

fn private(path: &std::path::Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}
fn directory(path: &std::path::Path) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
struct Fixture {
    session: Uuid,
    journal: PathBuf,
    launch: Launch,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let session = Uuid::new_v4();
        let journal = home
            .join("sessions")
            .join(session.to_string())
            .join("journal");
        directory(&journal);
        let distribution = home.join("distribution");
        directory(&distribution);
        let worker = distribution.join("worker.mjs");
        private(&worker, b"synthetic pipe peer; not Node/browser code");
        private(&distribution.join("guardian.py"), GUARDIAN.as_bytes());
        let mut config = default_config();
        config["fixture_mode"] = json!(mode);
        Self {
            session,
            journal,
            launch: Launch {
                node: "/usr/bin/python3".into(),
                worker,
                chromium: "/bin/true".into(),
                config,
            },
        }
    }
    fn host(&self) -> Arc<HostBrowser> {
        HostBrowser::new(
            self.journal.clone(),
            self.session,
            Uuid::new_v4(),
            Some(self.launch.clone()),
        )
    }
    fn trace(&self) -> Vec<Value> {
        let file = self.journal.join("host-browser/fixture-requests.jsonl");
        if !file.exists() {
            return Vec::new();
        }
        let raw = std::fs::read(file).unwrap();
        assert!(raw.len() < 65536);
        String::from_utf8(raw)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn slots(&self) -> Vec<Uuid> {
        let root = crate::config::default_data_dir().join("host-browser-capacity");
        (0..16)
            .filter_map(|n| std::fs::read_to_string(root.join(format!("browser-slot-{n}"))).ok())
            .map(|owner| owner.parse().unwrap())
            .collect()
    }
    fn reservations(&self) -> Vec<(String, i64, i64)> {
        let path = crate::config::default_data_dir().join("host-resources/reservations.sqlite3");
        if !path.is_file() {
            return Vec::new();
        }
        let db = rusqlite::Connection::open(path).unwrap();
        let mut query = db
            .prepare("SELECT owner,units,observed FROM reservations ORDER BY id")
            .unwrap();
        query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
    async fn start(
        &self,
        host: &HostBrowser,
        id: Uuid,
        socket: Uuid,
        principal: Uuid,
    ) -> Result<Value> {
        host.human(
            HostBrowserOperation::Start {
                command_id: id,
                incarnation: host.incarnation,
                expected_revision: 17,
            },
            socket,
            principal,
            None,
        )
        .await
    }
    async fn observed_close(&self, host: &HostBrowser) {
        let worker = host.inner.lock().await.worker.clone().unwrap();
        let witness: Value = serde_json::from_slice(
            &std::fs::read(self.journal.join("host-browser/fixture-witness.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(witness["temporary"], json!(worker.temporary));
        assert_eq!(witness["parent"], json!(std::process::id()));
        let pid = witness["pid"].as_u64().unwrap();
        assert_eq!(
            worker.child.lock().await.id(),
            Some(u32::try_from(pid).unwrap())
        );
        assert_eq!(witness["group"], json!(pid));
        assert_eq!(witness["uid"], json!(unsafe { libc::geteuid() }));
        host.close().await.unwrap();
        assert!(!worker.temporary.exists());
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            let start: u64 = stat
                .rsplit_once(')')
                .unwrap()
                .1
                .split_whitespace()
                .nth(19)
                .unwrap()
                .parse()
                .unwrap();
            assert_ne!(
                start,
                witness["start_ticks"].as_u64().unwrap(),
                "original fixture worker survived cleanup"
            );
        }
        assert!(!host.blocks_suspension().await);
        assert!(host.inner.lock().await.capacity.is_none());
        assert!(host.inner.lock().await.reservation.is_none());
        assert!(!self.slots().contains(&self.session));
        assert!(
            self.reservations()
                .iter()
                .all(|(_, _, observed)| *observed == 1)
        );
    }
}

fn sibling_slots(fixture: &Fixture) -> (Uuid, PathBuf, Vec<Uuid>) {
    let sessions = fixture.journal.parent().unwrap().parent().unwrap();
    let owner = Uuid::new_v4();
    let old = sessions.join(owner.to_string());
    let worker = old.join("journal/host-browser");
    directory(&worker);
    private(
        &old.join("registration.json"),
        json!({"session_id":owner}).to_string().as_bytes(),
    );
    private(
        &worker.join("worker.lock"),
        json!({"pid":i32::MAX,"fixture_only":true})
            .to_string()
            .as_bytes(),
    );
    private(&worker.join("guardian-cleanup.json"),json!({"observed":true,"cleanup_complete":true,"descendants_terminated":true,"descendants_reaped":true,"temporary_cleaned":true,"fixture_only":true}).to_string().as_bytes());
    let root = crate::config::default_data_dir().join("host-browser-capacity");
    let mut owners = Vec::new();
    for n in 0..16 {
        let id = if n == 0 { owner } else { Uuid::new_v4() };
        drop(crate::host_browser_capacity::Capacity::acquire(&root, id, 16).unwrap());
        owners.push(id);
    }
    (owner, old, owners)
}

#[test]
fn cold_start_recovers_only_observed_sibling_slot_using_actual_journal_namespace() {
    child(
        "host_browser::cold_lifecycle_tests::cold_start_recovers_only_observed_sibling_slot_using_actual_journal_namespace",
        async {
            let fixture = Fixture::new("normal");
            let (owner, old, owners) = sibling_slots(&fixture);
            let host = fixture.host();
            fixture
                .start(&host, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
                .await
                .unwrap();
            let slots = fixture.slots();
            assert_eq!(slots[0], fixture.session);
            assert_eq!(&slots[1..], &owners[1..]);
            assert!(!old.join("journal/host-browser/worker.lock").exists());
            let audits: Vec<_> = std::fs::read_dir(&old)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("browser-capacity-recovery-")
                })
                .collect();
            assert_eq!(audits.len(), 1);
            let audit: Value = serde_json::from_slice(&std::fs::read(&audits[0]).unwrap()).unwrap();
            assert_eq!(audit["session_id"], json!(owner));
            assert_eq!(audit["external_effects_reconciled"], false);
            fixture.observed_close(&host).await;
            assert_eq!(fixture.slots(), owners[1..]);
        },
    );
}

#[test]
fn cold_full_capacity_busy_sibling_or_unverified_cleanup_refuses_without_releasing_foreign_slots() {
    child(
        "host_browser::cold_lifecycle_tests::cold_full_capacity_busy_sibling_or_unverified_cleanup_refuses_without_releasing_foreign_slots",
        async {
            let fixture = Fixture::new("normal");
            let (_, old, owners) = sibling_slots(&fixture);
            let guardian =
                crate::attachment::journal::open_private_file(&old.join("guardian.lock")).unwrap();
            guardian.try_lock().unwrap();
            let host = fixture.host();
            assert!(
                fixture
                    .start(&host, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
                    .await
                    .is_err()
            );
            assert_eq!(fixture.slots(), owners);
            assert!(fixture.trace().is_empty());
            assert!(!host.blocks_suspension().await);
            drop(guardian);
            private(
                &old.join("journal/host-browser/guardian-cleanup.json"),
                json!({"observed":false,"fixture_only":true})
                    .to_string()
                    .as_bytes(),
            );
            assert!(
                fixture
                    .start(&host, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
                    .await
                    .is_err()
            );
            assert_eq!(fixture.slots(), owners);
            assert!(old.join("journal/host-browser/worker.lock").exists());
            assert!(fixture.trace().is_empty());
            assert!(fixture.reservations().is_empty());
            assert_eq!(
                std::fs::read_dir(&old)
                    .unwrap()
                    .filter(|e| e
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("browser-capacity-recovery-"))
                    .count(),
                0
            );
        },
    );
}
fn child(name: &str, body: impl std::future::Future<Output = ()>) {
    super::owned_child_tests::run(name, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(body)
    });
}

#[test]
fn missing_guardian_distribution_refuses_without_retained_owned_profile_directory() {
    let root = tempfile::tempdir().unwrap();
    let profiles = root.path().join("profiles");
    directory(&profiles);
    let worker = root.path().join("worker.mjs");
    private(&worker, b"never executed");
    let launch = Launch {
        node: "/usr/bin/python3".into(),
        worker,
        chromium: "/bin/true".into(),
        config: default_config(),
    };
    let failure = Worker::spawn_in(&launch, root.path(), &profiles)
        .err()
        .unwrap();
    assert_eq!(
        failure.to_string(),
        "browser guardian distribution unavailable"
    );
    assert_eq!(std::fs::read_dir(&profiles).unwrap().count(), 0);
    assert!(!root.path().join("guardian-cleanup.json").exists());
}

#[test]
fn invalid_distribution_and_profile_parent_refuse_before_any_child_or_kept_profile() {
    let root = tempfile::tempdir().unwrap();
    let profiles = root.path().join("profiles");
    directory(&profiles);
    let worker = root.path().join("worker.mjs");
    private(&worker, b"never executed");
    private(&root.path().join("guardian.py"), GUARDIAN.as_bytes());
    let good = Launch {
        node: "/usr/bin/python3".into(),
        worker,
        chromium: "/bin/true".into(),
        config: default_config(),
    };
    for broken in [
        Launch {
            node: "relative".into(),
            ..good.clone()
        },
        Launch {
            chromium: root.path().join("missing"),
            ..good.clone()
        },
    ] {
        assert!(Worker::spawn_in(&broken, root.path(), &profiles).is_err());
        assert_eq!(std::fs::read_dir(&profiles).unwrap().count(), 0);
    }
    let blocked = root.path().join("blocked-parent");
    private(&blocked, b"retained fixture");
    assert!(Worker::spawn_in(&good, root.path(), &blocked).is_err());
    assert_eq!(std::fs::read(blocked).unwrap(), b"retained fixture");
    assert!(!root.path().join("fixture-witness.json").exists());
}

#[test]
fn cold_public_start_owns_exact_profile_reservations_receipt_and_observed_close() {
    child(
        "host_browser::cold_lifecycle_tests::cold_public_start_owns_exact_profile_reservations_receipt_and_observed_close",
        async {
            let fixture = Fixture::new("normal");
            let host = fixture.host();
            let id = Uuid::new_v4();
            let socket = Uuid::new_v4();
            let principal = Uuid::new_v4();
            private(
                &host.root().unwrap().join("guardian-cleanup.json"),
                b"previous fixture cleanup evidence",
            );
            let reply = fixture.start(&host, id, socket, principal).await.unwrap();
            assert_eq!(reply["status"]["running"], true);
            assert!(!host.root().unwrap().join("guardian-cleanup.json").exists());
            assert!(host.blocks_suspension().await);
            assert_eq!(fixture.slots(), vec![fixture.session]);
            assert_eq!(
                fixture.reservations(),
                vec![(fixture.session.to_string(), 3, 0)]
            );
            let trace = fixture.trace();
            assert_eq!(
                trace
                    .iter()
                    .map(|r| r["op"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["init", "open"]
            );
            assert_eq!(trace[0]["config"]["root"], json!(host.root().unwrap()));
            assert_eq!(
                trace[0]["config"]["executable"],
                json!(fixture.launch.chromium)
            );
            assert_eq!(
                fixture.start(&host, id, socket, principal).await.unwrap()["receipt"]["state"],
                "completed"
            );
            assert_eq!(fixture.trace().len(), 2);
            fixture.observed_close(&host).await;
            assert_eq!(
                host.human(
                    HostBrowserOperation::Receipt { command_id: id },
                    socket,
                    principal,
                    None
                )
                .await
                .unwrap()["receipt"]["state"],
                "completed"
            );
            assert_eq!(
                fixture
                    .trace()
                    .iter()
                    .filter(|r| r["op"] == "shutdown")
                    .count(),
                1
            );
        },
    );
}

#[test]
fn cold_open_without_status_uses_one_readonly_refresh_before_admission() {
    child(
        "host_browser::cold_lifecycle_tests::cold_open_without_status_uses_one_readonly_refresh_before_admission",
        async {
            let fixture = Fixture::new("refresh_after_open");
            let host = fixture.host();
            fixture
                .start(&host, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
                .await
                .unwrap();
            assert_eq!(
                fixture
                    .trace()
                    .iter()
                    .map(|r| r["op"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["init", "open", "status"]
            );
            fixture.observed_close(&host).await;
        },
    );
}

#[test]
fn cold_missing_guardian_releases_pre_spawn_capacity_but_retains_original_unknown_receipt() {
    child(
        "host_browser::cold_lifecycle_tests::cold_missing_guardian_releases_pre_spawn_capacity_but_retains_original_unknown_receipt",
        async {
            let fixture = Fixture::new("normal");
            std::fs::remove_file(fixture.launch.worker.with_file_name("guardian.py")).unwrap();
            let host = fixture.host();
            let id = Uuid::new_v4();
            let socket = Uuid::new_v4();
            let principal = Uuid::new_v4();
            assert_eq!(
                fixture
                    .start(&host, id, socket, principal)
                    .await
                    .unwrap_err()
                    .to_string(),
                "browser guardian distribution unavailable"
            );
            assert!(!host.blocks_suspension().await);
            assert!(fixture.slots().is_empty());
            assert_eq!(
                fixture.reservations(),
                vec![(fixture.session.to_string(), 3, 1)]
            );
            assert!(fixture.trace().is_empty());
            private(
                &fixture.launch.worker.with_file_name("guardian.py"),
                GUARDIAN.as_bytes(),
            );
            assert_eq!(
                fixture.start(&host, id, socket, principal).await.unwrap()["receipt"]["state"],
                "unknown"
            );
            assert!(fixture.trace().is_empty());
            fixture
                .start(&host, Uuid::new_v4(), socket, principal)
                .await
                .unwrap();
            fixture.observed_close(&host).await;
            assert_eq!(
                host.human(
                    HostBrowserOperation::Receipt { command_id: id },
                    socket,
                    principal,
                    None
                )
                .await
                .unwrap()["receipt"]["state"],
                "unknown"
            );
        },
    );
}

#[test]
fn cold_resource_store_failure_releases_only_new_capacity_and_never_spawns() {
    child(
        "host_browser::cold_lifecycle_tests::cold_resource_store_failure_releases_only_new_capacity_and_never_spawns",
        async {
            let fixture = Fixture::new("normal");
            let data = crate::config::default_data_dir();
            directory(&data);
            let blocked = data.join("host-resources");
            private(&blocked, b"owned blocked namespace");
            let host = fixture.host();
            assert!(
                fixture
                    .start(&host, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
                    .await
                    .is_err()
            );
            assert!(!host.blocks_suspension().await);
            assert!(fixture.slots().is_empty());
            assert!(fixture.trace().is_empty());
            assert_eq!(std::fs::read(blocked).unwrap(), b"owned blocked namespace");
        },
    );
}

#[test]
fn cold_init_or_open_failure_retains_cleanup_obligation_and_fences_any_automatic_restart() {
    child(
        "host_browser::cold_lifecycle_tests::cold_init_or_open_failure_retains_cleanup_obligation_and_fences_any_automatic_restart",
        async {
            for mode in ["init_failure", "open_failure", "malformed_open"] {
                let fixture = Fixture::new(mode);
                let host = fixture.host();
                let id = Uuid::new_v4();
                let socket = Uuid::new_v4();
                let principal = Uuid::new_v4();
                assert!(fixture.start(&host, id, socket, principal).await.is_err());
                assert!(host.blocks_suspension().await);
                assert!(host.inner.lock().await.worker.is_some());
                let before = fixture.trace().len();
                assert_eq!(
                    fixture.start(&host, id, socket, principal).await.unwrap()["receipt"]["state"],
                    "unknown"
                );
                assert!(
                    fixture
                        .start(&host, Uuid::new_v4(), socket, principal)
                        .await
                        .is_err()
                );
                assert_eq!(fixture.trace().len(), before);
                fixture.observed_close(&host).await;
            }
        },
    );
}

#[test]
fn cold_journal_shape_refusal_preserves_previous_marker_without_reservation_or_spawn() {
    child(
        "host_browser::cold_lifecycle_tests::cold_journal_shape_refusal_preserves_previous_marker_without_reservation_or_spawn",
        async {
            let fixture = Fixture::new("normal");
            let unusual = fixture.journal.parent().unwrap().join("not-journal");
            directory(&unusual);
            let host = HostBrowser::new(
                unusual,
                fixture.session,
                Uuid::new_v4(),
                Some(fixture.launch.clone()),
            );
            let root = host.root().unwrap();
            private(&root.join("guardian-cleanup.json"), b"retained old marker");
            assert_eq!(
                host.start().await.err().unwrap().to_string(),
                "browser journal/session layout unavailable"
            );
            assert_eq!(
                std::fs::read(root.join("guardian-cleanup.json")).unwrap(),
                b"retained old marker"
            );
            assert!(!host.blocks_suspension().await);
            assert!(fixture.slots().is_empty());
            assert!(fixture.reservations().is_empty());
        },
    );
}

#[test]
fn cleanup_evidence_refusal_retains_resource_obligations_even_after_fixture_process_exit() {
    child(
        "host_browser::cold_lifecycle_tests::cleanup_evidence_refusal_retains_resource_obligations_even_after_fixture_process_exit",
        async {
            let fixture = Fixture::new("unconfirmed_cleanup");
            let host = fixture.host();
            fixture
                .start(&host, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
                .await
                .unwrap();
            let worker = host.inner.lock().await.worker.clone().unwrap();
            assert!(host.close().await.is_err());
            assert!(!worker.temporary.exists());
            assert!(worker.child.lock().await.try_wait().unwrap().is_some());
            assert!(host.blocks_suspension().await);
            assert_eq!(fixture.slots(), vec![fixture.session]);
            assert_eq!(
                fixture.reservations(),
                vec![(fixture.session.to_string(), 3, 0)]
            );
            // This fixture is deliberately unqualified. The private child process's
            // data root is discarded after exit; no observed release is manufactured.
        },
    );
}
