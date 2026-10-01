//! Private account supervision contracts without identity-drop helpers or providers.
use super::*;
use crate::process::{access::store, identity_accounts::test_fixtures as f, test_support::Fixture};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn check(
    scope: &Scope,
    workspace: &Path,
    right: IdentityAuthorityRight,
    connection: Uuid,
    account: Option<Uuid>,
) -> IdentityAuthorityRequest {
    IdentityAuthorityRequest {
        schema: 1,
        actor: scope.actor(workspace),
        right,
        connection_id: connection,
        account_id: account,
    }
}
fn account_projection(scope: &Scope, workspace: &Path) -> IdentityAccountScope {
    IdentityAccountScope {
        full_access: false,
        can_use: true,
        can_enroll: false,
        account_ids: vec![],
        enrollment_connections: vec![],
        actor: scope.actor(workspace),
    }
}

#[test]
fn projected_owner_connection_and_session_scopes_preserve_current_account_and_enrollment_rights() {
    let root = Fixture::new();
    let owner = projection(&Scope::Owner, &root.0, &root.0).unwrap();
    assert!(owner.full_access && owner.can_use && owner.can_enroll);
    assert_eq!(owner.actor.principal, "owner");
    assert!(owner.account_ids.is_empty());
    for rights in [
        vec![ProcessRight::AccountUse],
        vec![ProcessRight::AccountEnroll],
        vec![ProcessRight::AccountUse, ProcessRight::AccountEnroll],
    ] {
        let mut grant = root.connection();
        grant.rights = rights.clone();
        root.save_connection(&grant);
        let scope = Scope::Connection(grant.clone());
        let projected = projection(&scope, &root.0, &root.0).unwrap();
        assert_eq!(
            projected.can_use,
            rights.contains(&ProcessRight::AccountUse)
        );
        assert_eq!(
            projected.can_enroll,
            rights.contains(&ProcessRight::AccountEnroll)
        );
        assert_eq!(projected.account_ids, grant.accounts);
        assert_eq!(
            projected.enrollment_connections,
            grant.enrollment_connections
        );
        assert_eq!(projected.actor, scope.actor(&root.0));
        assert!(!projected.full_access);
        let mut session = root.session();
        session.rights = rights.clone();
        root.save_session(&session);
        let scope = Scope::Session(session.clone());
        let projected = projection(&scope, &root.0, &root.0).unwrap();
        assert_eq!(
            projected.can_use,
            rights.contains(&ProcessRight::AccountUse)
        );
        assert_eq!(
            projected.can_enroll,
            rights.contains(&ProcessRight::AccountEnroll)
        );
        assert_eq!(projected.account_ids, session.accounts);
        assert_eq!(
            projected.enrollment_connections,
            session.enrollment_connections
        );
    }
    let mut denied = root.connection();
    denied.rights = vec![ProcessRight::Observe];
    root.save_connection(&denied);
    assert!(projection(&Scope::Connection(denied), &root.0, &root.0).is_err());
    assert!(projection(&Scope::Owner, &root.0, Path::new("relative")).is_err());
    assert!(
        !root.0.join("accounts").exists(),
        "projection cannot open provider credentials"
    );
}

#[test]
fn stale_expired_revoked_and_changed_private_grants_cannot_project_account_authority() {
    let root = Fixture::new();
    for change in 0..8 {
        let grant = root.connection();
        root.save_connection(&grant);
        let mut current = grant.clone();
        match change {
            0 => current.revoked = true,
            1 => current.expires_at_ms = 0,
            2 => current.revision += 1,
            3 => current.principal_id = Uuid::new_v4(),
            4 => current.accounts.push(Uuid::new_v4()),
            5 => current.enrollment_connections.clear(),
            6 => current.rights.clear(),
            _ => current.workspaces.clear(),
        };
        root.save_connection(&current);
        assert!(projection(&Scope::Connection(grant), &root.0, &root.0).is_err());
    }
    for change in 0..7 {
        let grant = root.session();
        root.save_session(&grant);
        let mut current = grant.clone();
        match change {
            0 => current.revoked = true,
            1 => current.expires_at_ms = 0,
            2 => current.revision += 1,
            3 => current.principal_id = Uuid::new_v4(),
            4 => current.accounts.clear(),
            5 => current.enrollment_connections.clear(),
            _ => current.rights.clear(),
        };
        root.save_session(&current);
        assert!(projection(&Scope::Session(grant), &root.0, &root.0).is_err());
    }
}

