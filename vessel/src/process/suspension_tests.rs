use super::super::{database, test_support::Fixture};
use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn observer_rejects_mutations_and_missing_or_unlaunchable_binaries() {
    let f = Fixture::new();
    let mut r = f.registration();
    assert!(
        observe(None, &f.0, &r, RuntimeCommand::PrepareBrowser, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("requires execution owner")
    );
    assert!(
        observe(None, &f.0, &r, RuntimeCommand::Snapshot, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("binary missing")
    );
    r.executable = Some(f.0.join("absent"));
    assert!(
        observe(None, &f.0, &r, RuntimeCommand::Snapshot, None)
            .await
            .is_err()
    );
}

// Tiny private IPC peers, not Voyage executors. Assert the complete request
// before returning a response, with no credentials or inherited host state.
#[tokio::test]
async fn one_shot_observer_checks_peer_identity_exit_status_and_framing() {
    let f = Fixture::new();
    for scenario in [
        "ok",
        "protocol",
        "session",
        "incarnation",
        "resumed",
        "exit",
        "truncated",
    ] {
        let mut r = f.registration();
        let binary = f.0.join(format!("observer-{scenario}"));
        let grant = GrantBinding {
            grant_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            revision: 7,
        };
        let source = format!(
            r#"#!/usr/bin/env python3
import json, struct, sys
assert sys.argv[1:] == ['observe-suspended', '--directory', {directory:?}]
n = struct.unpack('>I', sys.stdin.buffer.read(4))[0]
r = json.loads(sys.stdin.buffer.read(n))
assert r['session_id'] == {session:?}
assert r['incarnation'] == {incarnation:?}
assert r['token'] == 'offline-fixture-not-a-credential'
assert r['protocol'] == {protocol}
assert r['command']['op'] == 'snapshot'
assert r['authorization']['grant_id'] == {grant_id:?}
assert r['authorization']['revision'] == 7
response = dict(protocol={protocol}, session_id={session:?}, incarnation={incarnation:?}, resumed_from=None, error=None, outcome_unknown=False, result={{'offline': True}})
scenario = {scenario:?}
if scenario == 'protocol': response['protocol'] += 1
if scenario == 'session': response['session_id'] = '00000000-0000-0000-0000-000000000000'
if scenario == 'incarnation': response['incarnation'] = '00000000-0000-0000-0000-000000000000'
if scenario == 'resumed': response['resumed_from'] = {incarnation:?}
payload = json.dumps(response).encode()
sys.stdout.buffer.write(struct.pack('>I', len(payload)))
sys.stdout.buffer.write(payload[:2] if scenario == 'truncated' else payload)
sys.stdout.buffer.flush()
sys.exit(9 if scenario == 'exit' else 0)
"#,
            directory = f.0.to_str().unwrap(),
            session = r.session_id.to_string(),
            incarnation = r.incarnation.to_string(),
            protocol = PROCESS_PROTOCOL,
            grant_id = grant.grant_id.to_string()
        );
        std::fs::write(&binary, source).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        r.executable = Some(binary);
        let result = observe(None, &f.0, &r, RuntimeCommand::Snapshot, Some(grant)).await;
        if scenario == "ok" {
            assert_eq!(result.unwrap().result, serde_json::json!({"offline":true}));
        } else {
            let error = result.unwrap_err().to_string();
            if matches!(scenario, "protocol" | "session" | "incarnation" | "resumed") {
                assert!(
                    error.contains("observer identity mismatch"),
                    "{scenario}: {error}"
                );
            }
            if scenario == "exit" {
                assert!(error.contains("suspended observation failed"), "{error}");
            }
        }
    }
}

#[tokio::test]
async fn lifecycle_lock_is_per_session_and_incarnation_fences_precede_io() {
    let f = Fixture::new();
    let supervisor = f.supervisor().await;
    let first = f.registration();
    let second = f.registration();
    database::save(&f.0, &first).await.unwrap();
    database::save(&f.0, &second).await.unwrap();
    let a = supervisor.lifecycle_lock(first.session_id).await.unwrap();
    assert!(Arc::ptr_eq(
        &a,
        &supervisor.lifecycle_lock(first.session_id).await.unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &a,
        &supervisor.lifecycle_lock(second.session_id).await.unwrap()
    ));
    assert!(supervisor.lifecycle_lock(Uuid::new_v4()).await.is_err());
    for command in [
        RuntimeCommand::Snapshot,
        RuntimeCommand::Events {
            after: 0,
            limit: 1,
            wait_ms: 0,
            projection: None,
        },
    ] {
        assert!(
            supervisor
                .dispatch_session(first.session_id, Some(Uuid::new_v4()), command, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("stale runtime incarnation")
        );
    }
    assert!(
        supervisor
            .stop(first.session_id, Uuid::new_v4(), None)
            .await
            .unwrap_err()
            .to_string()
            .contains("stale runtime incarnation")
    );
    let observer = supervisor.current_observer_registration(&first);
    assert_eq!(observer.executable, Some(supervisor.binary.clone()));
    assert_eq!(observer.incarnation, first.incarnation);
    assert_eq!(first.executable, None);
}

#[tokio::test]
async fn positive_cleanup_evidence_stops_without_launching() {
    let f = Fixture::new();
    let supervisor = f.supervisor().await;
    let r = f.registration();
    database::save(&f.0, &r).await.unwrap();
    let dir = registry::directory(&f.0, r.session_id);
    registry::private_directory(&dir).unwrap();
    super::super::access::store::save(&dir.join("stopped.json"), &serde_json::json!({"session_id":r.session_id,"incarnation":r.incarnation,"cleanup_observed":true,"suspended":true})).unwrap();
    let reply = supervisor
        .stop(r.session_id, r.incarnation, None)
        .await
        .unwrap();
    assert_eq!(
        reply.result,
        serde_json::json!({"status":"stopped","cleanup":"observed"})
    );
    assert_eq!(reply.incarnation, r.incarnation);
    assert!(!reply.outcome_unknown);
    assert_eq!(
        supervisor.registration(r.session_id).await.unwrap().state,
        ProcessState::Stopped
    );
    assert!(!supervisor.binary.exists());
}
#[tokio::test]
async fn remembered_names_validate_and_persist_only_for_current_incarnation() {
    let f = Fixture::new();
    let supervisor = f.supervisor().await;
    let r = f.registration();
    database::save(&f.0, &r).await.unwrap();
    registry::private_directory(&registry::directory(&f.0, r.session_id)).unwrap();
    for name in ["".to_string(), "line\nbreak".into(), "é".repeat(257)] {
        assert!(
            supervisor
                .remember_name(r.session_id, r.incarnation, &name)
                .await
                .is_err()
        );
    }
    assert!(
        supervisor
            .remember_name(r.session_id, Uuid::new_v4(), "valid")
            .await
            .is_err()
    );
    assert!(
        supervisor
            .remember_name(Uuid::new_v4(), r.incarnation, "valid")
            .await
            .is_err()
    );
    let name = "é".repeat(256);
    for _ in 0..2 {
        supervisor
            .remember_name(r.session_id, r.incarnation, &name)
            .await
            .unwrap();
    }
    assert_eq!(
        supervisor
            .registration(r.session_id)
            .await
            .unwrap()
            .name
            .as_deref(),
        Some(name.as_str())
    );
}

#[tokio::test]
async fn saved_reads_after_recovery_keep_incarnation_and_never_launch_executor() {
    for stale_socket in [false, true] {
        let f = Fixture::new();
        let supervisor = f.supervisor().await;
        let mut registration = f.registration();
        registration.state = ProcessState::Unavailable;
        registration.executable = Some(f.0.join("retired-voyage"));
        database::save(&f.0, &registration).await.unwrap();
        let directory = registry::directory(&f.0, registration.session_id);
        registry::private_directory(&directory).unwrap();
        super::super::access::store::save(
            &directory.join("recovered.json"),
            &serde_json::json!({"session_id":registration.session_id,
                "incarnation":registration.incarnation,"restart_permitted":true,
                "cleanup_disposition":"observed"}),
        )
        .unwrap();
        if stale_socket {
            let listener =
                std::os::unix::net::UnixListener::bind(directory.join("runtime.sock")).unwrap();
            drop(listener); // A pathname alone is not a live owner.
        }
        let source = r#"#!/usr/bin/env python3
import json, pathlib, struct, sys
pathlib.Path(sys.argv[0]).with_suffix('.calls').open('a').write(sys.argv[1]+'\n')
assert sys.argv[1] == 'observe-suspended', 'observation launched an executor'
n=struct.unpack('>I',sys.stdin.buffer.read(4))[0]
r=json.loads(sys.stdin.buffer.read(n))
assert r['command']['op'] in ('snapshot','history')
reply=dict(protocol=r['protocol'],session_id=r['session_id'],incarnation=r['incarnation'],resumed_from=None,error=None,outcome_unknown=False,result={'saved':True})
payload=json.dumps(reply).encode()
sys.stdout.buffer.write(struct.pack('>I',len(payload))+payload)
"#;
        std::fs::write(&supervisor.binary, source).unwrap();
        std::fs::set_permissions(&supervisor.binary, std::fs::Permissions::from_mode(0o700))
            .unwrap();
        for command in [
            RuntimeCommand::Snapshot,
            RuntimeCommand::History {
                offset: 0,
                limit: 1,
                expected_revision: None,
            },
        ] {
            let result = supervisor
                .dispatch_session(registration.session_id, None, command, None)
                .await
                .unwrap();
            assert_eq!(result.incarnation, registration.incarnation);
            assert!(result.resumed_from.is_none());
            assert_eq!(result.result, serde_json::json!({"saved":true}));
            assert_eq!(
                serde_json::to_value(
                    supervisor
                        .registration(registration.session_id)
                        .await
                        .unwrap()
                )
                .unwrap(),
                serde_json::to_value(&registration).unwrap()
            );
        }
        assert_eq!(
            std::fs::read_to_string(supervisor.binary.with_extension("calls")).unwrap(),
            "observe-suspended\nobserve-suspended\n"
        );
    }
}

#[tokio::test]
async fn suspended_snapshots_coalesce_and_invalidate_without_caching_scoped_reads() {
    let f = Fixture::new();
    let supervisor = Arc::new(f.supervisor().await);
    let r = f.registration();
    let directory = registry::directory(&f.0, r.session_id);
    registry::private_directory(&directory).unwrap();
    let marker = directory.join("stopped.json");
    let stopped = serde_json::json!({"session_id":r.session_id,"incarnation":r.incarnation,"cleanup_observed":true,"suspended":true});
    super::super::access::store::save(&marker, &stopped).unwrap();
    let launches = f.0.join("launches");
    std::fs::write(&launches, "").unwrap();
    let source = format!(
        r#"#!/usr/bin/env python3
import json, struct, sys, time
with open({launches:?}, 'a') as log: log.write('observe\n')
n = struct.unpack('>I', sys.stdin.buffer.read(4))[0]
r = json.loads(sys.stdin.buffer.read(n))
time.sleep(0.05)
v = dict(protocol=1, session_id=r['session_id'], incarnation=r['incarnation'], outcome_unknown=False, resumed_from=None, result={{'revision':1}}, error=None)
b = json.dumps(v).encode(); sys.stdout.buffer.write(struct.pack('>I', len(b)) + b)
"#
    );
    std::fs::write(&supervisor.binary, source).unwrap();
    std::fs::set_permissions(&supervisor.binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut readers = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let supervisor = supervisor.clone();
        let r = r.clone();
        let directory = directory.clone();
        readers.spawn(async move {
            supervisor
                .observe_current(&directory, &r, RuntimeCommand::Snapshot, None)
                .await
                .unwrap()
        });
    }
    while let Some(result) = readers.join_next().await {
        assert_eq!(result.unwrap().result["revision"], 1);
    }
    assert_eq!(
        std::fs::read_to_string(&launches).unwrap().lines().count(),
        1
    );
    // An atomic lifecycle marker replacement invalidates even identical content.
    super::super::access::store::save(&marker, &stopped).unwrap();
    supervisor
        .observe_current(&directory, &r, RuntimeCommand::Snapshot, None)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&launches).unwrap().lines().count(),
        2
    );
    let binding = GrantBinding {
        grant_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        revision: 1,
    };
    for _ in 0..2 {
        supervisor
            .observe_current(
                &directory,
                &r,
                RuntimeCommand::Snapshot,
                Some(binding.clone()),
            )
            .await
            .unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&launches).unwrap().lines().count(),
        4
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(marker).unwrap()).unwrap(),
        stopped
    );
}

