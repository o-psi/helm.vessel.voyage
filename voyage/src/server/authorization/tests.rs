use super::*;
use crate::policy::ExecutionAuthority;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use uuid::Uuid;
use voyage_protocol::process::{ProcessState, RuntimeCommand};
fn grant(workspace: &Path) -> ProcessGrant {
    ProcessGrant {
        full_access: false,
        grant_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        workspace: workspace.to_owned(),
        revision: 1,
        rights: vec![
            ProcessRight::Observe,
            ProcessRight::History,
            ProcessRight::Execute,
        ],
        accounts: vec![],
        enrollment_connections: vec![],
        expires_at_ms: u64::MAX,
        revoked: false,
        token_hash: "synthetic".into(),
        parent_grant: None,
        connection_binding: None,
        participant_binding: None,
    }
}
fn binding(g: &ProcessGrant) -> GrantBinding {
    GrantBinding {
        grant_id: g.grant_id,
        principal_id: g.principal_id,
        revision: g.revision,
    }
}
fn save(path: &Path, g: &impl serde::Serialize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(g).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
#[test]
fn current_authority_rechecks_identity_revision_expiry_and_revocation() {
    let root = tempfile::tempdir().unwrap();
    let original = grant(root.path());
    let b = binding(&original);
    let path = root.path().join("grant.json");
    save(&path, &original);
    read_current(&path, &b, original.session_id).unwrap();
    for field in [
        "grant",
        "principal",
        "session",
        "revision",
        "expired",
        "revoked",
    ] {
        let mut g = original.clone();
        match field {
            "grant" => g.grant_id = Uuid::new_v4(),
            "principal" => g.principal_id = Uuid::new_v4(),
            "session" => g.session_id = Uuid::new_v4(),
            "revision" => g.revision += 1,
            "expired" => g.expires_at_ms = 0,
            "revoked" => g.revoked = true,
            _ => unreachable!(),
        };
        save(&path, &g);
        assert!(
            read_current(&path, &b, original.session_id).is_err(),
            "{field}"
        );
    }
    save(&path, &original);
    let authority = GrantAuthority {
        path: path.clone(),
        binding: b,
        session: original.session_id,
        account: None,
        browser_history: true,
    };
    authority.check().unwrap();
    let mut g = original.clone();
    g.rights.retain(|r| *r != ProcessRight::History);
    save(&path, &g);
    assert!(authority.check().is_err());
    g = original;
    g.rights.retain(|r| *r != ProcessRight::Execute);
    save(&path, &g);
    assert!(authority.check().is_err());
}
#[test]
fn private_grant_files_reject_public_permissions_links_and_oversize() {
    let root = tempfile::tempdir().unwrap();
    let g = grant(root.path());
    let path = root.path().join("grant.json");
    save(&path, &g);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_private::<ProcessGrant>(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let link = root.path().join("link");
    symlink(&path, &link).unwrap();
    assert!(load_private::<ProcessGrant>(&link).is_err());
    let hard = root.path().join("hard");
    fs::hard_link(&path, &hard).unwrap();
    assert!(load_private::<ProcessGrant>(&path).is_err());
    fs::remove_file(hard).unwrap();
    fs::write(&path, vec![b' '; 16385]).unwrap();
    assert!(load_private::<ProcessGrant>(&path).is_err());
    fs::write(&path, b"{}").unwrap();
    assert!(load_private::<ProcessGrant>(&path).is_err());
}
#[test]
fn admission_enforces_operation_rights_supervised_path_and_workspace() {
    let root = tempfile::tempdir().unwrap();
    let mut g = grant(root.path());
    let directory = root.path().join("sessions").join(g.session_id.to_string());
    let path = root
        .path()
        .join("access/grants")
        .join(format!("{}.json", g.grant_id));
    save(&path, &g);
    let registration = ProcessRegistration {
        executable: None,
        protocol: 1,
        session_id: g.session_id,
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        restart_from: None,
        initialize: None,
        config_path: None,
        token: "fixture".into(),
        workspace: root.path().to_owned(),
        state: ProcessState::Live,
        name: None,
    };
    let actor = LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    };
    let mut request = RuntimeRequest {
        protocol: 1,
        session_id: g.session_id,
        incarnation: registration.incarnation,
        token: "fixture".into(),
        authorization: None,
        command: RuntimeCommand::Health,
    };
    let local = authorize_parts(actor, &registration, &request, &directory).unwrap();
    assert!(local.authority.is_none());
    assert_eq!(local.actor, actor);
    request.authorization = Some(binding(&g));
    let scoped = authorize_parts(actor, &registration, &request, &directory).unwrap();
    assert_eq!(scoped.actor.principal_id, g.principal_id);
    assert!(scoped.authority.is_some());
    assert!(authorize_parts(actor, &registration, &request, root.path()).is_err());
    assert!(
        authorize_parts(
            actor,
            &registration,
            &request,
            &root.path().join("wrong").join(g.session_id.to_string())
        )
        .is_err()
    );
    g.rights.clear();
    save(&path, &g);
    assert!(authorize_parts(actor, &registration, &request, &directory).is_err());
    g.rights.push(ProcessRight::Observe);
    g.workspace = root.path().join("different");
    save(&path, &g);
    assert!(authorize_parts(actor, &registration, &request, &directory).is_err());
}

#[test]
fn owner_connection_authority_requires_current_explicit_parent() {
    let root = tempfile::tempdir().unwrap();
    let mut g = grant(root.path());
    g.full_access = true;
    g.rights = ProcessRight::all();
    let path = root
        .path()
        .join("access/grants")
        .join(format!("{}.json", g.grant_id));
    save(&path, &g);
    assert!(read_current(&path, &binding(&g), g.session_id).is_err());
    let mut parent = voyage_protocol::process::ConnectionGrant {
        full_access: true,
        schema_version: 1,
        grant_id: Uuid::new_v4(),
        principal_id: g.principal_id,
        vessel_id: Uuid::new_v4(),
        revision: 1,
        rights: ProcessRight::all(),
        accounts: vec![],
        enrollment_connections: vec![],
        expires_at_ms: u64::MAX,
        revoked: false,
        token_hash: "synthetic".into(),
        workspaces: vec![],
    };
    save(
        &root.path().join("identity/key.json"),
        &serde_json::json!({"vessel_id": parent.vessel_id}),
    );
    g.connection_binding = Some(GrantBinding {
        grant_id: parent.grant_id,
        principal_id: parent.principal_id,
        revision: parent.revision,
    });
    let parent_path = root
        .path()
        .join("access/connections")
        .join(format!("{}.json", parent.grant_id));
    save(&path, &g);
    save(&parent_path, &parent);
    read_current(&path, &binding(&g), g.session_id).unwrap();
    for case in 0..4 {
        let mut changed = parent.clone();
        match case {
            0 => changed.full_access = false,
            1 => changed.revoked = true,
            2 => changed.revision += 1,
            _ => changed.expires_at_ms = 0,
        }
        save(&parent_path, &changed);
        assert!(read_current(&path, &binding(&g), g.session_id).is_err());
    }
    parent.full_access = false;
    save(&parent_path, &parent);
    let mut legacy = serde_json::to_value(&g).unwrap();
    legacy.as_object_mut().unwrap().remove("full_access");
    assert!(
        !serde_json::from_value::<ProcessGrant>(legacy)
            .unwrap()
            .full_access
    );
}
