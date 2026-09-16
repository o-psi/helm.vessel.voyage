//! Actual coordinator admission and executable/IPC boundary, with no host accounts.
use super::{database, registry, test_support::Fixture};
use serde_json::Value;
use std::{os::unix::fs::PermissionsExt, path::Path};
use uuid::Uuid;
use voyage_protocol::process::*;

// Only the external child is controlled; production admission, persistence,
// spawning, authenticated framing, catalogue and resolution remain intact.
const CHILD: &str = r#"#!/usr/bin/python3
import json, os, socket, struct, sys, time
args = sys.argv[1:]
assert args[0] == 'supervise', args
root = args[args.index('--directory') + 1]
with open(os.path.join(root, 'registration.json')) as f:
    reg = json.load(f)
assert args[args.index('--session') + 1] == reg['session_id']
assert args[args.index('--incarnation') + 1] == reg['incarnation']
assert args[args.index('--workspace') + 1] == reg['workspace']
with open(os.path.join(root, 'fixture-argv.json'), 'w') as f:
    json.dump(args, f)
sock = socket.socket(socket.AF_UNIX)
sock.bind(os.path.join(root, 'runtime.sock'))
sock.listen(8)
sock.settimeout(1)
def exact(c, n):
    data = b''
    while len(data) < n:
        part = c.recv(n - len(data))
        if not part:
            raise EOFError()
        data += part
    return data
deadline = time.monotonic() + 20
while time.monotonic() < deadline and not os.path.exists(os.path.join(root, 'fixture-stop')):
    try:
        c, _ = sock.accept()
    except socket.timeout:
        continue
    with c:
        c.settimeout(3)
        req = json.loads(exact(c, struct.unpack('!I', exact(c, 4))[0]))
        assert req['protocol'] == reg['protocol']
        assert req.get('authorization') is None
        assert req['token'] == reg['token']
        assert req['session_id'] == reg['session_id']
        assert req['incarnation'] == reg['incarnation']
        with open(os.path.join(root, 'fixture-requests.jsonl'), 'a') as f:
            f.write(json.dumps(req['command']) + '\n')
        command = req['command']
        assert command['op'] in ('health', 'snapshot'), command
        # The child only exposes private health IPC; catalogue reads stay local.
        encoded = json.dumps(command).lower()
        error = None
        if 'snapshot' in encoded:
            error = 'fixture has no snapshot'
        response = dict(protocol=reg['protocol'], session_id=reg['session_id'],
                        incarnation=reg['incarnation'],
                        result={}, error=error, outcome_unknown=False)
        body = json.dumps(response).encode()
        c.sendall(struct.pack('!I', len(body)) + body)
sock.close()
with open(os.path.join(root, 'fixture-stopped'), 'w') as f:
    f.write('done')
"#;

