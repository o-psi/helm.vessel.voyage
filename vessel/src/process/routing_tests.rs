use super::super::{database, registry, test_support::Fixture};
use super::*;
use serde_json::json;
use tokio::net::UnixListener;
use uuid::Uuid;

async fn peer(
    directory: &Path,
    registration: &ProcessRegistration,
    result: serde_json::Value,
    fault: &str,
) -> tokio::task::JoinHandle<RuntimeRequest> {
    let listener = UnixListener::bind(directory.join("runtime.sock")).unwrap();
    let r = registration.clone();
    let fault = fault.to_owned();
    tokio::spawn(async move {
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let request: RuntimeRequest = read_frame(&mut stream).await.unwrap();
        assert_eq!(request.protocol, PROCESS_PROTOCOL);
        assert_eq!(request.session_id, r.session_id);
        assert_eq!(request.incarnation, r.incarnation);
        assert_eq!(request.token, r.token);
        let response = RuntimeResponse {
            protocol: if fault == "protocol" {
                PROCESS_PROTOCOL + 1
            } else {
                PROCESS_PROTOCOL
            },
            session_id: if fault == "session" {
                Uuid::new_v4()
            } else {
                r.session_id
            },
            incarnation: if fault == "incarnation" {
                Uuid::new_v4()
            } else {
                r.incarnation
            },
            resumed_from: None,
            result,
            error: (fault == "rejected").then(|| "offline refusal".into()),
            outcome_unknown: fault == "unknown",
        };
        if fault != "disconnect" {
            write_frame(&mut stream, &response).await.unwrap();
        }
        request
    })
}

#[tokio::test]
async fn private_forwarding_preserves_identity_authorization_payload_and_refusal() {
    for fault in ["", "rejected", "unknown"] {
        let f = Fixture::new();
        let mut r = f.registration();
        r.state = ProcessState::Live;
        let binding = GrantBinding {
            grant_id: Uuid::new_v4(),
            revision: 19,
            principal_id: Uuid::new_v4(),
        };
        let task = peer(&f.0, &r, json!({"cursor": 72, "events": []}), fault).await;
        let response = forward_authorized(
            &f.0,
            &r,
            RuntimeCommand::Events {
                after: 71,
                limit: 9,
                wait_ms: 0,
            },
            Some(binding.clone()),
        )
        .await
        .unwrap();
        assert_eq!(response.result["cursor"], 72);
        assert_eq!(response.error.is_some(), fault == "rejected");
        assert_eq!(response.outcome_unknown, fault == "unknown");
        let request = task.await.unwrap();
        let actual = request.authorization.unwrap();
        assert_eq!(actual.grant_id, binding.grant_id);
        assert_eq!(actual.principal_id, binding.principal_id);
        assert_eq!(actual.revision, 19);
        assert!(matches!(
            request.command,
            RuntimeCommand::Events {
                after: 71,
                limit: 9,
                wait_ms: 0
            }
        ));
    }
}

#[tokio::test]
async fn mismatched_identity_and_lost_reply_are_unknown_not_replayed() {
    for fault in ["protocol", "session", "incarnation", "disconnect"] {
        let f = Fixture::new();
        let mut r = f.registration();
        r.state = ProcessState::Live;
        let task = peer(&f.0, &r, json!({}), fault).await;
        let error = forward(&f.0, &r, RuntimeCommand::Health).await.unwrap_err();
        assert!(error.downcast_ref::<OutcomeUnknown>().is_some());
        assert!(error.downcast_ref::<NotConnected>().is_none());
        if fault != "disconnect" {
            assert!(format!("{error:#}").contains("runtime identity mismatch"));
        }
        assert!(matches!(
            task.await.unwrap().command,
            RuntimeCommand::Health
        ));
    }
    let f = Fixture::new();
    let mut r = f.registration();
    r.state = ProcessState::Live;
    let error = forward(&f.0, &r, RuntimeCommand::Health).await.unwrap_err();
    assert!(error.downcast_ref::<NotConnected>().is_some());
    assert!(error.downcast_ref::<OutcomeUnknown>().is_some());
}

#[tokio::test]
async fn inspection_uses_health_without_exposing_private_registration() {
    for (state, fault, expected) in [
        (ProcessState::Live, "", ProcessState::Live),
        (
            ProcessState::Starting,
            "rejected",
            ProcessState::Unavailable,
        ),
        (
            ProcessState::Relinquished,
            "rejected",
            ProcessState::Relinquished,
        ),
        (ProcessState::Stopped, "rejected", ProcessState::Stopped),
    ] {
        let f = Fixture::new();
        let mut r = f.registration();
        r.state = state;
        let task = peer(&f.0, &r, json!({}), fault).await;
        let info = inspect(&f.0, &r).await;
        assert_eq!(info.state, expected);
        assert_eq!(info.session_id, r.session_id);
        assert_eq!(info.incarnation, r.incarnation);
        assert!(!serde_json::to_string(&info).unwrap().contains(&r.token));
        assert!(matches!(
            task.await.unwrap().command,
            RuntimeCommand::Health
        ));
    }
}

#[tokio::test]
async fn supervisor_selects_current_owner_and_fences_exact_owner_before_ipc() {
    use voyage_protocol::vessel::{VoyageCommand, VoyageRequest};
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut r = f.registration();
    r.state = ProcessState::Live;
    database::save(&f.0, &r).await.unwrap();
    let dir = registry::directory(&f.0, r.session_id);
    registry::private_directory(&dir).unwrap();
    let task = peer(&dir, &r, json!({"revision": 12}), "").await;
    let result = s
        .voyage(
            VoyageRequest {
                session_id: r.session_id,
                incarnation: None,
                command: VoyageCommand::Snapshot,
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(result["result"]["revision"], 12);
    assert!(matches!(
        task.await.unwrap().command,
        RuntimeCommand::Snapshot
    ));
    // A stale explicit fence is refused before any second connection is attempted.
    let error = s
        .voyage(
            VoyageRequest {
                session_id: r.session_id,
                incarnation: Some(Uuid::new_v4()),
                command: VoyageCommand::Cancel {
                    command_id: Uuid::new_v4(),
                    expected_revision: 12,
                    expires_at_ms: u64::MAX,
                    run_id: Uuid::new_v4(),
                },
            },
            None,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("stale runtime incarnation"));
    assert!(error.downcast_ref::<OutcomeUnknown>().is_none());
}
