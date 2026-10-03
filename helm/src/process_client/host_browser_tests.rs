use super::*;
fn adapter() -> Adapter {
    let (status, _) = watch::channel(Status {
        summary: String::new(),
        launcher: None,
        finished: false,
    });
    Adapter {
        client: Client::local(
            std::env::temp_dir().join(format!("missing-host-view-test-{}", Uuid::new_v4())),
        ),
        session: Uuid::new_v4(),
        owner: Arc::new(Mutex::new((Uuid::new_v4(), 1))),
        preparation: Arc::new(Preparation::default()),
        socket: Uuid::new_v4(),
        origin: "http://127.0.0.1:12345".into(),
        host: "127.0.0.1:12345".into(),
        launch: Mutex::new(Some("one-use".into())),
        secret: "Bearer test-secret".into(),
        csrf: "test-csrf".into(),
        gate: tokio::sync::Mutex::new(()),
        binding: Mutex::new(None),
        stop: CancellationToken::new(),
        status,
    }
}
fn headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    for (key, value) in [
        ("host", "127.0.0.1:12345"),
        ("origin", "http://127.0.0.1:12345"),
        ("authorization", "Bearer test-secret"),
        ("x-helm-csrf", "test-csrf"),
    ] {
        h.insert(
            axum::http::HeaderName::from_static(key),
            value.parse().unwrap(),
        );
    }
    h
}

