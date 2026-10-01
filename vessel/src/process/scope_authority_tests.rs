//! Pure admission and owned local framing. Successful root epoch/UID publication
//! remains a separate disposable native fixture; tests never bypass that gate.
use super::super::test_support::Fixture;
use super::*;

fn request(root: &Path) -> ScopeCheck {
    ScopeCheck {
        schema: 1,
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        token: "t".repeat(64),
        handle: ExecutionScopeHandle {
            socket_name: socket_name(root),
            lease_id: Uuid::new_v4(),
            secret: "s".repeat(64),
        },
    }
}

#[test]
fn socket_identity_is_stable_private_and_separates_control_realms() {
    let first = socket_name(Path::new("/synthetic/private-realm-one"));
    assert_eq!(
        first,
        socket_name(Path::new("/synthetic/private-realm-one"))
    );
    assert_ne!(
        first,
        socket_name(Path::new("/synthetic/private-realm-two"))
    );
    assert!(first.starts_with("voyage-scope-"));
    assert!(!first.contains("private-realm"));
}

#[test]
fn scope_grant_rechecks_exact_current_binding_rights_and_revocation() {
    let fixture = Fixture::new();
    let registration = fixture.registration();
    let mut grant = fixture.session();
    grant.session_id = registration.session_id;
    fixture.save_session(&grant);
    let binding = GrantBinding {
        grant_id: grant.grant_id,
        principal_id: grant.principal_id,
        revision: grant.revision,
    };
    let current = current_grant(&fixture.0, &registration, &binding).unwrap();
    assert_eq!(current.accounts, grant.accounts);
    assert_eq!(current.rights, grant.rights);
    for field in 0..3 {
        let mut changed = binding.clone();
        match field {
            0 => changed.grant_id = Uuid::new_v4(),
            1 => changed.principal_id = Uuid::new_v4(),
            _ => changed.revision += 1,
        };
        assert!(current_grant(&fixture.0, &registration, &changed).is_err());
    }
    let mut changed_registration = registration.clone();
    changed_registration.session_id = Uuid::new_v4();
    assert!(current_grant(&fixture.0, &changed_registration, &binding).is_err());
    changed_registration = registration.clone();
    changed_registration.workspace = fixture.0.join("other");
    assert!(current_grant(&fixture.0, &changed_registration, &binding).is_err());
    let pristine = grant.clone();
    for field in 0..3 {
        grant = pristine.clone();
        match field {
            0 => grant.revoked = true,
            1 => grant.expires_at_ms = 0,
            _ => grant.rights.clear(),
        };
        fixture.save_session(&grant);
        assert!(current_grant(&fixture.0, &registration, &binding).is_err());
    }
}

#[tokio::test]
async fn unprotected_runtime_cannot_mint_root_authority_or_create_lease_state() {
    let fixture = Fixture::new();
    let registration = fixture.registration();
    let grant = fixture.session();
    let binding = GrantBinding {
        grant_id: grant.grant_id,
        revision: grant.revision,
        principal_id: grant.principal_id,
    };
    assert!(mint(&fixture.0, &registration, &binding).await.is_err());
    assert!(!fixture.0.join("access/runtime-scope-leases").exists());
    assert!(start(&fixture.0).unwrap().is_none());
}

#[tokio::test]
async fn malformed_scope_identity_is_refused_before_private_catalogue_access() {
    let fixture = Fixture::new();
    for field in 0..6 {
        let mut value = request(&fixture.0);
        match field {
            0 => value.schema = 0,
            1 => value.session_id = Uuid::nil(),
            2 => value.incarnation = Uuid::nil(),
            3 => value.token.clear(),
            4 => value.handle.secret.clear(),
            _ => value.handle.socket_name = socket_name(Path::new("/different-root")),
        };
        assert!(
            scope(&fixture.0, unsafe { libc::geteuid() }, value)
                .await
                .is_err()
        );
        assert!(!fixture.0.join("catalogue.sqlite3").exists());
        assert!(!fixture.0.join("access/runtime-scope-leases").exists());
    }
}