fn executable(f: &Fixture) -> std::path::PathBuf {
    let path = f.0.join("controlled-voyage");
    std::fs::write(&path, CHILD).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

// Always stops via a private marker, never by a potentially reused numeric PID.
// The child also has a bounded lifetime if a test panics.
async fn stop(directory: &Path) {
    std::fs::write(directory.join("fixture-stop"), b"").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !directory.join("fixture-stopped").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

fn initialization(f: &Fixture) -> RuntimeInitialization {
    RuntimeInitialization::ManagedImport {
        transfer_id: Uuid::new_v4(),
        source_directory: f.0.join("input"),
        expected_revision: 7,
    }
}
fn original(f: &Fixture, command_id: Uuid, session_id: Uuid) -> VesselCommand {
    VesselCommand::Start {
        command_id,
        session_id,
        workspace: f.0.clone(),
    }
}

#[tokio::test]
async fn initialized_coordinator_spawns_authenticates_persists_and_replays() {
    for configured in [false, true] {
        let f = Fixture::new();
        let mut s = f.supervisor().await;
        s.binary = executable(&f);
        let session = Uuid::new_v4();
        let command = Uuid::new_v4();
        let init = initialization(&f);
        let config = configured.then(|| f.0.join("launch.json"));
        if let Some(path) = &config {
            std::fs::write(path, "{}").unwrap();
        }
        let request = if let Some(path) = &config {
            VesselCommand::StartConfigured {
                command_id: command,
                session_id: session,
                workspace: f.0.clone(),
                config_path: path.clone(),
            }
        } else {
            original(&f, command, session)
        };
        let response = s
            .start_initialized(
                command,
                session,
                f.0.clone(),
                config.clone(),
                Some(init.clone()),
                request.clone(),
            )
            .await
            .unwrap();
        assert_eq!(response["state"], "live");
        assert_eq!(response["session_id"], session.to_string());
        let directory = registry::directory(&f.0, session);
        let registration = database::registration(&f.0, session).await.unwrap();
        assert_eq!(registration.initialize, Some(init.clone()));
        assert_eq!(registration.config_path, config);
        assert_eq!(registration.executable.as_ref(), Some(&s.binary));
        assert_eq!(registration.workspace, f.0);
        assert_eq!(registration.token.len(), 64);
        let published = registry::load(&directory.join("registration.json")).unwrap();
        assert_eq!(published.incarnation, registration.incarnation);
        let receipt = database::creation_receipt(&f.0, command)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.state, ProcessState::Live);
        let replay = s
            .start_initialized(
                command,
                session,
                f.0.clone(),
                config.clone(),
                Some(init),
                request,
            )
            .await
            .unwrap();
        assert_eq!(response, replay);
        assert_eq!(
            database::registration(&f.0, session)
                .await
                .unwrap()
                .incarnation,
            registration.incarnation
        );
        let resolved = s
            .resolve_start(command, session, f.0.clone(), config.clone())
            .await
            .unwrap();
        assert_eq!(resolved["status"], "created");
        let args: Vec<String> =
            serde_json::from_slice(&std::fs::read(directory.join("fixture-argv.json")).unwrap())
                .unwrap();
        assert_eq!(args[0], "supervise");
        assert!(args.contains(&session.to_string()));
        assert!(args.contains(&registration.incarnation.to_string()));
        assert_eq!(args.contains(&"--config".to_owned()), configured);
        if let Some(path) = config {
            let index = args.iter().position(|v| v == "--config").unwrap();
            assert_eq!(Path::new(&args[index + 1]), path);
        }
        let log = std::fs::read_to_string(directory.join("fixture-requests.jsonl")).unwrap();
        let requests: Vec<Value> = log
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(requests.iter().any(|v| v["op"] == "health"));
        assert!(requests.iter().all(|v| v["op"] == "health"));
        stop(&directory).await;
    }
}

#[tokio::test]
async fn initialized_spawn_failure_is_retained_and_never_retried() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let session = Uuid::new_v4();
    let command = Uuid::new_v4();
    let request = original(&f, command, session);
    let init = initialization(&f);
    let error = s
        .start_initialized(
            command,
            session,
            f.0.clone(),
            None,
            Some(init.clone()),
            request.clone(),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<super::routing::OutcomeUnknown>()
            .is_some()
    );
    let registration = database::registration(&f.0, session).await.unwrap();
    assert_eq!(registration.state, ProcessState::Starting);
    assert!(
        database::creation_receipt(&f.0, command)
            .await
            .unwrap()
            .is_none()
    );
    let response = s
        .start_initialized(command, session, f.0.clone(), None, Some(init), request)
        .await
        .unwrap();
    assert_eq!(response["state"], "unavailable");
    assert_eq!(
        database::registration(&f.0, session)
            .await
            .unwrap()
            .incarnation,
        registration.incarnation
    );
    let resolved = s
        .resolve_start(command, session, f.0.clone(), None)
        .await
        .unwrap();
    assert_eq!(resolved["status"], "unknown");
    assert!(
        !registry::directory(&f.0, session)
            .join("runtime.sock")
            .exists()
    );
}