#[test]
fn ordinary_private_scope_drift_and_symlink_aliases_refuse_without_reading_credentials() {
    let root = Fixture::new();
    let grant = root.connection();
    root.save_connection(&grant);
    let other = Fixture::new();
    assert!(projection(&Scope::Connection(grant.clone()), &root.0, &other.0).is_err());
    let alias = root.0.join("workspace-alias");
    std::os::unix::fs::symlink(&root.0, &alias).unwrap();
    assert!(projection(&Scope::Owner, &root.0, &alias).is_err());
    let path = store::connection_path(&root.0, grant.grant_id);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(other.0.join("unrelated"), &path).unwrap();
    assert!(projection(&Scope::Connection(grant), &root.0, &root.0).is_err());
    assert!(!other.0.join("unrelated").exists());
    assert!(!bytes.is_empty());
}

#[test]
fn bound_account_selection_keeps_process_owner_command_token_workspace_configuration_and_peers_exact()
 {
    let registration = f::registration();
    bound_current(&registration, &registration).unwrap();
    let mut state_only = registration.clone();
    state_only.state = ProcessState::Unavailable;
    bound_current(&registration, &state_only).unwrap();
    for change in 0..6 {
        let mut current = registration.clone();
        match change {
            0 => current.incarnation = Uuid::new_v4(),
            1 => current.peer_uids = None,
            2 => current.command_id = Uuid::new_v4(),
            3 => current.token = "different private token".into(),
            4 => current.workspace = "/fixture/other".into(),
            _ => current.config_path = Some("/fixture/other-config".into()),
        };
        assert!(bound_current(&registration, &current).is_err());
    }
}

#[test]
fn bound_account_grant_must_match_exact_principal_revision_session_and_workspace() {
    let root = Fixture::new();
    let registration = root.registration();
    let mut grant = root.session();
    grant.session_id = registration.session_id;
    let binding = GrantBinding {
        grant_id: grant.grant_id,
        principal_id: grant.principal_id,
        revision: grant.revision,
    };
    account_scope_current(&grant, &binding, &registration).unwrap();
    for change in 0..5 {
        let mut current = grant.clone();
        match change {
            0 => current.grant_id = Uuid::new_v4(),
            1 => current.principal_id = Uuid::new_v4(),
            2 => current.revision += 1,
            3 => current.session_id = Uuid::new_v4(),
            _ => current.workspace = root.0.join("different"),
        };
        assert!(account_scope_current(&current, &binding, &registration).is_err());
    }
}

#[test]
fn callback_actor_tracks_grant_revision_parent_connection_and_workspace_not_a_borrowed_principal() {
    let root = Fixture::new();
    let grant = root.session();
    let scope = Scope::Session(grant.clone());
    let mut request = check(
        &scope,
        &root.0,
        IdentityAuthorityRight::Use,
        Uuid::new_v4(),
        Some(Uuid::new_v4()),
    );
    assert!(callback_actor_current(&scope, &root.0, &request));
    request.actor.workspace = root.0.join("different").to_string_lossy().into_owned();
    assert!(!callback_actor_current(&scope, &root.0, &request));
    request.actor = scope.actor(&root.0);
    request.actor.principal = "owner".into();
    assert!(!callback_actor_current(&scope, &root.0, &request));
    let mut derived = grant.clone();
    derived.connection_binding = Some(GrantBinding {
        grant_id: Uuid::new_v4(),
        principal_id: grant.principal_id,
        revision: 9,
    });
    let parent_scope = Scope::Session(derived.clone());
    request.actor = parent_scope.actor(&root.0);
    assert!(callback_actor_current(&parent_scope, &root.0, &request));
    assert!(!callback_actor_current(&scope, &root.0, &request));
    derived.connection_binding.as_mut().unwrap().revision += 1;
    assert!(!callback_actor_current(
        &Scope::Session(derived),
        &root.0,
        &request
    ));
}