async fn refusal(length: u32, body: &[u8]) {
    let fixture = Fixture::new();
    let (mut client, server) = UnixStream::pair().unwrap();
    let directory = fixture.0.clone();
    let worker =
        tokio::spawn(
            async move { connection(directory, server, unsafe { libc::geteuid() }).await },
        );
    client.write_u32(length).await.unwrap();
    client.write_all(body).await.unwrap();
    client.shutdown().await.unwrap();
    let bytes = tokio::time::timeout(Duration::from_secs(3), async {
        let length = client.read_u32().await.unwrap() as usize;
        assert!(length <= MAX_FRAME);
        let mut bytes = vec![0; length];
        client.read_exact(&mut bytes).await.unwrap();
        bytes
    })
    .await
    .unwrap();
    assert_eq!(bytes, br#"{"scope":null}"#);
    assert!(
        serde_json::from_slice::<ScopeReply>(&bytes)
            .unwrap()
            .scope
            .is_none()
    );
    worker.await.unwrap().unwrap();
    assert!(!fixture.0.join("access/runtime-scope-leases").exists());
}

#[tokio::test]
async fn size_json_and_truncation_failures_return_only_fixed_private_refusal() {
    refusal(0, b"").await;
    refusal((MAX_FRAME + 1) as u32, b"").await;
    refusal(8, b"{}").await;
    let invalid = br#"{"secret":"synthetic-private-credential","unrecognized":true}"#;
    refusal(invalid.len() as u32, invalid).await;
    let invalid_utf8 = [0xff];
    refusal(1, &invalid_utf8).await;
}

#[tokio::test]
async fn stalled_scope_peer_is_bounded_and_exposes_no_diagnostics() {
    let fixture = Fixture::new();
    let (mut client, server) = UnixStream::pair().unwrap();
    let directory = fixture.0.clone();
    let started = tokio::time::Instant::now();
    let worker =
        tokio::spawn(
            async move { connection(directory, server, unsafe { libc::geteuid() }).await },
        );
    let length = tokio::time::timeout(DEADLINE + Duration::from_secs(1), client.read_u32())
        .await
        .unwrap()
        .unwrap() as usize;
    let mut bytes = vec![0; length];
    client.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, br#"{"scope":null}"#);
    assert!(started.elapsed() >= DEADLINE);
    worker.await.unwrap().unwrap();
}

#[tokio::test]
async fn service_terminal_outcomes_are_failures_and_drop_observes_owned_future_cleanup() {
    let mut stopped = Service {
        task: tokio::spawn(async { Ok(()) }),
    };
    assert!(
        stopped
            .wait()
            .await
            .unwrap_err()
            .to_string()
            .contains("service stopped")
    );
    let mut failed = Service {
        task: tokio::spawn(async { anyhow::bail!("synthetic local service failure") }),
    };
    assert!(failed.wait().await.is_err());
    let (ready, started) = tokio::sync::oneshot::channel();
    let (finished, cleaned) = tokio::sync::oneshot::channel();
    struct OwnedFuture(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for OwnedFuture {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let owned = Service {
        task: tokio::spawn(async move {
            let _guard = OwnedFuture(Some(finished));
            let _ = ready.send(());
            std::future::pending::<Result<()>>().await
        }),
    };
    started.await.unwrap();
    drop(owned);
    tokio::time::timeout(Duration::from_secs(1), cleaned)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn absent_optional_service_does_not_manufacture_success_or_spin() {
    let mut service = None;
    assert!(
        tokio::time::timeout(Duration::from_millis(20), wait(&mut service))
            .await
            .is_err()
    );
    assert!(service.is_none());
}

#[test]
fn guardian_socket_separates_session_incarnation_and_control_realm() {
    let root = Path::new("/synthetic/private-realm-one");
    let session = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let name = guardian_socket_name(root, session, incarnation);
    assert_eq!(name, guardian_socket_name(root, session, incarnation));
    assert_ne!(name, socket_name(root));
    assert_ne!(
        name,
        guardian_socket_name(root, Uuid::new_v4(), incarnation)
    );
    assert_ne!(name, guardian_socket_name(root, session, Uuid::new_v4()));
    assert_ne!(
        name,
        guardian_socket_name(Path::new("/other"), session, incarnation)
    );
    assert!(name.len() <= 80);
    assert!(!name.contains("private-realm"));
}

#[tokio::test]
async fn guardian_listener_refuses_another_owner_without_loading_authority() {
    let fixture = Fixture::new();
    let mut value = request(&fixture.0);
    value.handle.socket_name =
        guardian_socket_name(&fixture.0, value.session_id, value.incarnation);
    let owner = (Uuid::new_v4(), value.incarnation);
    let (mut client, server) = UnixStream::pair().unwrap();
    let directory = fixture.0.clone();
    let worker = tokio::spawn(async move {
        connection_for(directory, server, unsafe { libc::geteuid() }, Some(owner)).await
    });
    let body = serde_json::to_vec(&value).unwrap();
    client.write_u32(body.len() as u32).await.unwrap();
    client.write_all(&body).await.unwrap();
    let length = client.read_u32().await.unwrap() as usize;
    let mut bytes = vec![0; length];
    client.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, br#"{"scope":null}"#);
    worker.await.unwrap().unwrap();
    assert!(!fixture.0.join("access/runtime-scope-leases").exists());
}

#[test]
fn unprotected_guardian_start_never_manufactures_a_root_listener() {
    let fixture = Fixture::new();
    assert!(start_guardian(&fixture.0, &fixture.registration()).is_err());
}