#[tokio::test]
async fn negative_resolution_survives_reconstruction_and_blocks_initialized_launch() {
    let f = Fixture::new();
    let session = Uuid::new_v4();
    let command = Uuid::new_v4();
    let s = f.supervisor().await;
    for _ in 0..2 {
        assert_eq!(
            s.resolve_start(command, session, f.0.clone(), None)
                .await
                .unwrap()["status"],
            "not_admitted"
        );
    }
    drop(s);
    let s = f.supervisor().await;
    let error = s
        .start_initialized(
            command,
            session,
            f.0.clone(),
            None,
            Some(initialization(&f)),
            original(&f, command, session),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("fenced"));
    assert!(s.registrations.lock().await.unwrap().is_empty());
    assert!(!registry::directory(&f.0, session).exists());
    assert!(
        s.resolve_start(command, Uuid::new_v4(), f.0.clone(), None)
            .await
            .is_err()
    );
    assert!(
        registry::command_record(&f.0, command, &original(&f, command, session), false)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn configured_resolution_binds_path_and_envelope_before_any_spawn() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let session = Uuid::new_v4();
    let command = Uuid::new_v4();
    let config = f.0.join("config.json");
    std::fs::write(&config, "{}").unwrap();
    assert_eq!(
        s.resolve_start(command, session, f.0.clone(), Some(config.clone()))
            .await
            .unwrap()["status"],
        "not_admitted"
    );
    assert!(
        s.resolve_start(command, session, f.0.clone(), None)
            .await
            .is_err()
    );
    let other = f.0.join("other.json");
    std::fs::write(&other, "{}").unwrap();
    assert!(
        s.resolve_start(command, session, f.0.clone(), Some(other))
            .await
            .is_err()
    );
    assert!(
        s.start(command, session, f.0.clone(), Some(config))
            .await
            .is_err()
    );
    assert!(s.registrations.lock().await.unwrap().is_empty());
}

#[tokio::test]
async fn legacy_registration_resolution_never_invents_an_admission() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut r = f.registration();
    database::save(&f.0, &r).await.unwrap();
    assert_eq!(
        s.resolve_start(r.command_id, r.session_id, r.workspace.clone(), None)
            .await
            .unwrap()["status"],
        "unknown"
    );
    assert!(
        !registry::command_record(
            &f.0,
            r.command_id,
            &original(&f, r.command_id, r.session_id),
            false
        )
        .await
        .unwrap()
    );
    assert!(
        s.resolve_start(r.command_id, Uuid::new_v4(), r.workspace.clone(), None)
            .await
            .is_err()
    );
    r.initialize = Some(initialization(&f));
    database::save(&f.0, &r).await.unwrap();
    assert!(
        s.resolve_start(r.command_id, r.session_id, r.workspace, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn start_rejects_invalid_identity_config_and_workspace_before_host_accounts() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    for (command, session) in [(Uuid::nil(), Uuid::new_v4()), (Uuid::new_v4(), Uuid::nil())] {
        assert!(s.start(command, session, f.0.clone(), None).await.is_err());
        assert!(
            s.resolve_start(command, session, f.0.clone(), None)
                .await
                .is_err()
        );
    }
    for path in [
        std::path::PathBuf::from("relative"),
        f.0.clone(),
        f.0.join("missing"),
    ] {
        assert!(
            s.start(
                Uuid::new_v4(),
                Uuid::new_v4(),
                f.0.clone(),
                Some(path.clone())
            )
            .await
            .is_err()
        );
        // Resolution preserves a lexical config envelope, even if the file is gone.
        assert_eq!(
            s.resolve_start(Uuid::new_v4(), Uuid::new_v4(), f.0.clone(), Some(path))
                .await
                .unwrap()["status"],
            "not_admitted"
        );
    }
    let file = f.0.join("not-a-directory");
    std::fs::write(&file, b"").unwrap();
    for workspace in [file, f.0.join("missing")] {
        assert!(
            s.start(Uuid::new_v4(), Uuid::new_v4(), workspace, None)
                .await
                .is_err()
        );
    }
    assert!(s.registrations.lock().await.unwrap().is_empty());
}

#[tokio::test]
async fn retained_admission_without_registration_is_unknown_not_a_new_launch() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let session = Uuid::new_v4();
    let command = Uuid::new_v4();
    let request = original(&f, command, session);
    registry::command_record(&f.0, command, &request, true)
        .await
        .unwrap();
    let error = s
        .start_initialized(
            command,
            session,
            f.0.clone(),
            None,
            Some(initialization(&f)),
            request,
        )
        .await
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<super::routing::OutcomeUnknown>()
            .is_some()
    );
    assert_eq!(
        s.resolve_start(command, session, f.0.clone(), None)
            .await
            .unwrap()["status"],
        "unknown"
    );
    assert!(s.registrations.lock().await.unwrap().is_empty());
}

