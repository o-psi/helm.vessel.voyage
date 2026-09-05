use super::*;
use std::{
    io::Write,
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
struct Directory {
    _root: tempfile::TempDir,
    path: std::path::PathBuf,
}
impl Directory {
    fn path(&self) -> &Path {
        &self.path
    }
}
fn directory() -> Directory {
    let root = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    let path = {
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        root.path().to_owned()
    };
    #[cfg(windows)]
    let path = {
        let path = root.path().join("private");
        voyage_storage::PrivateDirectory::open(&path).unwrap();
        path
    };
    Directory { _root: root, path }
}
fn open(dir: &Path) -> EnrollmentClient {
    EnrollmentClient::open(dir, "https://vessel.example", false).unwrap()
}
fn challenge(client: &EnrollmentClient, operation: ProofOperation) -> Challenge {
    Challenge {
        version: 2,
        id: Uuid::new_v4(),
        origin: client.origin().into(),
        expires_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            + 50_000,
        server_tag: vec![7; 32],
        operation,
    }
}
fn active(client: &mut EnrollmentClient) {
    client.state.status = Status::Active;
    client.state.epoch = 1;
    client.state.owner_id = Some(Uuid::new_v4());
    client.persist().unwrap();
}
fn pending_rotation(client: &mut EnrollmentClient) -> ProofOperation {
    let key = SigningKey::generate().unwrap();
    let operation = ProofOperation::Rotate {
        machine_id: client.machine_id(),
        epoch: client.epoch(),
        transaction_id: Uuid::new_v4(),
        new_public_key: key.public_key(),
    };
    client.state.pending = Some(Pending {
        operation: operation.clone(),
        new_private_key: Some(key.as_pkcs8().to_vec()),
    });
    client.state.status = Status::Rotating;
    client.persist().unwrap();
    operation
}
#[test]
fn strict_origin_policy() {
    for value in ["http://127.0.0.1", "http://[::1]:9000"] {
        assert!(validate_origin(value, true).is_ok());
        assert_eq!(validate_origin(value, false), Err(ClientError::Origin));
    }
    for value in [
        "http://localhost",
        "http://127.1",
        "http://2130706433",
        "http://0x7f000001",
        "http://10.0.0.1",
        "https://u:p@example.com",
        "https://@example.com",
        "https://example.com/path",
        "https://example.com?x=secret",
        "https://example.com#secret",
        " https://example.com",
        "https://example.com\n",
        "wss://example.com",
        "https://example.com\\",
    ] {
        assert_eq!(
            validate_origin(value, true),
            Err(ClientError::Origin),
            "{value}"
        );
    }
    assert_eq!(
        validate_origin("https://EXAMPLE.com:443/", false).unwrap(),
        "https://example.com"
    );
}
#[test]
fn private_persistent_identity_and_process_lock() {
    let root = directory();
    let path = root.path().join("enrollment");
    let client = open(&path);
    let id = client.machine_id();
    let key = client.key().unwrap().public_key();
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(path.join("client.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(matches!(
        EnrollmentClient::open(&path, client.origin(), false),
        Err(ClientError::Busy)
    ));
    drop(client);
    let client = open(&path);
    assert_eq!(client.machine_id(), id);
    assert_eq!(client.key().unwrap().public_key(), key);
    drop(client);
    assert!(matches!(
        EnrollmentClient::open(&path, "https://different.example", false),
        Err(ClientError::Conflict)
    ));
}
#[test]
#[cfg(unix)]
fn unsafe_files_and_directories_fail_closed() {
    let root = directory();
    let path = root.path().join("enrollment");
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        EnrollmentClient::open(&path, "https://vessel.example", false),
        Err(ClientError::Storage)
    ));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(root.path().join("missing"), path.join("client.json")).unwrap();
    assert!(matches!(
        EnrollmentClient::open(&path, "https://vessel.example", false),
        Err(ClientError::Storage)
    ));
}
#[test]
fn validates_challenge_context_before_signing() {
    let root = directory();
    let mut client = open(root.path());
    active(&mut client);
    let operation = ProofOperation::Connect {
        machine_id: client.machine_id(),
        epoch: 1,
    };
    let good = challenge(&client, operation.clone());
    let proof = client
        .sign_challenge(good.clone(), &operation, None)
        .unwrap();
    proof
        .verify(
            &client.key().unwrap().public_key(),
            client.origin(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        )
        .unwrap();
    let mut wrong = good.clone();
    wrong.origin = "https://evil.example".into();
    assert!(client.sign_challenge(wrong, &operation, None).is_err());
    let mut wrong = good.clone();
    wrong.expires_at_ms = 1;
    assert!(client.sign_challenge(wrong, &operation, None).is_err());
    let mut wrong = good.clone();
    wrong.operation = ProofOperation::Connect {
        machine_id: client.machine_id(),
        epoch: 2,
    };
    assert!(client.sign_challenge(wrong, &operation, None).is_err());
    let mut wrong = good;
    wrong.server_tag.clear();
    assert!(client.sign_challenge(wrong, &operation, None).is_err());
}
#[test]
fn rotation_crash_retains_both_keys_and_transaction_until_valid_receipt() {
    let root = directory();
    let mut client = open(root.path());
    active(&mut client);
    let old = client.key().unwrap().public_key();
    let op = pending_rotation(&mut client);
    drop(client);
    let mut client = open(root.path());
    assert_eq!(client.status(), Status::Rotating);
    assert_eq!(client.state.pending.as_ref().unwrap().operation, op);
    assert_eq!(client.key().unwrap().public_key(), old);
    let next = client
        .state
        .pending
        .as_ref()
        .unwrap()
        .new_private_key
        .as_deref()
        .unwrap();
    let public = SigningKey::from_pkcs8(next).unwrap().public_key();
    let proof = client
        .sign_challenge(challenge(&client, op.clone()), &op, Some(next))
        .unwrap();
    proof
        .verify(
            &old,
            client.origin(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        )
        .unwrap();
    let mut receipt = Receipt {
        machine_id: client.machine_id(),
        owner_id: client.owner_id().unwrap(),
        epoch: 3,
        revoked: false,
    };
    assert_eq!(client.accept_receipt(&receipt), Err(ClientError::Response));
    receipt.epoch = 2;
    client.accept_receipt(&receipt).unwrap();
    drop(client);
    let client = open(root.path());
    assert_eq!(client.epoch(), 2);
    assert_eq!(client.key().unwrap().public_key(), public);
    assert!(client.state.pending.is_none());
}
#[test]
fn detach_preserves_sessions_and_identity_and_blocks_connect() {
    let root = directory();
    let mut client = open(root.path());
    active(&mut client);
    let id = client.machine_id();
    fs::write(root.path().join("session-fixture"), "user work").unwrap();
    client.detach().unwrap();
    assert_eq!(client.active(), Err(ClientError::Conflict));
    drop(client);
    let client = open(root.path());
    assert_eq!(client.status(), Status::Detached);
    assert_eq!(client.machine_id(), id);
    assert_eq!(
        fs::read_to_string(root.path().join("session-fixture")).unwrap(),
        "user work"
    );
}
#[test]
fn corrupted_state_and_owner_switch_rejected() {
    let root = directory();
    let mut client = open(root.path());
    active(&mut client);
    pending_rotation(&mut client);
    assert_eq!(client.detach(), Err(ClientError::Conflict));
    let receipt = Receipt {
        machine_id: client.machine_id(),
        owner_id: Uuid::new_v4(),
        epoch: 2,
        revoked: false,
    };
    assert_eq!(client.accept_receipt(&receipt), Err(ClientError::Response));
    client.state.pending.as_mut().unwrap().new_private_key = Some(vec![1]);
    client.persist().unwrap();
    drop(client);
    assert!(matches!(
        EnrollmentClient::open(root.path(), "https://vessel.example", false),
        Err(ClientError::Storage)
    ));
}

// Strictly offline loopback fixture. Uses only generated test keys/fake invitation
// values; no external server or credentials. Each response closes its connection.
type FixtureResponse = Box<dyn FnOnce(&str, serde_json::Value) -> Option<String> + Send>;

fn fixture(responses: Vec<FixtureResponse>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        for respond in responses {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let end;
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") {
                    end = bytes.len();
                    break;
                }
            }
            let headers = String::from_utf8(bytes.clone()).unwrap();
            assert!(headers.contains("x-voyage-request: 2"));
            assert!(headers.contains("content-type: application/json"));
            let len: usize = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            bytes.resize(end + len, 0);
            socket.read_exact(&mut bytes[end..]).unwrap();
            let body = serde_json::from_slice(&bytes[end..]).unwrap();
            if let Some(response) = respond(&headers, body) {
                socket.write_all(response.as_bytes()).unwrap();
            }
        }
    });
    (origin, handle)
}
fn ok(value: serde_json::Value) -> String {
    let body = value.to_string();
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}
fn challenge_response(headers: &str, body: serde_json::Value) -> Option<String> {
    assert!(headers.starts_with("POST /v2/enrollment/challenge "));
    assert_eq!(body.as_object().unwrap().len(), 2);
    let origin = headers
        .lines()
        .find_map(|l| l.strip_prefix("origin: "))
        .unwrap();
    Some(ok(
        serde_json::json!({"version":2,"id":Uuid::new_v4(),"origin":origin,"expires_at_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64 + 50_000,"server_tag":vec![7;32],"operation":body["operation"]}),
    ))
}
#[tokio::test]
async fn websocket_connect_only_fetches_challenge_never_completes_http() {
    let (origin, server) = fixture(vec![Box::new(challenge_response)]);
    let root = directory();
    let mut client = EnrollmentClient::open(root.path(), &origin, true).unwrap();
    active(&mut client);
    let proof = client.connect_proof().await.unwrap();
    assert!(matches!(
        proof.challenge.operation,
        ProofOperation::Connect { epoch: 1, .. }
    ));
    server.join().unwrap();
}
#[tokio::test]
async fn lost_enrollment_response_recovers_original_transaction_after_reopen() {
    let seen = Arc::new(Mutex::new(None));
    let first = seen.clone();
    let last = seen.clone();
    let root = directory();
    let persisted = root.path().join("client.json");
    let (origin, server) = fixture(vec![
        Box::new(move |headers, body| {
            let state: State = serde_json::from_slice(&fs::read(&persisted).unwrap()).unwrap();
            assert_eq!(state.status, Status::Enrolling);
            assert_eq!(
                serde_json::to_value(&state.pending.unwrap().operation).unwrap(),
                body["operation"]
            );
            assert!(!state.private_key.is_empty());
            challenge_response(headers, body)
        }),
        Box::new(move |headers, body| {
            assert!(headers.starts_with("POST /v2/enrollment/complete "));
            *first.lock().unwrap() = Some(body["proof"]["challenge"]["operation"].clone());
            None
        }),
        Box::new(challenge_response),
        Box::new(move |_, body| {
            let op = &body["proof"]["challenge"]["operation"];
            assert_eq!(Some(op), last.lock().unwrap().as_ref());
            Some(ok(
                serde_json::json!({"machine_id":op["machine_id"],"owner_id":Uuid::new_v4(),"epoch":1,"revoked":false}),
            ))
        }),
    ]);
    let mut client = EnrollmentClient::open(root.path(), &origin, true).unwrap();
    assert_eq!(
        client.enroll(Uuid::new_v4(), &"x".repeat(43)).await,
        Err(ClientError::Network)
    );
    assert_eq!(client.status(), Status::Enrolling);
    drop(client);
    let mut client = EnrollmentClient::open(root.path(), &origin, true).unwrap();
    client.resume(None).await.unwrap();
    assert_eq!(client.status(), Status::Active);
    assert_eq!(
        client.enroll(Uuid::new_v4(), &"x".repeat(43)).await,
        Err(ClientError::Conflict)
    );
    server.join().unwrap();
}
#[tokio::test]
async fn redirects_oversize_and_secret_errors_are_not_exposed() {
    for response in [
        "HTTP/1.1 302 Found\r\nlocation: https://secret.invalid/token\r\ncontent-length: 0\r\n\r\n"
            .to_string(),
        "HTTP/1.1 200 OK\r\ncontent-length: 999999\r\n\r\n".to_string(),
        ok(serde_json::json!({"secret":"do-not-echo"})),
    ] {
        let (origin, server) = fixture(vec![Box::new(move |_, _| Some(response))]);
        let root = directory();
        let mut client = EnrollmentClient::open(root.path(), &origin, true).unwrap();
        active(&mut client);
        let error = client.connect_proof().await.unwrap_err();
        assert!(!format!("{error:?} {error}").contains("secret"));
        assert!(matches!(error, ClientError::Denied | ClientError::Response));
        server.join().unwrap();
    }
}
#[tokio::test]
async fn revoke_persists_tombstone_and_retains_user_data() {
    let owner = Uuid::new_v4();
    let (origin, server) = fixture(vec![
        Box::new(challenge_response),
        Box::new(move |_, body| {
            let op = &body["proof"]["challenge"]["operation"];
            assert_eq!(op["type"], "revoke");
            Some(ok(
                serde_json::json!({"machine_id":op["machine_id"],"owner_id":owner,"epoch":2,"revoked":true}),
            ))
        }),
    ]);
    let root = directory();
    let mut client = EnrollmentClient::open(root.path(), &origin, true).unwrap();
    active(&mut client);
    client.state.owner_id = Some(owner);
    client.persist().unwrap();
    fs::write(root.path().join("sessions"), "untouched").unwrap();
    client.revoke().await.unwrap();
    drop(client);
    let client = EnrollmentClient::open(root.path(), &origin, true).unwrap();
    assert_eq!(client.status(), Status::Revoked);
    assert_eq!(client.epoch(), 2);
    assert!(client.connect_proof().await.is_err());
    assert_eq!(
        fs::read_to_string(root.path().join("sessions")).unwrap(),
        "untouched"
    );
    server.join().unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn root_owned_macos_temp_alias_allows_enrollment_but_not_user_directory_aliases() {
    for temp_root in ["/var/tmp", "/tmp"] {
        let root = tempfile::tempdir_in(temp_root).unwrap();
        let directory = root.path().join("client");
        let client = EnrollmentClient::open(&directory, "https://example.com", false).unwrap();
        drop(client);
        EnrollmentClient::open(&directory, "https://example.com", false).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&directory, &alias).unwrap();
        assert!(EnrollmentClient::open(&alias, "https://example.com", false).is_err());
    }
}

#[test]
#[cfg(unix)]
fn enrollment_rejects_user_symlink_ancestors_and_directory_aliases() {
    let root = directory();
    let actual = root.path().join("actual");
    fs::create_dir(&actual).unwrap();
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    assert!(
        EnrollmentClient::open(&alias.join("client"), "https://vessel.example", false).is_err()
    );
    assert!(!actual.join("client").exists());
    assert!(EnrollmentClient::open(&alias, "https://vessel.example", false).is_err());
}

#[test]
#[cfg(windows)]
fn native_replacement_failure_retains_identity_and_poisons_the_client() {
    let root = directory();
    let mut client = open(root.path());
    let original = fs::read(root.path().join("client.json")).unwrap();
    let id = client.machine_id();
    let held = client
        .private_directory
        .open_file("client.json", false)
        .unwrap();
    client.state.status = Status::Detached;
    assert_eq!(client.persist().unwrap_err(), ClientError::Storage);
    assert_eq!(client.ready().unwrap_err(), ClientError::Storage);
    assert_eq!(fs::read(root.path().join("client.json")).unwrap(), original);
    drop(held);
    drop(client);
    let reopened = open(root.path());
    assert_eq!(reopened.machine_id(), id);
    assert_eq!(reopened.status(), Status::Unenrolled);
}
