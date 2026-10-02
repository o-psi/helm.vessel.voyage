//! Actual owned SQLite effects plus the production late reply boundary, not an
//! end-to-end scheduling race, runtime launch, provider or native-owner claim.
use super::*;
use crate::process::{database, identity, test_support::Fixture};
use voyage_protocol::notifications::{Destination, NotificationKind, NotificationOperation};
const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn accepted_session_metadata_then_complete_saved_authority_drift_retains_receipt_as_unknown()
{
    for drift in [
        "token",
        "rights",
        "revision",
        "revoked",
        "account",
        "enrollment",
        "parent",
        "participant",
    ] {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let r = f.registration();
        database::save(&f.0, &r).await.unwrap();
        let mut g = f.session();
        g.session_id = r.session_id;
        g.token_hash = store::hash(TOKEN);
        g.rights = vec![ProcessRight::Observe];
        f.save_session(&g);
        let destination = Destination {
            id: Uuid::new_v4(),
            recipient_grant_id: g.grant_id,
            recipient_principal_id: g.principal_id,
            recipient_grant_revision: g.revision,
            source_vessel_id: identity::public(&f.0).unwrap().vessel_id,
            source_session_id: r.session_id,
            event_kinds: vec![NotificationKind::Test],
            expires_at_ms: store::now().unwrap() + 60_000,
            notification_ttl_ms: 30_000,
            quiet_hours_utc: None,
        };
        s.notifications(
            NotificationOperation::Configure {
                command_id: Uuid::new_v4(),
                destination: destination.clone(),
            },
            None,
        )
        .await
        .unwrap();
        let id = Uuid::new_v4();
        let accepted = s
            .notifications(
                NotificationOperation::Accept {
                    command_id: id,
                    destination_id: destination.id,
                },
                Some(GrantBinding {
                    grant_id: g.grant_id,
                    principal_id: g.principal_id,
                    revision: g.revision,
                }),
            )
            .await
            .unwrap();
        assert!(accepted["accepted_at_ms"].as_u64().is_some());
        let receipt = || {
            let db = rusqlite::Connection::open_with_flags(
                f.0.join("notifications/notifications.sqlite3"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let command: (String, String, i64) = db
                .query_row(
                    "SELECT id,destination_id,operation FROM commands WHERE id=?1",
                    [id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            let state: (String, i64) = db
                .query_row(
                    "SELECT payload,accepted FROM destinations WHERE id=?1",
                    [destination.id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let count: i64 = db
                .query_row("SELECT count(*) FROM commands", [], |row| row.get(0))
                .unwrap();
            (command, state, count)
        };
        let before = receipt();
        assert_eq!(before.0, (id.to_string(), destination.id.to_string(), 2));
        assert_eq!(before.2, 2);
        let mut changed = g.clone();
        match drift {
            "token" => changed.token_hash = store::hash("late-session-token-rotation"),
            "rights" => changed.rights.clear(),
            "revision" => changed.revision += 1,
            "revoked" => changed.revoked = true,
            "account" => changed.accounts.push(Uuid::new_v4()),
            "enrollment" => changed.enrollment_connections.push(Uuid::new_v4()),
            "parent" => {
                changed.parent_grant = Some(GrantBinding {
                    grant_id: Uuid::new_v4(),
                    principal_id: g.principal_id,
                    revision: 1,
                })
            }
            _ => {
                changed.participant_binding = Some(ParticipantGrantBinding {
                    binding_id: Uuid::new_v4(),
                    revision: 1,
                })
            }
        }
        f.save_session(&changed);
        let wire = super::super::super::api::response(s.finish_granted_reply(&g, TOKEN, accepted));
        assert!(wire.error.is_some());
        assert!(wire.outcome_unknown, "late {drift}");
        assert!(wire.result.is_null());
        assert_eq!(
            receipt(),
            before,
            "exact accepted receipt must not be replayed or rewritten"
        );
    }
}

#[tokio::test]
async fn session_read_success_redaction_and_preflight_refusal_keep_their_outcome_distinctions() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = f.session();
    g.token_hash = store::hash(TOKEN);
    g.rights = vec![ProcessRight::Observe];
    f.save_session(&g);
    let vessel = identity::public(&f.0).unwrap().vessel_id;
    let frozen = store::process_authority_fingerprint(&g).unwrap();
    let normal = super::super::super::api::response(
        s.granted_with_authority(
            g.grant_id,
            TOKEN.into(),
            Some(vessel),
            Some(frozen.clone()),
            VesselCommand::Capabilities,
        )
        .await,
    );
    assert!(normal.error.is_none());
    assert!(!normal.outcome_unknown);
    let redacted = s
        .finish_granted_reply(
            &g,
            TOKEN,
            json!({"catalogue":{"summary":"private fixture history"}}),
        )
        .unwrap();
    assert!(redacted["catalogue"]["summary"].is_null());
    g.accounts.push(Uuid::new_v4());
    f.save_session(&g);
    let refused = super::super::super::api::response(
        s.granted_with_authority(
            g.grant_id,
            TOKEN.into(),
            Some(vessel),
            Some(frozen),
            VesselCommand::Capabilities,
        )
        .await,
    );
    assert!(refused.error.is_some());
    assert!(!refused.outcome_unknown);
    assert!(refused.result.is_null());
    assert!(s.registrations.lock().await.unwrap().is_empty());
    assert!(!f.0.join("notifications").exists());
    let held = g.clone();
    g.accounts.push(Uuid::new_v4());
    f.save_session(&g);
    let withheld = super::super::super::api::response(s.finish_granted_reply(
        &held,
        TOKEN,
        json!({"safe":true}),
    ));
    assert!(withheld.error.is_some());
    assert!(withheld.outcome_unknown);
    assert!(withheld.result.is_null());
}
