use super::*;
use crate::process_client::loopback_tests::{Peer, frame, response, send};
use serde_json::json;
use voyage_protocol::vessel::VesselEventSubscription;

#[tokio::test]
async fn commands_correlate_out_of_order_and_late_receipts_never_replay() {
    let mut peer = Peer::open().await;
    let a = peer.client.clone();
    let b = peer.client.clone();
    let first = tokio::spawn(async move { a.request(VesselCommand::Capabilities).await });
    let (id1, _) = peer.command().await;
    let second = tokio::spawn(async move { b.request(VesselCommand::Capabilities).await });
    let (id2, _) = peer.command().await;
    assert_ne!(id1, id2);
    peer.reply(id2, json!({"order":2})).await;
    peer.reply(id1, json!({"order":1})).await;
    assert_eq!(first.await.unwrap().unwrap()["order"], 1);
    assert_eq!(second.await.unwrap().unwrap()["order"], 2);
    peer.reply(id1, json!({"late":true})).await;
    let c = peer.client.clone();
    let third = tokio::spawn(async move { c.request(VesselCommand::Capabilities).await });
    let (id3, _) = peer.command().await;
    assert_ne!(id3, id1);
    peer.reply(id3, json!(3)).await;
    assert_eq!(third.await.unwrap().unwrap(), 3);
}

#[tokio::test]
async fn malformed_frames_fence_socket_and_fail_all_pending_without_retry() {
    for message in [
        Message::Text("not json".into()),
        Message::Binary(vec![0, 1].into()),
        server_message(&ServerFrame::Hello {
            protocol: VESSEL_API_VERSION,
            socket_id: Uuid::new_v4(),
            vessel_id: Uuid::new_v4(),
        })
        .unwrap(),
        server_message(&ServerFrame::Reply {
            request_id: Uuid::new_v4(),
            response: VesselResponse {
                protocol: VESSEL_API_VERSION + 1,
                ..response(json!(null))
            },
        })
        .unwrap(),
    ] {
        let mut peer = Peer::open().await;
        let mut observed = peer.client.connection_state();
        let prior = *observed.borrow_and_update();
        let client = peer.client.clone();
        let pending =
            tokio::spawn(async move { client.request(VesselCommand::Capabilities).await });
        peer.command().await;
        peer.socket.send(message).await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(3), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("outcome unknown"));
        tokio::time::timeout(Duration::from_secs(3), observed.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observed.borrow().socket_id, None);
        assert_eq!(observed.borrow().loss_generation, prior.loss_generation + 1);
    }
}

#[tokio::test]
async fn reverse_requests_require_live_handler_and_responder_lifetime() {
    let mut peer = Peer::open().await;
    let request = || ReverseRequest::BrowserWork {
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
    };
    let absent = Uuid::new_v4();
    send(
        &mut peer.socket,
        ServerFrame::ReverseRequest {
            request_id: absent,
            request: request(),
        },
    )
    .await;
    assert!(
        matches!(frame(&mut peer.socket).await, ClientFrame::ReverseReply { request_id, reply: ReverseReply::Unavailable } if request_id == absent)
    );
    let mut incoming = peer.client.reverse_requests().await.unwrap();
    assert!(peer.client.reverse_requests().await.is_err());
    for accepted in [true, false] {
        let id = Uuid::new_v4();
        send(
            &mut peer.socket,
            ServerFrame::ReverseRequest {
                request_id: id,
                request: request(),
            },
        )
        .await;
        let item = tokio::time::timeout(Duration::from_secs(3), incoming.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            item.socket_id,
            peer.client.connection_state().borrow().socket_id.unwrap()
        );
        assert_eq!(item.request_id, id);
        assert_eq!(item.reply.request_id(), id);
        if accepted {
            item.reply.respond(ReverseReply::Accepted).unwrap();
        } else {
            drop(item);
        }
        let ClientFrame::ReverseReply { request_id, reply } = frame(&mut peer.socket).await else {
            panic!("reverse reply")
        };
        assert_eq!(request_id, id);
        assert_eq!(matches!(reply, ReverseReply::Accepted), accepted);
    }
    drop(incoming);
    assert!(peer.client.reverse_requests().await.is_ok());
}

