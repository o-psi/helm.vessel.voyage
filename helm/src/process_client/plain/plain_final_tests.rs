use super::*;
#[path = "../../dispatch_fixture_final_tests.rs"]
mod fixture;
use fixture::*;
use serde_json::json;
use voyage_protocol::vessel::VesselCommand as V;

#[tokio::test]
async fn plain_commands_translate_decisions_and_reject_stale_answers() {
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    let r = Uuid::new_v4();
    let d = Uuid::new_v4();
    for (line, kind, response) in [
        (format!("/approve {d}"), "approval", json!("approved")),
        (format!("/deny {d}"), "approval", json!("denied")),
        (
            format!("/answer {d} two words"),
            "question",
            json!({"status":"custom","answer":"two words"}),
        ),
        ("/cancel".into(), "approval", json!(null)),
    ] {
        let op = if line == "/cancel" {
            "cancel"
        } else {
            "respond"
        };
        let mut expected =
            json!({"op":op,"session_id":s,"incarnation":i,"expected_revision":5,"run_id":r});
        if op == "respond" {
            expected["decision_id"] = json!(d);
            expected["response"] = response;
        }
        let peer = Peer::new(vec![
            (wire(V::Inspect { session_id: s }), info(s, i)),
            (expected, json!({"status":"applied"})),
        ])
        .await;
        let connection = session::Connection::open(&peer.client, s).await.unwrap();
        let snapshot = json!({"revision":5,"run":{"run_id":r},"decisions":[{"decision_id":d,"run_id":r,"request":{"kind":kind}}]});
        command(&connection, &snapshot, &line).await.unwrap();
        for invalid in [
            "/unknown".to_owned(),
            "/approve invalid".into(),
            format!("/answer {d}"),
            format!("/approve {}", Uuid::new_v4()),
        ] {
            assert!(command(&connection, &snapshot, &invalid).await.is_err());
        }
        drop(connection);
        peer.finish().await;
    }
}

#[tokio::test]
async fn plain_admission_steers_active_runs_and_checks_receipts() {
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    let r = Uuid::new_v4();
    for state in [
        "completed",
        "running",
        "accepted",
        "awaiting_decision",
        "cancel_requested",
    ] {
        for receipt in [
            json!({"run_id":r}),
            json!({"record":{"run_id":r}}),
            json!({"status":"rejected"}),
            json!({"status":"accepted"}),
        ] {
            let op = if state == "completed" {
                "submit"
            } else {
                "steer"
            };
            let peer = Peer::new(vec![
                (wire(V::Inspect { session_id: s }), info(s, i)),
                (
                    json!({"op":op,"session_id":s,"expected_revision":5,"prompt":"hello"}),
                    receipt.clone(),
                ),
            ])
            .await;
            let connection = session::Connection::open(&peer.client, s).await.unwrap();
            let snapshot = json!({"revision":5,"run":{"run_id":r,"state":state}});
            assert!(connection.send(&snapshot, " ".into()).await.is_err());
            assert!(connection.send(&snapshot, "x".repeat(65537)).await.is_err());
            assert!(connection.send(&json!({}), "hello".into()).await.is_err());
            let result = connection.send(&snapshot, "hello".into()).await;
            if receipt.get("run_id").is_some() || receipt.get("record").is_some() {
                assert_eq!(result.unwrap(), r);
            } else {
                assert!(result.is_err());
            }
            drop(connection);
            peer.finish().await;
        }
    }
}

#[tokio::test]
async fn output_drain_validates_utf8_byte_cursors_and_bounded_batches() {
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    let r = Uuid::new_v4();
    let mut script = vec![(wire(V::Inspect { session_id: s }), info(s, i))];
    for offset in (0..16).step_by(2) {
        script.push((json!({"op":"run_output","run_id":r,"offset":offset,"limit":65536}),json!({"run_id":r,"offset":offset,"data":"é","next_offset":offset+2,"has_more":true,"state":"running"})));
    }
    script.push((json!({"op":"run_output","offset":16}),json!({"run_id":r,"offset":16,"data":"","next_offset":16,"has_more":false,"state":"completed"})));
    let peer = Peer::new(script).await;
    let connection = session::Connection::open(&peer.client, s).await.unwrap();
    let mut offset = 0;
    assert_eq!(
        output::drain(&connection, r, &mut offset).await.unwrap(),
        "running"
    );
    assert_eq!(offset, 16);
    assert_eq!(
        output::drain(&connection, r, &mut offset).await.unwrap(),
        "completed"
    );
    drop(connection);
    peer.finish().await;
    for invalid in [
        json!({}),
        json!({"run_id":r,"offset":0}),
        json!({"run_id":r,"offset":0,"data":"a"}),
        json!({"run_id":r,"offset":0,"data":"é","next_offset":1}),
        json!({"run_id":r,"offset":0,"data":"","next_offset":0}),
    ] {
        let peer = Peer::new(vec![
            (wire(V::Inspect { session_id: s }), info(s, i)),
            (json!({"op":"run_output"}), invalid),
        ])
        .await;
        let connection = session::Connection::open(&peer.client, s).await.unwrap();
        assert!(output::drain(&connection, r, &mut 0).await.is_err());
        drop(connection);
        peer.finish().await;
    }
}

#[tokio::test]
async fn public_follow_frontends_observe_completion_failure_and_paged_output() {
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    let r = Uuid::new_v4();
    let d = Uuid::new_v4();
    for machine in [false, true] {
        for state in ["completed", "failed", "cancelled"] {
            let script = vec![
                (wire(V::Inspect { session_id: s }), info(s, i)),
                (json!({"op":"snapshot"}), json!({"observation_cursor":12})),
                (wire(V::Capabilities), json!({"features":[]})),
                (
                    json!({"op":"run_output","offset":0}),
                    json!({"run_id":r,"offset":0,"data":"é","next_offset":2,"has_more":true,"state":"running"}),
                ),
                // JSON emits decisions between pages; plain drains its bounded batch first.
            ];
            let mut script = script;
            if machine {
                script.push((
                    json!({"op":"decisions"}),
                    json!([{"run_id":r,"decision_id":d,"request":{"kind":"question"}}]),
                ));
            }
            script.push((json!({"op":"run_output","offset":2}),json!({"run_id":r,"offset":2,"data":"","next_offset":2,"has_more":false,"state":state})));
            let peer = Peer::new(script).await;
            let result = if machine {
                follow_json(&peer.client, s, r).await
            } else {
                follow(&peer.client, s, r).await
            };
            assert_eq!(result.is_ok(), state == "completed");
            peer.finish().await;
        }
    }
}

#[tokio::test]
async fn session_submit_snapshots_revision_before_admitting() {
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    let c = Uuid::new_v4();
    let r = Uuid::new_v4();
    let peer = Peer::new(vec![
        (wire(V::Inspect { session_id: s }), info(s, i)),
        (
            json!({"op":"snapshot"}),
            json!({"revision":19,"run":{"state":"completed","run_id":r}}),
        ),
        (
            json!({"op":"submit","command_id":c,"expected_revision":19,"prompt":"new turn"}),
            json!({"run_id":r}),
        ),
    ])
    .await;
    let connection = session::Connection::open(&peer.client, s).await.unwrap();
    assert_eq!(
        connection.submit(Some(c), "new turn".into()).await.unwrap(),
        r
    );
    drop(connection);
    peer.finish().await;
}
