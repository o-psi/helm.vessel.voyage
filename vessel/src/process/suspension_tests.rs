use super::super::{database, test_support::Fixture};
use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn observer_rejects_mutations_and_missing_or_unlaunchable_binaries() {
    let f = Fixture::new();
    let mut r = f.registration();
    assert!(
        observe(&f.0, &r, RuntimeCommand::PrepareBrowser, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("requires execution owner")
    );
    assert!(
        observe(&f.0, &r, RuntimeCommand::Snapshot, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("binary missing")
    );
    r.executable = Some(f.0.join("absent"));
    assert!(
        observe(&f.0, &r, RuntimeCommand::Snapshot, None)
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
        let result = observe(&f.0, &r, RuntimeCommand::Snapshot, Some(grant)).await;
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