#[tokio::test]
async fn existing_session_admits_new_command_without_relaunch_and_checks_provenance() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut r = f.registration();
    r.initialize = Some(initialization(&f));
    database::save(&f.0, &r).await.unwrap();
    let command = Uuid::new_v4();
    let request = original(&f, command, r.session_id);
    let result = s
        .start_initialized(
            command,
            r.session_id,
            f.0.clone(),
            None,
            r.initialize.clone(),
            request.clone(),
        )
        .await
        .unwrap();
    assert_eq!(result["session_id"], r.session_id.to_string());
    assert_eq!(result["incarnation"], r.incarnation.to_string());
    assert!(
        registry::command_record(&f.0, command, &request, false)
            .await
            .unwrap()
    );
    assert!(
        database::creation_receipt(&f.0, command)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!registry::directory(&f.0, r.session_id).exists());
    // Same session with a new command still cannot replace initialization or config.
    let changed = Uuid::new_v4();
    assert!(
        s.start_initialized(
            changed,
            r.session_id,
            f.0.clone(),
            None,
            Some(initialization(&f)),
            original(&f, changed, r.session_id)
        )
        .await
        .is_err()
    );
    assert!(
        !registry::command_record(&f.0, changed, &original(&f, changed, r.session_id), false)
            .await
            .unwrap()
    );
    // Legacy command reuse must match the original session, even without a receipt.
    assert!(
        s.start_initialized(
            r.command_id,
            Uuid::new_v4(),
            f.0.clone(),
            None,
            r.initialize.clone(),
            original(&f, r.command_id, Uuid::new_v4())
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn admitted_uninitialized_registration_resolution_requires_canonical_workspace() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let r = f.registration();
    let request = original(&f, r.command_id, r.session_id);
    database::admit(&f.0, &r, serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    assert_eq!(
        s.resolve_start(r.command_id, r.session_id, f.0.clone(), None)
            .await
            .unwrap()["status"],
        "created"
    );
    // A durable request does not make a mismatching registration usable.
    let mut changed = r.clone();
    changed.workspace = f.0.join("elsewhere");
    database::save(&f.0, &changed).await.unwrap();
    assert_eq!(
        s.resolve_start(r.command_id, r.session_id, f.0.clone(), None)
            .await
            .unwrap()["status"],
        "unknown"
    );
    changed.workspace = f.0.clone();
    changed.initialize = Some(initialization(&f));
    database::save(&f.0, &changed).await.unwrap();
    assert_eq!(
        s.resolve_start(r.command_id, r.session_id, f.0.clone(), None)
            .await
            .unwrap()["status"],
        "unknown"
    );
    assert!(
        database::creation_receipt(&f.0, r.command_id)
            .await
            .unwrap()
            .is_none()
    );
}