#[test]
fn saved_snapshot_fingerprint_detects_journal_and_configuration_replacement() {
    let f = Fixture::new();
    let mut r = f.registration();
    let journal = f.0.join("journal");
    std::fs::create_dir(&journal).unwrap();
    r.config_path = Some(f.0.join("config"));
    std::fs::write(r.config_path.as_ref().unwrap(), "old").unwrap();
    let original = snapshot_identity(&f.0, &r).unwrap();
    std::fs::write(journal.join("journal.sqlite3-wal"), "change").unwrap();
    let changed = snapshot_identity(&f.0, &r).unwrap();
    assert_ne!(original, changed);
    std::fs::write(r.config_path.as_ref().unwrap(), "new configuration").unwrap();
    assert_ne!(changed, snapshot_identity(&f.0, &r).unwrap());
    r.incarnation = Uuid::new_v4();
    assert_ne!(original, snapshot_identity(&f.0, &r).unwrap());
}

#[tokio::test]
async fn cache_retention_is_bounded_and_does_not_split_inflight_hydration() {
    let cache = ObservationCache::default();
    let pinned_id = Uuid::new_v4();
    let pinned = cache.entry(pinned_id).await;
    for _ in 0..1024 {
        drop(cache.entry(Uuid::new_v4()).await);
    }
    assert!(cache.entries.lock().await.len() <= 512);
    assert!(Arc::ptr_eq(&pinned, &cache.entry(pinned_id).await));
}

