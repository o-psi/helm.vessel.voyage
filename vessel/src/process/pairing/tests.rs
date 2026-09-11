use super::*;
use std::{cell::Cell, os::unix::fs::PermissionsExt};
thread_local! { static FAIL: Cell<&'static str> = const { Cell::new("") }; }
pub(super) fn failpoint(stage: &'static str) -> Result<()> {
    if FAIL.with(|f| {
        if f.get() == stage {
            f.set("");
            true
        } else {
            false
        }
    }) {
        anyhow::bail!("synthetic interrupted publication");
    }
    Ok(())
}
fn request(invitation: &Invitation, command: Uuid) -> PairRequest {
    PairRequest {
        protocol: 1,
        command_id: command,
        principal_id: invitation.principal_id,
        invitation_id: invitation.invitation_id,
        code: invitation.code.clone(),
    }
}
fn count(root: &Path) -> usize {
    connection_audit(root, 128, None).unwrap()["events"]
        .as_array()
        .unwrap()
        .len()
}

#[test]
fn encrypted_pairing_lifecycle() {
    if std::env::var_os("VOYAGE_TEST_PAIRING_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!("voyage-pairing-test-{}", Uuid::new_v4()));
        let keys = PathBuf::from("/dev/shm").join(format!("voyage-key-test-{}", Uuid::new_v4()));
        registry::private_directory(&root).unwrap();
        registry::private_directory(&keys).unwrap();
        let key = keys.join("key");
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&key)
            .unwrap();
        use std::io::Write;
        f.write_all(&[7; 32]).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::pairing::tests::encrypted_pairing_lifecycle",
                "--nocapture",
            ])
            .env("VOYAGE_TEST_PAIRING_CHILD", &root)
            .env("VOYAGE_CREDENTIAL_KEY_FILE", &key)
            .output()
            .unwrap();
        std::fs::remove_dir_all(&keys).unwrap();
        assert!(
            output.status.success(),
            "child failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::remove_dir_all(root).unwrap();
        return;
    }
    let root = PathBuf::from(std::env::var_os("VOYAGE_TEST_PAIRING_CHILD").unwrap());
    let key = PathBuf::from(std::env::var_os("VOYAGE_CREDENTIAL_KEY_FILE").unwrap());
    registry::private_directory(&root.join("identity")).unwrap();
    store::save(
        &root.join("identity/key.json"),
        &json!({"vessel_id":Uuid::new_v4()}),
    )
    .unwrap();
    let workspace = root.join("workspace");
    registry::private_directory(&workspace).unwrap();
    let make_invite = || {
        invite(
            &root,
            "http://127.0.0.1:9999",
            Uuid::new_v4(),
            vec![ApprovedWorkspace {
                id: Uuid::new_v4(),
                path: workspace.clone(),
                name: "not-in-audit".into(),
                provider_ready: None,
            }],
            vec![ProcessRight::Observe],
            Vec::new(),
            Vec::new(),
            600,
        )
        .unwrap()
    };
    assert_eq!(
        connection_audit(&root, 64, None).unwrap()["recording"],
        "not_started"
    );
    let mut retained_revoke = None;
    for stage in ["grant_publish", "audit_finalize"] {
        let invitation = make_invite();
        let command = Uuid::new_v4();
        FAIL.with(|f| f.set(stage));
        assert!(
            redeem(
                &root,
                &invitation.endpoint,
                None,
                request(&invitation, command)
            )
            .is_err()
        );
        let before = count(&root);
        let credential = redeem(
            &root,
            &invitation.endpoint,
            None,
            request(&invitation, command),
        )
        .unwrap();
        assert_eq!(count(&root), before + 1); // one observation, not another intent
        let persisted = std::fs::read(state_path(&root)).unwrap();
        assert!(voyage_storage::credentials::encrypted(&persisted));
        assert!(
            !persisted
                .windows(credential.token.len())
                .any(|w| w == credential.token.as_bytes())
        );
        let again = redeem(
            &root,
            &invitation.endpoint,
            None,
            request(&invitation, command),
        )
        .unwrap();
        assert_eq!(again.token, credential.token);
        assert_eq!(std::fs::read(state_path(&root)).unwrap(), persisted);
        let audit = serde_json::to_string(&connection_audit(&root, 128, None).unwrap()).unwrap();
        for secret in [
            credential.token.as_str(),
            invitation.code.as_str(),
            "not-in-audit",
            workspace.to_str().unwrap(),
            "token_hash",
        ] {
            assert!(!audit.contains(secret));
        }
        let revoke_id = Uuid::new_v4();
        FAIL.with(|f| f.set("audit_finalize"));
        assert!(revoke(&root, credential.grant_id, 1, revoke_id).is_err());
        let before = count(&root);
        assert_eq!(
            revoke(&root, credential.grant_id, 1, revoke_id).unwrap()["cleanup"],
            "not_observed"
        );
        assert_eq!(count(&root), before + 1);
        let bytes = std::fs::read(state_path(&root)).unwrap();
        revoke(&root, credential.grant_id, 1, revoke_id).unwrap();
        assert_eq!(std::fs::read(state_path(&root)).unwrap(), bytes);
        assert!(revoke(&root, credential.grant_id, 2, revoke_id).is_err());
        retained_revoke = Some((credential.grant_id, revoke_id));
        assert!(
            redeem(
                &root,
                &invitation.endpoint,
                None,
                request(&invitation, command)
            )
            .is_err()
        );
    }
    let (revoked, command) = retained_revoke.unwrap();
    // Later revisions and pruned event windows do not change exact outcomes.
    revoke(&root, revoked, 2, Uuid::new_v4()).unwrap();
    let mut state = load(&root).unwrap();
    let vessel = identity(&root).unwrap();
    for _ in 0..4100 {
        append(
            &mut state,
            audit::event(Kind::AuditStarted, 1, vessel, None, None, None),
        )
        .unwrap();
    }
    save(&root, &state).unwrap();
    let before = std::fs::read(state_path(&root)).unwrap();
    assert_eq!(revoke(&root, revoked, 1, command).unwrap()["revision"], 2);
    assert_eq!(std::fs::read(state_path(&root)).unwrap(), before);
    let invitation = make_invite();
    let command = Uuid::new_v4();
    let credential = redeem(
        &root,
        &invitation.endpoint,
        None,
        request(&invitation, command),
    )
    .unwrap();
    std::fs::remove_file(connection_path(&root, credential.grant_id)).unwrap();
    assert!(
        redeem(
            &root,
            &invitation.endpoint,
            None,
            request(&invitation, command)
        )
        .is_err()
    );
    assert!(!connection_path(&root, credential.grant_id).exists());
    let before = std::fs::read(state_path(&root)).unwrap();
    std::fs::rename(&key, key.with_extension("locked")).unwrap();
    assert!(connection_audit(&root, 64, None).is_err());
    assert!(protect_credentials(&root).is_err());
    assert!(inventory(&root).is_ok()); // credential-free inventory needs no unlock
    assert_eq!(std::fs::read(state_path(&root)).unwrap(), before);
    std::fs::rename(key.with_extension("locked"), &key).unwrap();
    std::fs::write(&key, [8; 32]).unwrap();
    assert!(connection_audit(&root, 64, None).is_err());
    std::fs::write(&key, [7; 32]).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(connection_audit(&root, 64, None).is_err());
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let held = lock(&root).unwrap();
    assert!(connection_audit(&root, 64, None).is_err());
    drop(held);
    assert!(connection_audit(&root, 64, None).is_ok());
    // Explicit legacy conversion changes encoding only, not the exact retained state.
    let state = load(&root).unwrap();
    let plaintext = serde_json::to_vec(&state).unwrap();
    store::save_bytes(&state_path(&root), &plaintext, STATE_BYTES).unwrap();
    protect_credentials(&root).unwrap();
    assert_eq!(
        serde_json::to_vec(&load(&root).unwrap()).unwrap(),
        plaintext
    );
    let sealed = std::fs::read(state_path(&root)).unwrap();
    let mut corrupt = sealed.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    std::fs::write(state_path(&root), &corrupt).unwrap();
    assert!(protect_credentials(&root).is_err());
    assert_eq!(std::fs::read(state_path(&root)).unwrap(), corrupt);
    std::fs::write(state_path(&root), &sealed).unwrap();
    // Final review: dangling state links are unsafe, not a fresh empty journal.
    let saved_state = directory(&root).join("test-original-state");
    std::fs::rename(state_path(&root), &saved_state).unwrap();
    std::os::unix::fs::symlink("test-absent-state", state_path(&root)).unwrap();
    assert!(load(&root).is_err());
    assert!(protect_credentials(&root).is_err());
    assert!(store::save_bytes(&state_path(&root), b"{}", STATE_BYTES).is_err());
    assert!(
        std::fs::symlink_metadata(state_path(&root))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    std::fs::remove_file(state_path(&root)).unwrap();
    std::fs::rename(saved_state, state_path(&root)).unwrap();
    std::fs::set_permissions(state_path(&root), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store::save_bytes(&state_path(&root), b"{}", STATE_BYTES).is_err());
    assert_eq!(std::fs::read(state_path(&root)).unwrap(), sealed);
    std::fs::set_permissions(state_path(&root), std::fs::Permissions::from_mode(0o600)).unwrap();
}
