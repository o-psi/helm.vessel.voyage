use super::super::test_support::Fixture;
use super::*;

fn destination(f: &Fixture, r: &ProcessRegistration) -> Destination {
    let vessel = super::super::identity::public(&f.0).unwrap().vessel_id;
    Destination {
        id: Uuid::new_v4(),
        recipient_grant_id: vessel,
        recipient_principal_id: vessel,
        recipient_grant_revision: 1,
        source_vessel_id: vessel,
        source_session_id: r.session_id,
        event_kinds: vec![NotificationKind::Attention],
        expires_at_ms: u64::MAX - 1,
        notification_ttl_ms: 500,
        quiet_hours_utc: None,
    }
}

#[test]
fn local_authority_requires_exact_source_and_recipient_identity() {
    let f = Fixture::new();
    let r = f.registration();
    let d = destination(&f, &r);
    assert!(matches!(
        current_authority(&f.0, &d, ProcessRight::Observe, &r).unwrap(),
        RecipientAuthority::Local
    ));
    for change in 0..4 {
        let mut changed = d.clone();
        match change {
            0 => changed.source_vessel_id = Uuid::new_v4(),
            1 => changed.source_session_id = Uuid::new_v4(),
            2 => changed.recipient_principal_id = Uuid::new_v4(),
            _ => changed.recipient_grant_revision += 1,
        }
        assert!(current_authority(&f.0, &changed, ProcessRight::Observe, &r).is_err());
    }
    let mut relinquished = r;
    relinquished.state = ProcessState::Relinquished;
    assert!(current_authority(&f.0, &d, ProcessRight::Observe, &relinquished).is_err());
}

#[test]
fn session_recipient_cannot_gain_history_or_outlive_its_grant() {
    let f = Fixture::new();
    let r = f.registration();
    let mut d = destination(&f, &r);
    let mut g = f.session();
    g.session_id = r.session_id;
    d.recipient_grant_id = g.grant_id;
    d.recipient_principal_id = g.principal_id;
    d.recipient_grant_revision = g.revision;
    f.save_session(&g);
    assert!(matches!(
        current_authority(&f.0, &d, ProcessRight::Observe, &r).unwrap(),
        RecipientAuthority::Session(_)
    ));
    d.event_kinds.push(NotificationKind::Budget);
    assert!(current_authority(&f.0, &d, ProcessRight::Observe, &r).is_err());
    g.rights.push(ProcessRight::History);
    f.save_session(&g);
    assert!(current_authority(&f.0, &d, ProcessRight::Observe, &r).is_ok());
    for change in 0..7 {
        let mut changed = g.clone();
        match change {
            0 => changed.expires_at_ms = d.expires_at_ms - 1,
            1 => changed.session_id = Uuid::new_v4(),
            2 => changed.workspace = f.0.join("other"),
            3 => changed.revision += 1,
            4 => changed.revoked = true,
            5 => {
                changed.parent_grant = Some(GrantBinding {
                    grant_id: Uuid::new_v4(),
                    principal_id: g.principal_id,
                    revision: 1,
                })
            }
            _ => {
                changed.connection_binding = Some(GrantBinding {
                    grant_id: Uuid::new_v4(),
                    principal_id: g.principal_id,
                    revision: 1,
                })
            }
        }
        f.save_session(&changed);
        assert!(
            current_authority(&f.0, &d, ProcessRight::Observe, &r).is_err(),
            "change {change}"
        );
    }
}

#[test]
fn connection_recipient_rechecks_budget_permission_workspace_and_revision() {
    let f = Fixture::new();
    let r = f.registration();
    let mut d = destination(&f, &r);
    let mut g = f.connection();
    d.recipient_grant_id = g.grant_id;
    d.recipient_principal_id = g.principal_id;
    d.recipient_grant_revision = g.revision;
    f.save_connection(&g);
    assert!(matches!(
        current_authority(&f.0, &d, ProcessRight::Observe, &r).unwrap(),
        RecipientAuthority::Connection(_)
    ));
    d.event_kinds.push(NotificationKind::Budget);
    assert!(current_authority(&f.0, &d, ProcessRight::Observe, &r).is_err());
    g.rights.push(ProcessRight::History);
    f.save_connection(&g);
    assert!(current_authority(&f.0, &d, ProcessRight::Observe, &r).is_ok());
    for change in 0..6 {
        let mut changed = g.clone();
        match change {
            0 => changed.workspaces[0].path = f.0.join("other"),
            1 => changed.rights.clear(),
            2 => changed.revision += 1,
            3 => changed.principal_id = Uuid::new_v4(),
            4 => changed.expires_at_ms = d.expires_at_ms - 1,
            _ => changed.revoked = true,
        }
        f.save_connection(&changed);
        assert!(
            current_authority(&f.0, &d, ProcessRight::Observe, &r).is_err(),
            "change {change}"
        );
    }
}

#[test]
fn owner_notifications_follow_new_workspaces_but_not_revoked_authority() {
    let f = Fixture::new();
    let mut r = f.registration();
    r.workspace = f.0.join("later");
    crate::process::registry::private_directory(&r.workspace).unwrap();
    let mut g = f.connection();
    g.full_access = true;
    g.rights = ProcessRight::all();
    g.workspaces.clear();
    g.accounts.clear();
    g.enrollment_connections.clear();
    let mut d = destination(&f, &r);
    d.recipient_grant_id = g.grant_id;
    d.recipient_principal_id = g.principal_id;
    d.recipient_grant_revision = g.revision;
    d.event_kinds.push(NotificationKind::Budget);
    f.save_connection(&g);
    assert!(current_authority(&f.0, &d, ProcessRight::Observe, &r).is_ok());
    g.revoked = true;
    f.save_connection(&g);
    assert!(current_authority(&f.0, &d, ProcessRight::Observe, &r).is_err());
}
