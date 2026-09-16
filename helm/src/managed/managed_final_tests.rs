use super::*;
use helm::process_client::transport::Client;
#[path = "../dispatch_fixture_final_tests.rs"]
// Each consumer gets an independent scripted fixture module.
#[allow(clippy::duplicate_mod)]
mod fixture;
use fixture::*;
use voyage_protocol::vessel::VesselCommand as V;

#[tokio::test]
async fn managed_submit_cancel_and_recover_dispatch_without_runtime() {
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    let r = Uuid::new_v4();
    let c = Uuid::new_v4();
    for status in ["accepted", "duplicate", "rejected"] {
        let receipt = json!({"status":status,"run_id":r,"duplicate":true});
        let peer=Peer::new(vec![(wire(V::Capabilities),json!({})),(wire(V::Catalogue),json!([info(s,i)])),(json!({"op":"snapshot","session_id":s}),json!({"model":"synthetic","revision":7})),(json!({"op":"submit","session_id":s,"command_id":c,"expected_revision":7,"expires_at_ms":900,"prompt":"hello world"}),receipt)]).await;
        let args = Args {
            directory: peer.root.path().into(),
            json: true,
            command: ManagedCommand::Submit {
                session: s,
                expected_revision: 7,
                command_id: Some(c),
                expires_at_ms: Some(900),
                prompt: vec!["hello".into(), "world".into()],
            },
        };
        let result = commands::run(args, None, None, false).await;
        assert_eq!(result.is_err(), status == "rejected");
        peer.finish().await;
    }
    let peer = Peer::new(vec![
        (wire(V::Capabilities), json!({})),
        (wire(V::Catalogue), json!([info(s, i)])),
        (json!({"op":"snapshot"}), json!({"revision":7})),
        (
            json!({"op":"cancel","session_id":s,"incarnation":i,"run_id":r,"expected_revision":7}),
            json!({"status":"accepted"}),
        ),
    ])
    .await;
    commands::run(
        Args {
            directory: peer.root.path().into(),
            json: false,
            command: ManagedCommand::Cancel { session: s, run: r },
        },
        None,
        None,
        false,
    )
    .await
    .unwrap();
    peer.finish().await;
    let peer=Peer::new(vec![(wire(V::Capabilities),json!({})),(wire(V::Catalogue),json!([info(s,i)])),(wire(V::Catalogue),json!([info(s,i)])),(json!({"op":"recover","session_id":s,"incarnation":i,"acknowledge_cleanup":r,"reconcile_tools":r,"expected_revision":7,"acknowledge_resources":[c]}),json!({"status":"recovered"}))]).await;
    commands::run(
        Args {
            directory: peer.root.path().into(),
            json: true,
            command: ManagedCommand::Recover {
                session: s,
                acknowledge_cleanup: Some(r),
                reconcile_tools: Some(r),
                expected_revision: Some(7),
                acknowledge_resources: vec![c],
            },
        },
        None,
        None,
        false,
    )
    .await
    .unwrap();
    peer.finish().await;
}

#[tokio::test]
async fn managed_list_orders_pages_and_filters_after_cursor() {
    let ids = [Uuid::from_u128(3), Uuid::from_u128(1), Uuid::from_u128(2)];
    let i = Uuid::new_v4();
    for (after, limit, want, next) in [
        (None, 2, vec![ids[1], ids[2]], Some(ids[2])),
        (Some(ids[1]), 100, vec![ids[2], ids[0]], None),
        (Some(ids[0]), 1, vec![], None),
    ] {
        let peer = Peer::new(vec![(
            wire(V::Catalogue),
            json!(ids.iter().map(|s| info(*s, i)).collect::<Vec<_>>()),
        )])
        .await;
        let page = catalogue::list(&peer.client, peer.root.path(), after, limit)
            .await
            .unwrap();
        assert_eq!(
            page["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| serde_json::from_value::<Uuid>(row["id"].clone()).unwrap())
                .collect::<Vec<_>>(),
            want
        );
        assert_eq!(page["next_after"], json!(next));
        peer.finish().await;
    }
    let root = tempfile::tempdir().unwrap();
    let client = Client::local(root.path().join("missing"));
    for limit in [0, 101] {
        assert!(
            catalogue::list(&client, root.path(), None, limit)
                .await
                .is_err()
        );
    }
    assert!(
        connection::connect(std::path::Path::new("relative"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn managed_owner_state_gate_and_unknown_session_do_not_start_services() {
    let s = Uuid::new_v4();
    let i = Uuid::new_v4();
    let root = tempfile::tempdir().unwrap();
    let client = Client::local(root.path().join("absent"));
    for state in [
        "live",
        "suspended",
        "unavailable",
        "starting",
        "cleanup_unconfirmed",
        "relinquished",
    ] {
        let mut value = info(s, i);
        value["state"] = json!(state);
        let result = connection::live(&client, serde_json::from_value(value).unwrap()).await;
        assert_eq!(result.is_ok(), matches!(state, "live" | "suspended"));
    }
    let mut value = info(s, i);
    value["state"] = json!("stopped");
    let peer = Peer::new(vec![(
        json!({"op":"restart","session_id":s,"incarnation":i}),
        info(s, i),
    )])
    .await;
    assert_eq!(
        connection::live(&peer.client, serde_json::from_value(value).unwrap())
            .await
            .unwrap()
            .session_id,
        s
    );
    peer.finish().await;
    let peer = Peer::new(vec![(wire(V::Catalogue), json!([]))]).await;
    assert!(
        connection::session(&peer.client, peer.root.path(), s, None, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("unknown managed session")
    );
    peer.finish().await;
}

#[tokio::test]
async fn managed_rejects_missing_configuration_and_negative_deadline_before_mutation() {
    let peer = Peer::new(vec![(wire(V::Capabilities), json!({}))]).await;
    assert!(
        commands::run(
            Args {
                directory: peer.root.path().into(),
                json: true,
                command: ManagedCommand::Create {
                    id: None,
                    name: None
                }
            },
            None,
            None,
            false
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("requires configuration")
    );
    peer.finish().await;
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    for model_override in [false, true] {
        let peer = Peer::new(vec![
            (wire(V::Capabilities), json!({})),
            (wire(V::Catalogue), json!([info(s, i)])),
            (json!({"op":"snapshot"}), json!({"model":"saved"})),
        ])
        .await;
        assert!(
            commands::run(
                Args {
                    directory: peer.root.path().into(),
                    json: true,
                    command: ManagedCommand::Submit {
                        session: s,
                        expected_revision: 1,
                        command_id: None,
                        expires_at_ms: Some(-1),
                        prompt: vec!["hello".into()]
                    }
                },
                None,
                None,
                model_override
            )
            .await
            .is_err()
        );
        peer.finish().await;
    }
}