// Bounded process-level measurement, isolated from providers and installed
// services. The peer counts actual observer execs; it is not a real journal cost
// benchmark and must never be reported as native whole-product qualification.
#[tokio::test]
async fn retained_257_two_client_observer_spawn_measurement() {
    fn child_cpu() -> f64 {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        assert_eq!(
            unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) },
            0
        );
        let usage = unsafe { usage.assume_init() };
        usage.ru_utime.tv_sec as f64
            + usage.ru_utime.tv_usec as f64 / 1_000_000.0
            + usage.ru_stime.tv_sec as f64
            + usage.ru_stime.tv_usec as f64 / 1_000_000.0
    }
    let f = Fixture::new();
    let supervisor = f.supervisor().await;
    let launches = f.0.join("launches");
    std::fs::write(&launches, "").unwrap();
    let source = format!(
        r#"#!/usr/bin/env python3
import json, struct, sys
with open({launches:?}, 'a') as log: log.write('observe\n')
n = struct.unpack('>I', sys.stdin.buffer.read(4))[0]; r = json.loads(sys.stdin.buffer.read(n))
v = dict(protocol=1, session_id=r['session_id'], incarnation=r['incarnation'], outcome_unknown=False, resumed_from=None, result={{'revision':1}}, error=None)
b = json.dumps(v).encode(); sys.stdout.buffer.write(struct.pack('>I', len(b)) + b)
"#
    );
    std::fs::write(&supervisor.binary, source).unwrap();
    std::fs::set_permissions(&supervisor.binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut records = Vec::new();
    for _ in 0..257 {
        let mut r = f.registration();
        r.executable = Some(supervisor.binary.clone());
        let directory = registry::directory(&f.0, r.session_id);
        registry::private_directory(&directory).unwrap();
        super::super::access::store::save(&directory.join("stopped.json"), &serde_json::json!({"session_id":r.session_id,"incarnation":r.incarnation,"cleanup_observed":true,"suspended":true})).unwrap();
        records.push((directory, r));
    }
    let cpu = child_cpu();
    let start = std::time::Instant::now();
    for _ in 0..2 {
        for (directory, r) in &records {
            observe(None, directory, r, RuntimeCommand::Snapshot, None)
                .await
                .unwrap();
        }
    }
    let before_cpu = child_cpu() - cpu;
    let before_elapsed = start.elapsed().as_secs_f64();
    let before = std::fs::read_to_string(&launches).unwrap().lines().count();
    assert_eq!(before, 514);
    let cpu = child_cpu();
    let start = std::time::Instant::now();
    for (directory, r) in &records {
        supervisor
            .observe_current(directory, r, RuntimeCommand::Snapshot, None)
            .await
            .unwrap();
    }
    let hydration_cpu = child_cpu() - cpu;
    let hydration_elapsed = start.elapsed().as_secs_f64();
    let hydrated = std::fs::read_to_string(&launches).unwrap().lines().count();
    assert_eq!(hydrated - before, 257);
    let cpu = child_cpu();
    let start = std::time::Instant::now();
    // Four seconds of two clients probing the unchanged 257-entry catalogue.
    while start.elapsed() < Duration::from_secs(4) {
        for _ in 0..2 {
            for (directory, r) in &records {
                supervisor
                    .observe_current(directory, r, RuntimeCommand::Snapshot, None)
                    .await
                    .unwrap();
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let after_cpu = child_cpu() - cpu;
    let after_elapsed = start.elapsed().as_secs_f64();
    let after = std::fs::read_to_string(&launches).unwrap().lines().count() - hydrated;
    assert_eq!(after, 0);
    println!(
        "257-entry synthetic observer measurement: before=514 spawns/{before_elapsed:.3}s child_cpu={before_cpu:.6}s; hydrate=257 spawns/{hydration_elapsed:.3}s child_cpu={hydration_cpu:.6}s; unchanged two-client after={after} spawns/{after_elapsed:.3}s child_cpu={after_cpu:.6}s"
    );
}
