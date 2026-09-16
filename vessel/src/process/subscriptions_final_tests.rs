//! Offline subscription observation: no HTTP server, credentials, or environment changes.
use super::*;
use crate::process::{database, registry, test_support::Fixture};
use tokio::net::UnixListener;
use voyage_protocol::process::{
    PROCESS_PROTOCOL, RuntimeRequest, RuntimeResponse, read_frame, write_frame,
};

#[tokio::test]
async fn subscription_tracks_current_owner_and_marks_replay_gap_only_on_change() {
    for changed in [false, true] {
        let f = Fixture::new();
        let supervisor = Arc::new(f.supervisor().await);
        let mut r = f.registration();
        r.state = ProcessState::Live;
        database::save(&f.0, &r).await.unwrap();
        let dir = registry::directory(&f.0, r.session_id);
        registry::private_directory(&dir).unwrap();
        let listener = UnixListener::bind(dir.join("runtime.sock")).unwrap();
        let session_id = r.session_id;
        let incarnation = r.incarnation;
        let server = tokio::spawn(async move {
            let (mut stream, _) =
                tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let request: RuntimeRequest = read_frame(&mut stream).await.unwrap();
            let value = serde_json::to_value(request.command).unwrap();
            assert_eq!(value["after"], 7);
            assert_eq!(value["limit"], 128);
            assert_eq!(value["wait_ms"], 10000);
            write_frame(
                &mut stream,
                &RuntimeResponse {
                    protocol: PROCESS_PROTOCOL,
                    session_id,
                    incarnation,
                    resumed_from: None,
                    result: serde_json::json!({"events":[],"next_after":8}),
                    error: None,
                    outcome_unknown: false,
                },
            )
            .await
            .unwrap();
        });
        let old = if changed { Uuid::new_v4() } else { incarnation };
        let (subscription, event, keep) = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            observe_local(
                supervisor,
                VesselEventSubscription {
                    session_id,
                    incarnation: old,
                    after: 7,
                },
            ),
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert!(keep);
        assert!(event.error.is_none());
        assert_eq!(subscription.incarnation, incarnation);
        assert_eq!(event.incarnation, incarnation);
        assert_eq!(event.result["next_after"], 8);
        if changed {
            assert_eq!(event.result["owner_changed"], true);
            assert_eq!(event.result["replay_gap"], true);
            assert_eq!(event.result["recovery"], "snapshot");
        } else {
            assert!(event.result.get("owner_changed").is_none());
        }
    }
}

#[tokio::test]
async fn missing_subscription_owner_is_terminal_error_not_empty_success() {
    let f = Fixture::new();
    let supervisor = Arc::new(f.supervisor().await);
    let id = Uuid::new_v4();
    let inc = Uuid::new_v4();
    let (subscription, event, keep) = observe_local(
        supervisor,
        VesselEventSubscription {
            session_id: id,
            incarnation: inc,
            after: 99,
        },
    )
    .await;
    assert!(!keep);
    assert!(event.error.is_some());
    assert_eq!(event.session_id, id);
    assert_eq!(subscription.after, 99);
    assert_eq!(event.incarnation, inc);
}