#[tokio::test]
async fn duplicate_reverse_identity_terminates_socket_and_expired_response_is_refused() {
    let mut peer = Peer::open().await;
    let mut incoming = peer.client.reverse_requests().await.unwrap();
    let id = Uuid::new_v4();
    let request = ReverseRequest::BrowserWork {
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
    };
    send(
        &mut peer.socket,
        ServerFrame::ReverseRequest {
            request_id: id,
            request: request.clone(),
        },
    )
    .await;
    let item = incoming.recv().await.unwrap();
    let mut state = peer.client.connection_state();
    state.borrow_and_update();
    send(
        &mut peer.socket,
        ServerFrame::ReverseRequest {
            request_id: id,
            request,
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(3), state.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.borrow().socket_id, None);
    assert!(item.reply.respond(ReverseReply::Accepted).is_err());
}

#[tokio::test]
async fn event_stream_checks_identity_and_drop_unsubscribes_without_losing_commands() {
    let mut peer = Peer::open().await;
    let session = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let client = peer.client.clone();
    let task = tokio::spawn(async move {
        client
            .events(vec![VesselEventSubscription {
                session_id: session,
                incarnation,
                after: 4,
            }])
            .await
    });
    let ClientFrame::Subscribe {
        request_id,
        request,
    } = frame(&mut peer.socket).await
    else {
        panic!("subscribe")
    };
    assert_eq!(request.subscriptions[0].after, 4);
    send(&mut peer.socket, ServerFrame::Subscribed { request_id }).await;
    let mut events = task.await.unwrap().unwrap();
    send(
        &mut peer.socket,
        ServerFrame::Event {
            subscription_id: request_id,
            event: VesselEvent {
                protocol: VESSEL_API_VERSION,
                session_id: session,
                incarnation,
                result: json!({"sequence":5,"kind":"fixture"}),
                error: None,
                outcome_unknown: false,
            },
        },
    )
    .await;
    assert_eq!(events.next().await.unwrap().unwrap().result["sequence"], 5);
    drop(events);
    assert!(
        matches!(frame(&mut peer.socket).await, ClientFrame::Unsubscribe { subscription_id } if subscription_id == request_id)
    );
    assert!(peer.client.connection_state().borrow().socket_id.is_some());
}

#[tokio::test]
async fn subscription_refusal_and_invalid_admission_are_explicit() {
    let mut peer = Peer::open().await;
    let client = peer.client.clone();
    let task = tokio::spawn(async move {
        client
            .events(vec![VesselEventSubscription {
                session_id: Uuid::new_v4(),
                incarnation: Uuid::new_v4(),
                after: 0,
            }])
            .await
    });
    let ClientFrame::Subscribe { request_id, .. } = frame(&mut peer.socket).await else {
        panic!("subscribe")
    };
    peer.reply(request_id, json!(null)).await;
    assert!(task.await.unwrap().is_err());
    let socket = slot(Uuid::new_v4(), 0);
    for subscriptions in [
        vec![],
        vec![VesselEventSubscription {
            session_id: Uuid::nil(),
            incarnation: Uuid::new_v4(),
            after: 0,
        }],
        vec![VesselEventSubscription {
            session_id: Uuid::new_v4(),
            incarnation: Uuid::nil(),
            after: 0,
        }],
    ] {
        assert!(
            socket
                .events(
                    &peer.client,
                    VesselEventRequest {
                        protocol: VESSEL_API_VERSION,
                        subscriptions
                    }
                )
                .await
                .is_err()
        );
    }
    let subscription = VesselEventSubscription {
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        after: 0,
    };
    assert!(
        socket
            .events(
                &peer.client,
                VesselEventRequest {
                    protocol: VESSEL_API_VERSION,
                    subscriptions: vec![subscription.clone(), subscription]
                }
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn slot_pool_generation_and_disconnect_are_monotonic() {
    let id = Uuid::new_v4();
    let a = slot(id, 3);
    let b = slot(id, 3);
    let c = slot(id, 4);
    assert!(Arc::ptr_eq(&a, &b));
    assert!(!Arc::ptr_eq(&a, &c));
    a.state.send_modify(|s| {
        s.socket_id = Some(Uuid::new_v4());
        s.loss_generation = u64::MAX;
    });
    a.disconnect();
    a.disconnect();
    assert_eq!(
        *a.state().borrow(),
        ConnectionState {
            socket_id: None,
            loss_generation: u64::MAX
        }
    );
    assert!(a.stop.is_cancelled());
    assert!(format!("{a:?}").contains("DuplexConnection"));
}

fn server_message(frame: &ServerFrame) -> Result<Message> {
    Ok(Message::Text(serde_json::to_string(frame)?.into()))
}
