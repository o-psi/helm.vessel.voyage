//! Source-only hostile-peer/frame/cache cases; no root broker is simulated as
//! authoritative. The native peer-negative case requires ordinary Linux UID.
use super::*;
use uuid::Uuid;
use voyage_protocol::process::{ProcessPeerUids, ProcessRight, ProcessState};
fn registration() -> ProcessRegistration {
    ProcessRegistration {
        executable: None,
        protocol: 1,
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        restart_from: None,
        initialize: None,
        config_path: None,
        token: "PRIVATE-runtime-token".into(),
        peer_uids: Some(ProcessPeerUids {
            supervisor: 0,
            runtime: unsafe { libc::geteuid() },
        }),
        workspace: "/fixture/workspace".into(),
        state: ProcessState::Live,
        name: None,
    }
}
fn binding() -> GrantBinding {
    GrantBinding {
        grant_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        revision: 1,
    }
}
fn handle() -> ExecutionScopeHandle {
    ExecutionScopeHandle {
        socket_name: format!("voyage-scope-fixture-{}", Uuid::new_v4()),
        lease_id: Uuid::new_v4(),
        secret: "PRIVATE-scope-secret-xxxxxxxxxxxxxxxx".into(),
    }
}
fn scope(reg: &ProcessRegistration, b: &GrantBinding) -> RuntimeScope {
    RuntimeScope {
        schema: SCHEMA,
        session_id: reg.session_id,
        incarnation: reg.incarnation,
        binding: b.clone(),
        full_access: false,
        rights: vec![ProcessRight::Observe, ProcessRight::Execute],
        accounts: vec![],
        enrollment_connections: vec![],
        expires_at_ms: 1000,
        workspace: reg.workspace.clone(),
        connection_binding: None,
    }
}
#[test]
fn metadata_is_exact_epoch_binding_workspace_and_expiry_without_any_token_hash() {
    let reg = registration();
    let b = binding();
    let original = scope(&reg, &b);
    let grant = metadata_to_grant(&reg, &b, original.clone(), 999).unwrap();
    assert!(grant.token_hash.is_empty());
    assert!(grant.parent_grant.is_none());
    for field in 0..10 {
        let mut changed = original.clone();
        match field {
            0 => changed.schema += 1,
            1 => changed.session_id = Uuid::new_v4(),
            2 => changed.incarnation = Uuid::new_v4(),
            3 => changed.binding.revision += 1,
            4 => changed.binding.principal_id = Uuid::new_v4(),
            5 => changed.workspace = "/other".into(),
            6 => changed.expires_at_ms = 999,
            7 => changed.full_access = true,
            8 => changed.accounts = vec![Uuid::new_v4(); 257],
            _ => changed.rights = vec![ProcessRight::Execute; 65],
        }
        assert!(metadata_to_grant(&reg, &b, changed, 999).is_err());
    }
    let mut owner = original;
    owner.full_access = true;
    owner.connection_binding = Some(b.clone());
    assert!(metadata_to_grant(&reg, &b, owner, 999).is_ok());
}
#[test]
fn ordinary_abstract_socket_peer_is_rejected_before_registration_or_lease_secret() {
    use std::{
        io::Read,
        os::linux::net::SocketAddrExt,
        os::unix::net::{SocketAddr, UnixListener},
        time::Duration,
    };
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "this negative kernel-peer fixture requires ordinary UID"
    );
    let reg = registration();
    let b = binding();
    let h = handle();
    let address = SocketAddr::from_abstract_name(h.socket_name.as_bytes()).unwrap();
    let listener = UnixListener::bind_addr(&address).unwrap();
    let observer = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let error = current(&reg, &h, &b).unwrap_err();
    assert_eq!(error.to_string(), "execution scope unavailable");
    assert!(!format!("{error:?}").contains("PRIVATE"));
    assert!(observer.join().unwrap().is_empty());
}
#[test]
fn malformed_truncated_and_oversized_frames_never_construct_scope_metadata() {
    use std::{
        io::Write,
        os::unix::net::UnixStream,
        time::{Duration, Instant},
    };
    for (length, body) in [
        (0, b"".as_slice()),
        ((MAX_FRAME + 1) as u32, b"".as_slice()),
        (3, b"{".as_slice()),
        (3, b"bad".as_slice()),
    ] {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        writer.write_all(&length.to_be_bytes()).unwrap();
        writer.write_all(body).unwrap();
        drop(writer);
        assert!(linux::response(&mut reader, Instant::now() + Duration::from_secs(1)).is_err());
    }
    let (mut reader, writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let before = Instant::now();
    assert!(linux::response(&mut reader, before + Duration::from_millis(20)).is_err());
    assert!(before.elapsed() < Duration::from_secs(1));
    drop(writer);
}
#[test]
fn credential_cache_is_private_exact_binding_and_not_debug_or_history_data() {
    use std::os::unix::fs::MetadataExt;
    let root = tempfile::tempdir().unwrap();
    let b = binding();
    let h = handle();
    cache_validated(root.path(), &b, &h).unwrap();
    assert_eq!(
        std::fs::metadata(root.path().join("scope-credentials"))
            .unwrap()
            .mode()
            & 0o7777,
        0o700
    );
    let path = root
        .path()
        .join("scope-credentials")
        .join(cache_name(&b).unwrap());
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o600);
    assert_eq!(std::fs::metadata(&path).unwrap().nlink(), 1);
    assert_eq!(cached_handle(root.path(), &b).unwrap().secret, h.secret);
    assert!(!format!("{h:?}").contains("PRIVATE"));
    let mut changed = b.clone();
    changed.revision += 1;
    assert!(cached_handle(root.path(), &changed).is_err());
    assert!(!root.path().join("journal").exists());
}
#[test]
fn cache_rejects_readable_hardlinked_symlink_and_changed_binding_records() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let b = binding();
    let h = handle();
    cache_validated(root.path(), &b, &h).unwrap();
    let path = root
        .path()
        .join("scope-credentials")
        .join(cache_name(&b).unwrap());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(cached_handle(root.path(), &b).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let hard = root.path().join("hard");
    std::fs::hard_link(&path, &hard).unwrap();
    assert!(cached_handle(root.path(), &b).is_err());
    std::fs::remove_file(&hard).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    symlink(&hard, &path).unwrap();
    assert!(cached_handle(root.path(), &b).is_err());
    std::fs::remove_file(&path).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["binding"]["revision"] = 2.into();
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(cached_handle(root.path(), &b).is_err());
}
#[test]
fn cache_capacity_is_bounded_and_known_binding_can_rotate_opaque_lease() {
    let root = tempfile::tempdir().unwrap();
    let mut first = None;
    for _ in 0..64 {
        let b = binding();
        cache_validated(root.path(), &b, &handle()).unwrap();
        first.get_or_insert(b);
    }
    assert!(cache_validated(root.path(), &binding(), &handle()).is_err());
    let b = first.unwrap();
    let replacement = handle();
    cache_validated(root.path(), &b, &replacement).unwrap();
    assert_eq!(
        cached_handle(root.path(), &b).unwrap().lease_id,
        replacement.lease_id
    );
}

#[test]
fn private_goal_lease_survives_clean_registration_change_but_old_metadata_never_does() {
    let root = tempfile::tempdir().unwrap();
    let old = registration();
    let b = binding();
    let h = handle();
    cache_validated(root.path(), &b, &h).unwrap();
    let mut restarted = old.clone();
    restarted.incarnation = Uuid::new_v4();
    restarted.token = "new-PRIVATE-runtime-token".into();
    let cached = cached_handle(root.path(), &b).unwrap();
    assert_eq!(cached.lease_id, h.lease_id);
    assert!(metadata_to_grant(&restarted, &b, scope(&old, &b), 999).is_err());
    assert!(metadata_to_grant(&restarted, &b, scope(&restarted, &b), 999).is_ok());
    // This is only credential/metadata mechanics: execution epoch approval is
    // checked by the real root broker, never inferred from a readable cache.
    assert!(current(&restarted, &cached, &b).is_err());
}