#[test]
fn callback_permission_does_not_replace_current_authority_and_limits_account_connection_and_operation()
 {
    let root = Fixture::new();
    let mut grant = root.connection();
    let allowed_connection = grant.enrollment_connections[0];
    let allowed_account = grant.accounts[0];
    let scope = Scope::Connection(grant.clone());
    let mut projection = account_projection(&scope, &root.0);
    projection.account_ids = grant.accounts.clone();
    let use_account = check(
        &scope,
        &root.0,
        IdentityAuthorityRight::Use,
        allowed_connection,
        Some(allowed_account),
    );
    assert!(callback_permission(&scope, &use_account, Some(&projection)));
    assert!(!callback_permission(&scope, &use_account, None));
    let mut unknown = check(
        &scope,
        &root.0,
        IdentityAuthorityRight::Use,
        allowed_connection,
        Some(Uuid::new_v4()),
    );
    assert!(!callback_permission(&scope, &unknown, Some(&projection)));
    projection.can_enroll = true;
    assert!(callback_permission(&scope, &unknown, Some(&projection)));
    unknown.connection_id = Uuid::new_v4();
    assert!(!callback_permission(&scope, &unknown, Some(&projection)));
    let missing = check(
        &scope,
        &root.0,
        IdentityAuthorityRight::Use,
        allowed_connection,
        None,
    );
    projection.full_access = true;
    assert!(!callback_permission(&scope, &missing, Some(&projection)));
    let enroll = check(
        &scope,
        &root.0,
        IdentityAuthorityRight::Enroll,
        allowed_connection,
        None,
    );
    assert!(callback_permission(&scope, &enroll, None));
    let with_account = check(
        &scope,
        &root.0,
        IdentityAuthorityRight::Enroll,
        allowed_connection,
        Some(allowed_account),
    );
    assert!(!callback_permission(&scope, &with_account, None));
    let denied = check(
        &scope,
        &root.0,
        IdentityAuthorityRight::Enroll,
        Uuid::new_v4(),
        None,
    );
    assert!(!callback_permission(&scope, &denied, None));
    grant.full_access = true;
    let full = Scope::Connection(grant);
    assert!(callback_permission(&full, &denied, None));
    let recover = check(
        &Scope::Owner,
        &root.0,
        IdentityAuthorityRight::Recover,
        allowed_connection,
        Some(allowed_account),
    );
    assert!(
        !callback_permission(&Scope::Owner, &recover, Some(&projection)),
        "offline recovery cannot borrow account authority"
    );
}

#[test]
fn helper_request_and_response_are_bounded_and_wrong_operation_outputs_have_no_private_diagnostics()
{
    let root = Fixture::new();
    let scope = Scope::Owner;
    let projected = account_projection(&scope, &root.0);
    let mut request = IdentityHelperRequest {
        schema: IDENTITY_HELPER_SCHEMA,
        workspace: root.0.clone(),
        operation: IdentityHelperOperation::Accounts {
            scope: projected.clone(),
            transport: None,
        },
    };
    request_bound(&request).unwrap();
    if let IdentityHelperOperation::Accounts { scope, .. } = &mut request.operation {
        scope.actor.principal = "x".repeat(IDENTITY_HELPER_BYTES);
    }
    assert!(
        request_bound(&request)
            .unwrap_err()
            .to_string()
            .contains("request exceeds bounds")
    );
    let value = serde_json::json!({"scoped":true});
    assert_eq!(
        helper_value(IdentityHelperResponse::Value {
            value: value.clone()
        })
        .unwrap(),
        value
    );
    for response in [
        IdentityHelperResponse::Unavailable {},
        IdentityHelperResponse::Facts {
            facts: f::config_facts(&f::identity(
                voyage_protocol::execution_identity::AuthorityClass::Ordinary,
            )),
        },
    ] {
        assert_eq!(
            helper_value(response).unwrap_err().to_string(),
            "identity account operation unavailable"
        );
    }
    response_bound(&IdentityHelperResponse::Value {
        value: serde_json::json!({"bounded":true}),
    })
    .unwrap();
    assert!(
        response_bound(&IdentityHelperResponse::Value {
            value: serde_json::json!("x".repeat(IDENTITY_HELPER_BYTES))
        })
        .is_err()
    );
}