#[tokio::test]
async fn http_handlers_enforce_authentication_body_and_liveness() {
    let a = Arc::new(adapter());
    assert_eq!(
        alive(State(a.clone()), headers()).await,
        StatusCode::NO_CONTENT
    );
    for key in ["host", "origin", "authorization", "x-helm-csrf"] {
        let mut h = headers();
        h.remove(key);
        assert_eq!(
            alive(State(a.clone()), h.clone()).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            operation(State(a.clone()), h, Bytes::from_static(b"{}"))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    for body in [
        b"".as_slice(),
        b"{",
        b"{}",
        b"null",
        b"{\"action\":\"unknown\"}",
    ] {
        assert_eq!(
            operation(State(a.clone()), headers(), Bytes::copy_from_slice(body))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        page(State(a.clone()), headers()).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        page(State(a.clone()), HeaderMap::new()).await.status(),
        StatusCode::FORBIDDEN
    );
    a.stop.cancel();
    assert_eq!(
        alive(State(a.clone()), headers()).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        bootstrap(State(a.clone()), headers(), Bytes::from_static(b"one-use"))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(*a.launch.lock().unwrap(), Some("one-use".into()));
    assert_eq!(
        operation(
            State(a),
            headers(),
            Bytes::from_static(b"{\"action\":\"status\"}")
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn bootstrap_returns_context_without_consuming_bad_tokens() {
    let a = Arc::new(adapter());
    for token in ["", "one-us", "one-use-extra"] {
        assert_eq!(
            bootstrap(
                State(a.clone()),
                headers(),
                Bytes::copy_from_slice(token.as_bytes())
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
    let response = bootstrap(State(a.clone()), headers(), Bytes::from_static(b"one-use")).await;
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["authorization"], a.secret);
    assert_eq!(value["csrf"], a.csrf);
    assert_eq!(value["revision"], 1);
    assert_eq!(value["incarnation"], json!(a.owner.lock().unwrap().0));
    assert!(a.launch.lock().unwrap().is_none());
}

#[tokio::test]
async fn exchange_tracks_only_valid_current_owner_bindings_and_status() {
    use crate::process_client::loopback_tests::Peer;
    let mut peer = Peer::open().await;
    let mut a = adapter();
    a.client = peer.client.clone();
    a.socket = a.client.connection_state().borrow().socket_id.unwrap();
    let a = Arc::new(a);
    for (mode, running, expected) in [
        ("human", true, "running; control: human"),
        ("private", false, "not running; control: private"),
        ("agent", true, "running; control: agent"),
        ("invalid", false, "not running; control: unknown"),
    ] {
        let cloned = a.clone();
        let task = tokio::spawn(async move { cloned.exchange(Op::Status {}).await });
        let (id, _) = peer.command().await;
        let owner = a.owner.lock().unwrap().0;
        peer.voyage_reply(
            id,
            a.session,
            owner,
            json!({"status":{"mode":mode,"running":running,"binding":{}}}),
        )
        .await;
        task.await.unwrap().unwrap();
        assert_eq!(
            a.status.borrow().summary,
            format!("Executing-host browser: {expected}")
        );
        assert!(a.binding.lock().unwrap().is_none());
        assert!(!a.stop.is_cancelled());
    }
}

#[tokio::test]
async fn start_waits_for_gate_but_status_can_interrupt() {
    let a = Arc::new(adapter());
    let owner = a.owner.lock().unwrap().0;
    let start = serde_json::to_vec(&Op::Start {
        command_id: Uuid::new_v4(),
        incarnation: owner,
        expected_revision: 1,
    })
    .unwrap();
    let _guard = a.gate.lock().await;
    assert_eq!(
        operation(State(a.clone()), headers(), start.into())
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert!(
        !a.stop.is_cancelled(),
        "busy gate must not poison the viewer"
    );
    assert_eq!(
        operation(
            State(a.clone()),
            headers(),
            Bytes::from_static(b"{\"action\":\"status\"}")
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert!(
        a.stop.is_cancelled(),
        "status bypasses gate and observes lost socket"
    );
}

#[tokio::test]
async fn preparation_refreshes_fence_once_without_changing_start_intent() {
    use crate::process_client::loopback_tests::Peer;
    use voyage_protocol::vessel::{VesselCommand, VoyageCommand, VoyageRequest};
    for repeated in [false, true] {
        let mut peer = Peer::open().await;
        let mut a = adapter();
        a.client = peer.client.clone();
        a.socket = a.client.connection_state().borrow().socket_id.unwrap();
        let a = Arc::new(a);
        let old_owner = a.owner.lock().unwrap().0;
        let new_owner = Uuid::new_v4();
        let command_id = Uuid::new_v4();
        let cloned = a.clone();
        let task = tokio::spawn(async move {
            cloned
                .exchange(Op::Start {
                    command_id,
                    incarnation: old_owner,
                    expected_revision: 1,
                })
                .await
        });
        let (id, _) = peer.command().await;
        peer.voyage_reply(
            id,
            a.session,
            new_owner,
            json!({"status":"prepared","not_dispatched":true}),
        )
        .await;
        let (id, command) = peer.command().await;
        assert!(
            matches!(command, VesselCommand::Voyage(VoyageRequest { incarnation: Some(i), command: VoyageCommand::Snapshot, .. }) if i == new_owner)
        );
        peer.voyage_reply(id, a.session, new_owner, json!({"revision":42}))
            .await;
        let (id, command) = peer.command().await;
        assert!(
            matches!(command, VesselCommand::Voyage(VoyageRequest { incarnation: Some(i), command: VoyageCommand::HostBrowser { operation: Op::Start { command_id: cid, incarnation, expected_revision: 42 } }, .. }) if i == new_owner && incarnation == new_owner && cid == command_id)
        );
        let result = if repeated {
            json!({"status":"prepared","not_dispatched":true})
        } else {
            json!({"status":{"running":true,"mode":"human"}})
        };
        peer.voyage_reply(id, a.session, new_owner, result).await;
        assert_eq!(task.await.unwrap().is_err(), repeated);
        assert_eq!(*a.owner.lock().unwrap(), (new_owner, 42));
        assert_eq!(a.stop.is_cancelled(), repeated);
    }
}

fn current_binding(a: &Adapter) -> HostBrowserBinding {
    HostBrowserBinding {
        incarnation: a.owner.lock().unwrap().0,
        browser_id: Uuid::new_v4(),
        attachment_id: Uuid::new_v4(),
        tab_id: Uuid::new_v4(),
        document_epoch: 1,
        viewport_epoch: 1,
        controller_epoch: 1,
        capture_epoch: 1,
    }
}
#[tokio::test]
async fn unknown_read_only_mirror_failure_does_not_authorize_replay_or_poison_unrelated_controls() {
    use crate::process_client::loopback_tests::{Peer, response, send};
    use voyage_protocol::{duplex::ServerFrame, vessel::*};
    let mut peer = Peer::open().await;
    let mut a = adapter();
    a.client = peer.client.clone();
    a.socket = a.client.connection_state().borrow().socket_id.unwrap();
    let binding = current_binding(&a);
    let a = Arc::new(a);
    let worker = a.clone();
    let task = tokio::spawn(async move { worker.exchange(Op::Mirror { binding, since: 0 }).await });
    let (id, _) = peer.command().await;
    send(
        &mut peer.socket,
        ServerFrame::Reply {
            request_id: id,
            response: VesselResponse {
                error: Some("synthetic read refusal".into()),
                ..response(json!(null))
            },
        },
    )
    .await;
    assert!(task.await.unwrap().is_err());
    assert!(!a.stop.is_cancelled());
    assert!(a.binding.lock().unwrap().is_none());
}
#[tokio::test]
async fn unknown_control_failure_poisoning_prevents_a_second_effect_dispatch() {
    use crate::process_client::loopback_tests::{Peer, response, send};
    use voyage_protocol::{duplex::ServerFrame, vessel::*};
    let mut peer = Peer::open().await;
    let mut a = adapter();
    a.client = peer.client.clone();
    a.socket = a.client.connection_state().borrow().socket_id.unwrap();
    let binding = current_binding(&a);
    let a = Arc::new(a);
    let worker = a.clone();
    let task = tokio::spawn(async move {
        worker
            .exchange(Op::Close {
                command_id: Uuid::new_v4(),
                binding,
            })
            .await
    });
    let (id, _) = peer.command().await;
    send(
        &mut peer.socket,
        ServerFrame::Reply {
            request_id: id,
            response: VesselResponse {
                error: Some("uncertain synthetic effect".into()),
                ..response(json!(null))
            },
        },
    )
    .await;
    assert!(task.await.unwrap().is_err());
    assert!(a.stop.is_cancelled());
    assert!(a.exchange(Op::Status {}).await.is_err());
}
#[tokio::test]
async fn stale_binding_refusal_occurs_before_transport_and_preserves_current_owner() {
    let a = adapter();
    let owner = *a.owner.lock().unwrap();
    let mut binding = current_binding(&a);
    binding.incarnation = Uuid::new_v4();
    assert!(a.exchange(Op::Mirror { binding, since: 0 }).await.is_err());
    assert_eq!(*a.owner.lock().unwrap(), owner);
    assert!(!a.stop.is_cancelled());
    assert!(
        a.exchange(Op::Start {
            command_id: Uuid::nil(),
            incarnation: owner.0,
            expected_revision: owner.1
        })
        .await
        .is_err()
    );
    assert!(!a.stop.is_cancelled());
}
#[tokio::test]
async fn viewer_context_bootstrap_is_one_use_even_after_terminal_output_flags_change() {
    let a = Arc::new(adapter());
    let mut h = headers();
    h.insert("x-extra-private", "fixture".parse().unwrap());
    let first = bootstrap(State(a.clone()), h.clone(), Bytes::from_static(b"one-use")).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert!(a.launch.lock().unwrap().is_none());
    assert_eq!(
        bootstrap(State(a.clone()), h, Bytes::from_static(b"one-use"))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    a.stop.cancel();
    assert_eq!(alive(State(a), headers()).await, StatusCode::FORBIDDEN);
}
