//! Retained #213 host tests. Synthetic keys only; HTTP fixtures bind numeric loopback.
//! No default-host registry, real login, inference, or process-global environment mutation.
use super::*;
use base64::Engine;
use device::DeviceService;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn registry() -> (tempfile::TempDir, Registry) {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::new(dir.path().join("accounts"));
    (dir, registry)
}
fn api_connection(registry: &Registry) -> ConnectionDescriptor {
    registry
        .add_connection(
            "Synthetic API".into(),
            "https://api.openai.com/v1".into(),
            vec![Transport::OpenaiResponses, Transport::OpenaiChat],
        )
        .unwrap()
}
fn key(value: &str) -> ApiKeyInput {
    ApiKeyInput::Stored(value.into())
}
fn identity_token(subject: &str) -> String {
    let claims = serde_json::json!({"sub":subject,
        "https://api.openai.com/auth":{"chatgpt_account_id":"synthetic-owner"}});
    format!(
        "e30.{}.synthetic-signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}
fn tokens(revision: &str) -> OAuthTokens {
    OAuthTokens {
        access_token: format!("synthetic-access-{revision}"),
        refresh_token: format!("synthetic-refresh-{revision}"),
        id_token: Some(identity_token("synthetic-subject")),
        expires_at: u64::MAX / 2,
        account_id: "synthetic-owner".into(),
    }
}

#[test]
fn same_provider_accounts_do_not_overwrite_and_bindings_freeze_identity() {
    let (_dir, r) = registry();
    let c = api_connection(&r);
    let first = r
        .add_api(c.id, "first".into(), "First".into(), key("synthetic-one"))
        .unwrap();
    let second = r
        .add_api(c.id, "second".into(), "Second".into(), key("synthetic-two"))
        .unwrap();
    assert_ne!(first.id, second.id);
    let a = r.freeze(first.id, Transport::OpenaiResponses).unwrap();
    let b = r.freeze(second.id, Transport::OpenaiChat).unwrap();
    let before = r.list(|_| true).unwrap();
    assert!(
        r.add_api(
            c.id,
            "first".into(),
            "Overwrite".into(),
            key("synthetic-bad")
        )
        .is_err()
    );
    assert_eq!(r.list(|_| true).unwrap(), before);
    assert_eq!(r.resolve_api_key(&a).unwrap(), "synthetic-one");
    assert_eq!(r.resolve_api_key(&b).unwrap(), "synthetic-two");
    r.rename(first.id, "renamed".into(), "Renamed".into())
        .unwrap();
    let renamed = r.validate_binding(&a).unwrap();
    assert_eq!(renamed.identity_generation, first.identity_generation);
    assert_eq!(renamed.credential_revision, first.credential_revision);
    assert_eq!(renamed.metadata_revision, first.metadata_revision + 1);
    let rotated = r.rotate_api(&a, key("synthetic-rotation"), true).unwrap();
    assert_eq!(rotated.identity_generation, a.identity_generation);
    assert_eq!(rotated.credential_revision, first.credential_revision + 1);
    assert_eq!(rotated.capability_revision, first.capability_revision + 1);
    assert_eq!(r.resolve_api_key(&a).unwrap(), "synthetic-rotation");
    let replacement = r
        .rotate_api(&a, key("synthetic-replacement"), false)
        .unwrap();
    assert_eq!(replacement.identity_generation, a.identity_generation + 1);
    assert!(r.resolve_api_key(&a).is_err());
    assert_eq!(r.resolve_api_key(&b).unwrap(), "synthetic-two");
    let fresh = r.freeze(first.id, Transport::OpenaiChat).unwrap();
    r.logout(first.id, true).unwrap();
    let removed = r.list(|d| d.id == first.id).unwrap();
    r.logout(first.id, false).unwrap();
    assert_eq!(r.list(|d| d.id == first.id).unwrap(), removed);
    assert!(r.validate_binding(&fresh).is_err());
    assert!(
        r.reauthenticate_api(
            first.id,
            replacement.identity_generation + 1,
            key("synthetic-resurrection"),
            true
        )
        .is_err()
    );
    assert_eq!(r.resolve_api_key(&b).unwrap(), "synthetic-two");
    let public = serde_json::to_string(&r.list(|d| d.id == second.id).unwrap()).unwrap();
    assert!(!public.contains("synthetic-two"));
    assert!(!public.contains("renamed"));
    let mut wrong = b.clone();
    wrong.connection_revision += 1;
    assert!(r.validate_binding(&wrong).is_err());
    wrong = b.clone();
    wrong.transport = Transport::Anthropic;
    assert!(r.validate_binding(&wrong).is_err());
}

#[test]
fn oauth_refresh_revision_fences_failure_recovery_and_logout() {
    let (_dir, r) = registry();
    let c = r.ensure_chatgpt_connection().unwrap();
    let original = tokens("one");
    let a = r
        .add_oauth(c.id, "oauth".into(), "OAuth".into(), original.clone())
        .unwrap();
    let b = r.freeze(a.id, Transport::ChatgptOauth).unwrap();
    let fence = r.refresh_begin(&b, &original).unwrap();
    assert!(
        r.oauth_load(&b)
            .err()
            .unwrap()
            .downcast_ref::<crate::provider::chatgpt_oauth::RefreshPending>()
            .is_some()
    );
    assert!(r.refresh_begin(&b, &original).is_err());
    assert_eq!(
        r.validate_binding(&b).unwrap().availability,
        CredentialAvailability::RefreshPendingOrUncertain
    );
    let rotated = tokens("two");
    assert!(r.oauth_save(&b, &rotated, Some(Uuid::new_v4())).is_err());
    r.oauth_save(&b, &rotated, Some(fence)).unwrap();
    assert!(r.oauth_load(&b).unwrap() == rotated);
    let observed = r.validate_binding(&b).unwrap();
    assert_eq!(observed.identity_generation, a.identity_generation);
    assert_eq!(observed.credential_revision, a.credential_revision + 1);
    assert_eq!(observed.capability_revision, a.capability_revision);
    assert!(r.refresh_begin(&b, &original).is_err());
    let failed = r.refresh_begin(&b, &rotated).unwrap();
    // A failed/lost exchange intentionally leaves its durable intent unresolved across reopen.
    let reopened = Registry::new(r.root.clone());
    assert!(reopened.oauth_load(&b).is_err());
    reopened
        .reauthenticate_oauth(a.id, b.identity_generation, original.clone(), false)
        .unwrap();
    assert!(r.oauth_save(&b, &rotated, Some(failed)).is_err());
    let late = r.refresh_begin(&b, &original).unwrap();
    r.logout(a.id, false).unwrap();
    assert!(r.oauth_save(&b, &rotated, Some(late)).is_err());
    assert!(r.oauth_load(&b).is_err());
    let descriptor = r.list(|_| true).unwrap().1.remove(0);
    assert_eq!(descriptor.state, AccountState::SignInRequired);
    assert_eq!(descriptor.identity_generation, b.identity_generation + 1);
}

// A separate test-binary process provides real environment snapshots without unsafe set_var.
const CHILD_MODE: &str = "VOYAGE_SYNTHETIC_ACCOUNT_CHILD";
const CHILD_ROOT: &str = "VOYAGE_SYNTHETIC_ACCOUNT_ROOT";
const ENV_KEY: &str = "VOYAGE_SYNTHETIC_ACCOUNT_KEY";
fn child(root: &std::path::Path, mode: &str, value: Option<&str>) -> std::process::Child {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "accounts::tests::account_child", "--nocapture"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .env(CHILD_MODE, mode)
        .env(CHILD_ROOT, root)
        .env_remove(ENV_KEY);
    if let Some(value) = value {
        command.env(ENV_KEY, value);
    }
    command.spawn().unwrap()
}
fn wait_child(mut child: std::process::Child) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("synthetic child timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
#[test]
fn account_child() {
    let Ok(mode) = std::env::var(CHILD_MODE) else {
        return;
    };
    let r = Registry::new(PathBuf::from(std::env::var_os(CHILD_ROOT).unwrap()));
    if mode == "enroll-env" {
        let c = api_connection(&r);
        r.add_api(
            c.id,
            "env".into(),
            "Environment".into(),
            ApiKeyInput::Environment(ENV_KEY.into()),
        )
        .unwrap();
    } else if mode == "changed-env" || mode == "missing-env" {
        let a = r.list(|_| true).unwrap().1.remove(0);
        let b = r.freeze(a.id, Transport::OpenaiResponses).unwrap();
        assert!(r.resolve_api_key(&b).is_err());
        assert_eq!(
            a.availability,
            if mode == "changed-env" {
                CredentialAvailability::EnvironmentChanged
            } else {
                CredentialAvailability::EnvironmentUnavailable
            }
        );
    } else if mode == "refresh" {
        let a = r.list(|_| true).unwrap().1.remove(0);
        let b = r.freeze(a.id, Transport::ChatgptOauth).unwrap();
        // Simultaneous processes race on one durable expected-token version. Only the winner
        // can publish; a loser may see busy/uncertain or the completed credential revision.
        if let Ok(fence) = r.refresh_begin(&b, &tokens("one")) {
            std::thread::sleep(std::time::Duration::from_millis(100));
            r.oauth_save(&b, &tokens("two"), Some(fence)).unwrap();
        }
    } else {
        panic!("unknown synthetic child mode");
    }
}
#[test]
fn environment_changes_and_absence_fail_closed_across_processes() {
    let (_dir, r) = registry();
    wait_child(child(&r.root, "enroll-env", Some("synthetic-original")));
    let before = r.list(|_| true).unwrap().0;
    wait_child(child(&r.root, "changed-env", Some("synthetic-changed")));
    wait_child(child(&r.root, "missing-env", None));
    assert_eq!(r.list(|_| true).unwrap().0, before);
}
#[test]
fn independent_process_refresh_fences_publish_only_one_revision() {
    let (_dir, r) = registry();
    let c = r.ensure_chatgpt_connection().unwrap();
    let a = r
        .add_oauth(c.id, "oauth".into(), "OAuth".into(), tokens("one"))
        .unwrap();
    let b = r.freeze(a.id, Transport::ChatgptOauth).unwrap();
    let first = child(&r.root, "refresh", None);
    let second = child(&r.root, "refresh", None);
    wait_child(first);
    wait_child(second);
    assert!(r.oauth_load(&b).unwrap() == tokens("two"));
    assert_eq!(
        r.validate_binding(&b).unwrap().credential_revision,
        a.credential_revision + 1
    );
}

#[cfg(unix)]
#[test]
fn private_registry_rejects_links_permissions_and_oversize_without_overwrite() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (dir, r) = registry();
    api_connection(&r);
    let path = r.root.join("registry.json");
    let original = std::fs::read(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
        0
    );
    assert_eq!(
        std::fs::metadata(&r.root).unwrap().permissions().mode() & 0o077,
        0
    );
    let backup = dir.path().join("original");
    std::fs::rename(&path, &backup).unwrap();
    symlink(&backup, &path).unwrap();
    assert!(r.list(|_| true).is_err());
    assert_eq!(std::fs::read(&backup).unwrap(), original);
    std::fs::remove_file(&path).unwrap();
    std::fs::hard_link(&backup, &path).unwrap();
    assert!(r.list(|_| true).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(r.list(|_| true).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&path, vec![b' '; LIMIT + 1]).unwrap();
    assert!(r.list(|_| true).is_err());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), (LIMIT + 1) as u64);
}

struct Step {
    path: &'static str,
    status: u16,
    body: serde_json::Value,
    raw_body: Option<String>,
    gate: Option<Arc<Gate>>,
}
#[derive(Default)]
struct Gate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
fn step(path: &'static str, status: u16, body: serde_json::Value) -> Step {
    Step {
        path,
        status,
        body,
        raw_body: None,
        gate: None,
    }
}
fn begin() -> Step {
    step(
        "/device",
        200,
        serde_json::json!({"device_auth_id":"synthetic-device-secret",
        "user_code":"SYNTHETIC-CODE", "interval":30,"expires_in":600}),
    )
}
fn grant() -> Step {
    step(
        "/poll",
        200,
        serde_json::json!({"authorization_code":"synthetic-grant", "code_verifier":"synthetic-verifier"}),
    )
}
fn exchange() -> Step {
    step(
        "/token",
        200,
        serde_json::json!({"access_token":"synthetic-access", "refresh_token":"synthetic-refresh",
        "account_id":"synthetic-owner", "id_token":identity_token("synthetic-subject"), "expires_in":3600}),
    )
}
struct HttpFixture {
    address: std::net::SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl HttpFixture {
    async fn new(steps: Vec<Step>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let task = tokio::spawn(async move {
            for step in steps {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    assert!(bytes.len() <= 16_384);
                    if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                assert!(length <= 8192);
                while bytes.len() < header_end + length {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                assert!(headers.starts_with(&format!("POST {} HTTP/1.1\r\n", step.path)));
                let body = String::from_utf8(bytes[header_end..].to_vec()).unwrap();
                if step.path == "/poll" {
                    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(json["device_auth_id"], "synthetic-device-secret");
                    assert_eq!(json["user_code"], "SYNTHETIC-CODE");
                } else if step.path == "/token" {
                    assert!(body.contains("grant_type=authorization_code"));
                    assert!(body.contains("code=synthetic-grant"));
                }
                captured.lock().unwrap().push(step.path.into());
                if let Some(gate) = step.gate {
                    gate.entered.notify_one();
                    gate.release.notified().await;
                }
                let body = step.raw_body.unwrap_or_else(|| step.body.to_string());
                let response = format!(
                    "HTTP/1.1 {} Synthetic\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    step.status,
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            // Keep listening after the script. An unexpected exchange must be observed,
            // not hidden by connection refusal (which a cancelled worker might discard).
            let (_socket, _) = listener.accept().await.unwrap();
            captured.lock().unwrap().push("unexpected-request".into());
            panic!("unexpected synthetic provider request");
        });
        Self {
            address,
            seen,
            task,
        }
    }
    async fn complete(&mut self, count: usize) {
        assert_eq!(self.seen.lock().unwrap().len(), count);
        self.task.abort();
        let result = (&mut self.task).await;
        assert!(
            result.unwrap_err().is_cancelled(),
            "synthetic provider task failed"
        );
    }
}
fn request(r: &Registry) -> EnrollmentRequest {
    EnrollmentRequest {
        command_id: Uuid::new_v4(),
        enrollment_id: Uuid::new_v4(),
        connection_id: r.ensure_chatgpt_connection().unwrap().id,
        alias: "device".into(),
        label: "Device".into(),
        actor: EnrollmentActor {
            principal: "synthetic-principal".into(),
            workspace: "synthetic-workspace".into(),
        },
    }
}
fn service(r: &Registry, fixture: &HttpFixture) -> DeviceService {
    DeviceService::new(r.clone(), Arc::new(|_, _| true)).loopback_fixture(fixture.address)
}
fn no_private_code(service: &DeviceService, req: &EnrollmentRequest) {
    let status = service.status(req.enrollment_id, &req.actor).unwrap();
    assert!(status.user_code.is_none());
    assert!(status.verification_uri.is_none());
    assert!(status.status.effects_may_have_occurred);
}

#[tokio::test]
async fn device_http_pending_slowdown_success_exact_ids_and_private_projection() {
    let (_dir, r) = registry();
    let mut http = HttpFixture::new(vec![
        begin(),
        step(
            "/poll",
            400,
            serde_json::json!({"error":"authorization_pending"}),
        ),
        step("/poll", 400, serde_json::json!({"error":"slow_down"})),
        grant(),
        exchange(),
    ])
    .await;
    let s = service(&r, &http);
    let req = request(&r);
    let pending = s.start(req.clone()).await.unwrap();
    assert_eq!(pending.state, EnrollmentState::Pending);
    assert_eq!(pending.enrollment_id, req.enrollment_id);
    assert_eq!(s.start(req.clone()).await.unwrap(), pending);
    let mut conflict = req.clone();
    conflict.label = "Changed".into();
    assert!(s.start(conflict).await.is_err());
    let mut conflict = req.clone();
    conflict.command_id = Uuid::new_v4();
    assert!(s.start(conflict).await.is_err());
    let mut wrong_actor = req.actor.clone();
    wrong_actor.principal = "unrelated".into();
    assert!(s.status(req.enrollment_id, &wrong_actor).is_err());
    let private = s.status(req.enrollment_id, &req.actor).unwrap();
    assert_eq!(private.user_code.as_deref(), Some("SYNTHETIC-CODE"));
    let public = serde_json::to_string(&pending).unwrap();
    for secret in [
        "SYNTHETIC-CODE",
        "synthetic-device-secret",
        "auth.openai.com",
    ] {
        assert!(!public.contains(secret));
    }
    assert!(r.list(|_| true).unwrap().1.is_empty());
    assert!(
        r.add_oauth(
            req.connection_id,
            req.alias.clone(),
            "Collision".into(),
            tokens("collision")
        )
        .is_err()
    );
    // Poll before the retained deadline is a no-op.
    assert_eq!(
        s.drive(req.enrollment_id, &req.actor).await.unwrap().state,
        EnrollmentState::Pending
    );
    assert_eq!(http.seen.lock().unwrap().len(), 1);
    for interval in [30, 30] {
        assert_eq!(s.fixture_due(req.enrollment_id, false).unwrap(), interval);
        assert_eq!(
            s.drive(req.enrollment_id, &req.actor).await.unwrap().state,
            EnrollmentState::Pending
        );
    }
    assert_eq!(s.fixture_due(req.enrollment_id, false).unwrap(), 35);
    let done = s.drive(req.enrollment_id, &req.actor).await.unwrap();
    assert_eq!(done.state, EnrollmentState::Succeeded);
    let account = done.account_id.unwrap();
    assert_eq!(
        r.enrollment_actor(account).unwrap(),
        Some(req.actor.clone())
    );
    assert_eq!(r.list(|_| true).unwrap().1.len(), 1);
    no_private_code(&s, &req);
    assert_eq!(s.start(req.clone()).await.unwrap(), done);
    assert_eq!(
        s.cancel(Uuid::new_v4(), req.enrollment_id, &req.actor)
            .unwrap(),
        done
    );
    assert_eq!(s.drive(req.enrollment_id, &req.actor).await.unwrap(), done);
    http.complete(5).await;
}

#[tokio::test]
async fn device_http_denied_expired_and_uncertain_are_terminal_without_replay() {
    for (error, status, expected) in [
        ("access_denied", 400, EnrollmentState::Denied),
        ("access_denied", 403, EnrollmentState::Denied),
        ("expired_token", 400, EnrollmentState::Expired),
        ("expired_token", 404, EnrollmentState::Expired),
        ("unexpected", 400, EnrollmentState::Uncertain),
    ] {
        let (_dir, r) = registry();
        let mut http = HttpFixture::new(vec![
            begin(),
            step("/poll", status, serde_json::json!({"error":error})),
        ])
        .await;
        let s = service(&r, &http);
        let req = request(&r);
        s.start(req.clone()).await.unwrap();
        s.fixture_due(req.enrollment_id, false).unwrap();
        let terminal = s.drive(req.enrollment_id, &req.actor).await.unwrap();
        assert_eq!(terminal.state, expected);
        no_private_code(&s, &req);
        assert_eq!(s.start(req.clone()).await.unwrap(), terminal);
        assert_eq!(
            s.drive(req.enrollment_id, &req.actor).await.unwrap(),
            terminal
        );
        assert!(r.list(|_| true).unwrap().1.is_empty());
        http.complete(2).await;
    }
}

#[tokio::test]
async fn device_expiry_cancel_and_authority_revocation_prevent_polling() {
    for mode in ["expiry", "cancel", "revoke"] {
        let (_dir, r) = registry();
        let mut http = HttpFixture::new(vec![begin()]).await;
        let allowed = Arc::new(AtomicBool::new(true));
        let current = allowed.clone();
        let s = DeviceService::new(
            r.clone(),
            Arc::new(move |_, _| current.load(Ordering::SeqCst)),
        )
        .loopback_fixture(http.address);
        let req = request(&r);
        s.start(req.clone()).await.unwrap();
        s.fixture_due(req.enrollment_id, mode == "expiry").unwrap();
        if mode == "cancel" {
            let id = Uuid::new_v4();
            let cancelled = s.cancel(id, req.enrollment_id, &req.actor).unwrap();
            assert_eq!(
                s.cancel(id, req.enrollment_id, &req.actor).unwrap(),
                cancelled
            );
            assert!(
                s.cancel(req.command_id, req.enrollment_id, &req.actor)
                    .is_err()
            );
        }
        if mode == "revoke" {
            allowed.store(false, Ordering::SeqCst);
            assert!(s.status(req.enrollment_id, &req.actor).is_err());
        }
        let status = s.drive(req.enrollment_id, &req.actor).await.unwrap();
        assert_eq!(
            status.state,
            if mode == "expiry" {
                EnrollmentState::Expired
            } else {
                EnrollmentState::Cancelled
            }
        );
        assert!(r.list(|_| true).unwrap().1.is_empty());
        allowed.store(true, Ordering::SeqCst);
        no_private_code(&s, &req);
        http.complete(1).await;
    }
}

#[tokio::test]
async fn device_late_exchange_cannot_publish_after_cancel_or_revocation() {
    for revoke in [false, true] {
        let (_dir, r) = registry();
        let gate = Arc::new(Gate::default());
        let mut response = exchange();
        response.gate = Some(gate.clone());
        let mut http = HttpFixture::new(vec![begin(), grant(), response]).await;
        let allowed = Arc::new(AtomicBool::new(true));
        let current = allowed.clone();
        let s = DeviceService::new(
            r.clone(),
            Arc::new(move |_, _| current.load(Ordering::SeqCst)),
        )
        .loopback_fixture(http.address);
        let req = request(&r);
        s.start(req.clone()).await.unwrap();
        s.fixture_due(req.enrollment_id, false).unwrap();
        let worker_service = s.clone();
        let worker_req = req.clone();
        let worker = tokio::spawn(async move {
            worker_service
                .drive(worker_req.enrollment_id, &worker_req.actor)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), gate.entered.notified())
            .await
            .unwrap();
        if revoke {
            allowed.store(false, Ordering::SeqCst);
        } else {
            s.cancel(Uuid::new_v4(), req.enrollment_id, &req.actor)
                .unwrap();
        }
        gate.release.notify_one();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.state, EnrollmentState::Cancelled);
        assert!(result.account_id.is_none());
        assert!(result.effects_may_have_occurred);
        assert!(r.list(|_| true).unwrap().1.is_empty());
        allowed.store(true, Ordering::SeqCst);
        no_private_code(&s, &req);
        http.complete(3).await;
    }
}

#[tokio::test]
async fn device_lost_exchange_and_crashed_worker_are_not_reissued() {
    let (_dir, r) = registry();
    let gate = Arc::new(Gate::default());
    let mut response = exchange();
    response.gate = Some(gate.clone());
    let http = HttpFixture::new(vec![begin(), grant(), response]).await;
    let s = service(&r, &http);
    let req = request(&r);
    s.start(req.clone()).await.unwrap();
    s.fixture_due(req.enrollment_id, false).unwrap();
    let worker_service = s.clone();
    let worker_req = req.clone();
    let worker = tokio::spawn(async move {
        worker_service
            .drive(worker_req.enrollment_id, &worker_req.actor)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    let recovered = service(&Registry::new(r.root.clone()), &http);
    assert_eq!(
        recovered.start(req.clone()).await.unwrap().state,
        EnrollmentState::Exchanging
    );
    assert_eq!(
        recovered
            .drive(req.enrollment_id, &req.actor)
            .await
            .unwrap()
            .state,
        EnrollmentState::Exchanging
    );
    recovered.fixture_due(req.enrollment_id, true).unwrap();
    assert_eq!(
        recovered
            .drive(req.enrollment_id, &req.actor)
            .await
            .unwrap()
            .state,
        EnrollmentState::Uncertain
    );
    no_private_code(&recovered, &req);
    assert!(recovered.resume_candidates().unwrap().is_empty());
    assert!(r.list(|_| true).unwrap().1.is_empty());
    assert_eq!(http.seen.lock().unwrap().len(), 3);
    // Fixture and outstanding response are dropped; upstream effect outcome remains unknown.
}

#[test]
fn oauth_subject_change_and_legacy_identity_require_explicit_replacement() {
    let (_dir, r) = registry();
    let c = r.ensure_chatgpt_connection().unwrap();
    let original = tokens("one");
    let a = r
        .add_oauth(c.id, "identity".into(), "Identity".into(), original.clone())
        .unwrap();
    let binding = r.freeze(a.id, Transport::ChatgptOauth).unwrap();
    let mut different = tokens("two");
    different.id_token = Some(identity_token("different-subject"));
    assert!(
        r.reauthenticate_oauth(a.id, a.identity_generation, different.clone(), false)
            .is_err()
    );
    let fence = r.refresh_begin(&binding, &original).unwrap();
    assert!(r.oauth_save(&binding, &different, Some(fence)).is_err());
    let replacement = r
        .reauthenticate_oauth(a.id, a.identity_generation, different, true)
        .unwrap();
    assert_eq!(replacement.identity_generation, a.identity_generation + 1);
    assert!(r.oauth_load(&binding).is_err());
    let mut legacy = tokens("legacy");
    legacy.id_token = None;
    let a = r
        .add_oauth(c.id, "legacy".into(), "Legacy".into(), legacy.clone())
        .unwrap();
    let binding = r.freeze(a.id, Transport::ChatgptOauth).unwrap();
    let fence = r.refresh_begin(&binding, &legacy).unwrap();
    assert!(r.oauth_save(&binding, &legacy, Some(fence)).is_err());
    assert!(
        r.reauthenticate_oauth(a.id, a.identity_generation, legacy.clone(), false)
            .is_err()
    );
    let replacement = r
        .reauthenticate_oauth(a.id, a.identity_generation, legacy, true)
        .unwrap();
    assert_eq!(replacement.identity_generation, a.identity_generation + 1);
}

#[tokio::test]
async fn device_command_ids_are_disjoint_across_all_roles_and_exact_retries() {
    let (_dir, r) = registry();
    let mut http = HttpFixture::new(vec![begin(), begin()]).await;
    let s = service(&r, &http);
    let first = request(&r);
    for (command, enrollment) in [
        (Uuid::nil(), Uuid::new_v4()),
        (Uuid::new_v4(), Uuid::nil()),
        (first.command_id, first.command_id),
    ] {
        let mut invalid = first.clone();
        invalid.command_id = command;
        invalid.enrollment_id = enrollment;
        assert!(s.start(invalid).await.is_err());
    }
    s.start(first.clone()).await.unwrap();
    let cancelled_id = Uuid::new_v4();
    for invalid in [Uuid::nil(), first.command_id, first.enrollment_id] {
        assert!(
            s.cancel(invalid, first.enrollment_id, &first.actor)
                .is_err()
        );
    }
    let cancelled = s
        .cancel(cancelled_id, first.enrollment_id, &first.actor)
        .unwrap();
    assert_eq!(
        s.cancel(cancelled_id, first.enrollment_id, &first.actor)
            .unwrap(),
        cancelled
    );
    assert_eq!(s.start(first.clone()).await.unwrap(), cancelled);
    // Every existing role is unavailable to both roles of a new start.
    for reserved in [first.command_id, first.enrollment_id, cancelled_id] {
        for command_role in [false, true] {
            let mut conflict = request(&r);
            conflict.alias = "conflict".into();
            if command_role {
                conflict.command_id = reserved;
            } else {
                conflict.enrollment_id = reserved;
            }
            assert!(s.start(conflict).await.is_err());
        }
    }
    let mut second = request(&r);
    second.alias = "second".into();
    s.start(second.clone()).await.unwrap();
    assert!(
        s.cancel(cancelled_id, second.enrollment_id, &second.actor)
            .is_err()
    );
    assert!(
        s.cancel(second.enrollment_id, first.enrollment_id, &first.actor)
            .is_err()
    );
    assert!(
        s.cancel(second.command_id, first.enrollment_id, &first.actor)
            .is_err()
    );
    assert_eq!(
        s.status(second.enrollment_id, &second.actor)
            .unwrap()
            .status
            .state,
        EnrollmentState::Pending
    );
    http.complete(2).await;
}

#[tokio::test]
async fn device_cancel_revoke_or_expire_during_poll_prevents_token_exchange() {
    for mode in ["cancel", "revoke", "expire"] {
        let (_dir, r) = registry();
        let gate = Arc::new(Gate::default());
        let mut poll = grant();
        poll.gate = Some(gate.clone());
        // No exchange response is available: reaching that endpoint must never happen.
        let mut http = HttpFixture::new(vec![begin(), poll]).await;
        let allowed = Arc::new(AtomicBool::new(true));
        let current = allowed.clone();
        let s = DeviceService::new(
            r.clone(),
            Arc::new(move |_, _| current.load(Ordering::SeqCst)),
        )
        .loopback_fixture(http.address);
        let req = request(&r);
        s.start(req.clone()).await.unwrap();
        s.fixture_due(req.enrollment_id, false).unwrap();
        let worker_service = s.clone();
        let worker_req = req.clone();
        let worker = tokio::spawn(async move {
            worker_service
                .drive(worker_req.enrollment_id, &worker_req.actor)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), gate.entered.notified())
            .await
            .unwrap();
        match mode {
            "cancel" => {
                s.cancel(Uuid::new_v4(), req.enrollment_id, &req.actor)
                    .unwrap();
            }
            "revoke" => {
                allowed.store(false, Ordering::SeqCst);
            }
            "expire" => {
                s.fixture_due(req.enrollment_id, true).unwrap();
            }
            _ => unreachable!(),
        }
        gate.release.notify_one();
        let terminal = tokio::time::timeout(std::time::Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            terminal.state,
            if mode == "expire" {
                EnrollmentState::Expired
            } else {
                EnrollmentState::Cancelled
            }
        );
        assert!(terminal.account_id.is_none());
        assert!(r.list(|_| true).unwrap().1.is_empty());
        http.complete(2).await;
    }
}

#[tokio::test]
async fn device_invalid_verification_url_and_failed_exchange_remain_uncertain() {
    for invalid_begin in [false, true] {
        let (_dir, r) = registry();
        let steps = if invalid_begin {
            let mut response = begin();
            response.body["verification_uri"] =
                serde_json::json!("https://untrusted.invalid/device");
            vec![response]
        } else {
            // The exchange occurred but its token response cannot establish a login identity.
            vec![
                begin(),
                grant(),
                step(
                    "/token",
                    200,
                    serde_json::json!({"access_token":"synthetic-incomplete", "expires_in":3600}),
                ),
            ]
        };
        let expected_requests = steps.len();
        let mut http = HttpFixture::new(steps).await;
        let s = service(&r, &http);
        let req = request(&r);
        let start = s.start(req.clone()).await.unwrap();
        let terminal = if invalid_begin {
            start
        } else {
            s.fixture_due(req.enrollment_id, false).unwrap();
            s.drive(req.enrollment_id, &req.actor).await.unwrap()
        };
        assert_eq!(terminal.state, EnrollmentState::Uncertain);
        no_private_code(&s, &req);
        assert_eq!(s.start(req.clone()).await.unwrap(), terminal);
        assert_eq!(
            s.drive(req.enrollment_id, &req.actor).await.unwrap(),
            terminal
        );
        assert!(r.list(|_| true).unwrap().1.is_empty());
        http.complete(expected_requests).await;
    }
}

#[test]
fn private_registry_lock_contention_is_bounded_and_failed_mutation_is_atomic() {
    let (_dir, r) = registry();
    api_connection(&r);
    let before = std::fs::read(r.root.join("registry.json")).unwrap();
    let result: Result<()> = r.transaction(|db| {
        db.connections.clear();
        bail!("synthetic failed mutation")
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(r.root.join("registry.json")).unwrap(), before);
    let directory = Directory::open(&r.root).unwrap();
    let held = directory.lock().unwrap();
    let start = std::time::Instant::now();
    assert!(r.list(|_| true).is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    drop(held);
    assert_eq!(r.list(|_| true).unwrap().1.len(), 0);
    assert_eq!(std::fs::read(r.root.join("registry.json")).unwrap(), before);
}

#[tokio::test]
async fn resolve_enrollment_fences_absent_start_and_never_replays_provider_effects() {
    let (_dir, r) = registry();
    let mut fixture = HttpFixture::new(vec![]).await;
    let s = service(&r, &fixture);
    let req = request(&r);
    let fenced = s.resolve(req.clone()).unwrap();
    assert_eq!(fenced.state, EnrollmentState::Cancelled);
    assert!(!fenced.effects_may_have_occurred);
    assert_eq!(s.start(req.clone()).await.unwrap(), fenced);
    let mut conflict = req.clone();
    conflict.alias = "different".into();
    assert!(s.resolve(conflict).is_err());
    let status = s.status(req.enrollment_id, &req.actor).unwrap();
    assert!(status.user_code.is_none());
    assert_eq!(status.status, fenced);
    fixture.complete(0).await;
}

#[tokio::test]
async fn device_pending_http_status_keeps_code_until_actual_approval() {
    let (_dir, r) = registry();
    let mut plain = step("/poll", 403, serde_json::Value::Null);
    plain.raw_body = Some("Authorization pending".into());
    let mut empty = step("/poll", 404, serde_json::Value::Null);
    empty.raw_body = Some(String::new());
    let mut http = HttpFixture::new(vec![
        begin(),
        plain,
        empty,
        step(
            "/poll",
            403,
            serde_json::json!({"error":"device_authorization_pending"}),
        ),
        grant(),
        exchange(),
    ])
    .await;
    let s = service(&r, &http);
    let req = request(&r);
    s.start(req.clone()).await.unwrap();
    for _ in 0..3 {
        s.fixture_due(req.enrollment_id, false).unwrap();
        assert_eq!(
            s.drive(req.enrollment_id, &req.actor).await.unwrap().state,
            EnrollmentState::Pending
        );
        let private = s.status(req.enrollment_id, &req.actor).unwrap();
        assert_eq!(private.user_code.as_deref(), Some("SYNTHETIC-CODE"));
        assert!(private.failure.is_none());
    }
    s.fixture_due(req.enrollment_id, false).unwrap();
    assert_eq!(
        s.drive(req.enrollment_id, &req.actor).await.unwrap().state,
        EnrollmentState::Succeeded
    );
    http.complete(6).await;
}

#[tokio::test]
async fn device_diagnostics_identify_phase_without_body_secrets_or_effect_replay() {
    for (phase, steps) in [
        (
            EnrollmentPhase::RequestCode,
            vec![step(
                "/device",
                502,
                serde_json::json!({"secret":"DO-NOT-PUBLISH"}),
            )],
        ),
        (
            EnrollmentPhase::Poll,
            vec![
                begin(),
                step("/poll", 502, serde_json::json!({"secret":"DO-NOT-PUBLISH"})),
            ],
        ),
        (
            EnrollmentPhase::Exchange,
            vec![
                begin(),
                grant(),
                step(
                    "/token",
                    502,
                    serde_json::json!({"secret":"DO-NOT-PUBLISH"}),
                ),
            ],
        ),
    ] {
        let (_dir, r) = registry();
        let count = steps.len();
        let mut http = HttpFixture::new(steps).await;
        let s = service(&r, &http);
        let req = request(&r);
        s.start(req.clone()).await.unwrap();
        s.fixture_due(req.enrollment_id, false).unwrap();
        let state = s.drive(req.enrollment_id, &req.actor).await.unwrap();
        assert_eq!(state.state, EnrollmentState::Uncertain);
        let private = s.status(req.enrollment_id, &req.actor).unwrap();
        let diagnostic = private.failure.as_ref().unwrap();
        assert_eq!(diagnostic.phase, phase);
        assert_eq!(diagnostic.http_status, Some(502));
        let restored = service(&Registry::new(r.root.clone()), &http);
        assert_eq!(
            restored
                .status(req.enrollment_id, &req.actor)
                .unwrap()
                .failure,
            private.failure
        );
        let mut legacy = serde_json::to_value(&private).unwrap();
        legacy.as_object_mut().unwrap().remove("failure");
        assert!(
            serde_json::from_value::<PrivateEnrollmentStatus>(legacy)
                .unwrap()
                .failure
                .is_none()
        );
        assert!(private.user_code.is_none());
        let encoded = serde_json::to_string(&private).unwrap();
        assert!(!encoded.contains("DO-NOT-PUBLISH"));
        assert!(!encoded.contains("synthetic"));
        assert_eq!(s.drive(req.enrollment_id, &req.actor).await.unwrap(), state);
        assert_eq!(s.start(req.clone()).await.unwrap(), state);
        assert_eq!(
            s.cancel(Uuid::new_v4(), req.enrollment_id, &req.actor)
                .unwrap()
                .state,
            EnrollmentState::Cancelled
        );
        // Closing the uncertain attempt frees the alias for a genuinely new intent.
        let mut next = req.clone();
        next.command_id = Uuid::new_v4();
        next.enrollment_id = Uuid::new_v4();
        assert_eq!(s.resolve(next).unwrap().state, EnrollmentState::Cancelled);
        http.complete(count).await;
    }
}

#[tokio::test]
async fn device_code_visible_during_poll_but_hidden_before_token_exchange() {
    let (_dir, r) = registry();
    let poll_gate = Arc::new(Gate::default());
    let exchange_gate = Arc::new(Gate::default());
    let mut poll = grant();
    poll.gate = Some(poll_gate.clone());
    let mut token = exchange();
    token.gate = Some(exchange_gate.clone());
    let mut http = HttpFixture::new(vec![begin(), poll, token]).await;
    let s = service(&r, &http);
    let req = request(&r);
    s.start(req.clone()).await.unwrap();
    s.fixture_due(req.enrollment_id, false).unwrap();
    let driver = s.clone();
    let request = req.clone();
    let work = tokio::spawn(async move {
        driver
            .drive(request.enrollment_id, &request.actor)
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(5), poll_gate.entered.notified())
        .await
        .unwrap();
    let private = s.status(req.enrollment_id, &req.actor).unwrap();
    assert_eq!(private.status.state, EnrollmentState::Exchanging);
    assert_eq!(private.user_code.as_deref(), Some("SYNTHETIC-CODE"));
    poll_gate.release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), exchange_gate.entered.notified())
        .await
        .unwrap();
    no_private_code(&s, &req);
    exchange_gate.release.notify_one();
    assert_eq!(work.await.unwrap().state, EnrollmentState::Succeeded);
    http.complete(3).await;
}