#[tokio::test]
async fn authority_pipe_accepts_only_bounded_schema_and_non_nil_connection_frames() {
    let root = Fixture::new();
    let request = check(
        &Scope::Owner,
        &root.0,
        IdentityAuthorityRight::Use,
        Uuid::new_v4(),
        Some(Uuid::new_v4()),
    );
    let bytes = serde_json::to_vec(&request).unwrap();
    let (mut sender, mut receiver) = tokio::net::UnixStream::pair().unwrap();
    sender.write_u32(bytes.len() as u32).await.unwrap();
    sender.write_all(&bytes).await.unwrap();
    let parsed = crate::process::identity_authority::read(&mut receiver)
        .await
        .unwrap();
    assert_eq!(parsed.actor, request.actor);
    assert_eq!(parsed.connection_id, request.connection_id);
    assert_eq!(parsed.account_id, request.account_id);
    for bad in [vec![],b"not-json".to_vec(),serde_json::to_vec(&serde_json::json!({"schema":2,"actor":request.actor,"right":"use","connection_id":request.connection_id,"account_id":request.account_id})).unwrap(),serde_json::to_vec(&serde_json::json!({"schema":1,"actor":request.actor,"right":"use","connection_id":Uuid::nil(),"account_id":request.account_id})).unwrap(),serde_json::to_vec(&serde_json::json!({"schema":1,"actor":request.actor,"right":"use","connection_id":request.connection_id,"account_id":request.account_id,"executable":"not-an-authority-surface"})).unwrap()] {
        let (mut sender,mut receiver)=tokio::net::UnixStream::pair().unwrap();sender.write_u32(bad.len() as u32).await.unwrap();sender.write_all(&bad).await.unwrap();sender.shutdown().await.unwrap();assert!(crate::process::identity_authority::read(&mut receiver).await.is_err());
    }
    let (mut sender, mut receiver) = tokio::net::UnixStream::pair().unwrap();
    sender.write_u32(2049).await.unwrap();
    assert!(
        crate::process::identity_authority::read(&mut receiver)
            .await
            .is_err()
    );
    let (mut sender, mut receiver) = tokio::net::UnixStream::pair().unwrap();
    sender.write_u32(20).await.unwrap();
    sender.write_all(b"short").await.unwrap();
    sender.shutdown().await.unwrap();
    assert!(
        crate::process::identity_authority::read(&mut receiver)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn authority_reply_contains_only_boolean_schema_and_no_account_or_execution_material() {
    for allowed in [false, true] {
        let (mut sender, mut receiver) = tokio::net::UnixStream::pair().unwrap();
        crate::process::identity_authority::reply(&mut sender, allowed)
            .await
            .unwrap();
        let length = receiver.read_u32().await.unwrap();
        assert!(length <= 128);
        let mut bytes = vec![0; length as usize];
        receiver.read_exact(&mut bytes).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({"schema":1,"allowed":allowed})
        );
    }
}

#[tokio::test]
async fn ordinary_callers_cannot_select_review_targets_or_spawn_identity_helpers() {
    let root = Fixture::new();
    let identity = f::identity(voyage_protocol::execution_identity::AuthorityClass::Ordinary);
    assert!(
        selected_for(&Scope::Owner, &root.0, Selection::ReviewTarget(&identity))
            .await
            .unwrap_err()
            .to_string()
            .contains("enrolled human owner")
    );
    assert!(
        selected_for(
            &Scope::Session(root.session()),
            &root.0,
            Selection::ReviewTarget(&identity)
        )
        .await
        .is_err()
    );
    let response = run_owned_helper(
        &root.0,
        &root.0.join("never-created-binary"),
        &Scope::Owner,
        &root.0,
        ProcessRight::AccountUse,
        IdentityHelperOperation::ReviewConfig {
            config_path: root.0.join("private-do-not-read"),
        },
        Selection::Default,
    )
    .await;
    assert!(response.is_err());
    assert!(!root.0.join("private-do-not-read").exists());
    assert!(!root.0.join("identity-enrollments").exists());
    assert!(!root.0.join("never-created-binary").exists());
}
