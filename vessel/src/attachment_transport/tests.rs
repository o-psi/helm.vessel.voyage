#![cfg(any(unix, windows))]
use super::*;
#[test]
fn direction_rejects_server_observations_and_client_commands() {
    let id = Uuid::new_v4();
    assert!(outbound(&Frame::Heartbeat { connection_id: id }, id, id, id).is_err());
    assert!(
        inbound(
            &Frame::ReplayRequest {
                connection_id: id,
                request_id: id,
                session_id: id,
                after: Default::default(),
                limit: 1
            },
            id
        )
        .is_err()
    );
    assert!(
        inbound(
            &Frame::Heartbeat {
                connection_id: Uuid::new_v4()
            },
            id
        )
        .is_err()
    );
}
use crate::enrollment::EnrollmentStore;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message as ClientMessage, client::IntoClientRequest},
};
use voyage_protocol::enrollment::{ProofOperation, SignedChallenge, SigningKey};
struct Fixture {
    _dir: tempfile::TempDir,
    store: EnrollmentStore,
    key: SigningKey,
    machine: Uuid,
    api: AttachmentApi,
    rx: mpsc::Receiver<AuthenticatedFrame>,
    address: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
// Security/lifecycle fixtures use production deadlines. Only tests asserting a
// particular deadline shorten that deadline; native ACL/SQLite work must not
// inherit a150ms handshake or30ms authorization budget accidentally.
fn functional_limits() -> Limits {
    Limits {
        sockets: 2,
        machines: 1,
        queue: 2,
        ..Limits::default()
    }
}
impl Fixture {
    async fn new() -> Self {
        Self::new_at(now()).await
    }
    async fn new_at(seed_time: i64) -> Self {
        Self::new_at_with_limits(seed_time, functional_limits()).await
    }
    async fn new_with_limits(limits: Limits) -> Self {
        Self::new_at_with_limits(now(), limits).await
    }
    async fn new_at_with_limits(seed_time: i64, limits: Limits) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let origin = format!("http://{address}");
        let mut store =
            EnrollmentStore::open(&dir.path().join("authority"), &origin, true).unwrap();
        let key = SigningKey::generate().unwrap();
        let machine = Uuid::new_v4();
        let invitation = store.invite(60_000, seed_time).unwrap();
        let operation = ProofOperation::Enroll {
            machine_id: machine,
            transaction_id: Uuid::new_v4(),
            invitation_id: invitation.id,
            public_key: key.public_key(),
        };
        let challenge = store
            .challenge(operation, Some(&invitation.key), seed_time)
            .unwrap();
        let proof = SignedChallenge {
            signature: key.sign(&challenge).unwrap(),
            challenge,
            new_signature: None,
        };
        store
            .complete(&proof, Some(&invitation.key), seed_time)
            .unwrap();
        let enrollment = EnrollmentApi::new(
            EnrollmentStore::open(&dir.path().join("authority"), &origin, true).unwrap(),
            "fixture-operator-token-at-least-32-bytes",
        )
        .unwrap();
        let (api, rx) =
            AttachmentApi::with_limits(enrollment, Features::default(), limits).unwrap();
        let router = api.clone().router();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            _dir: dir,
            store,
            key,
            machine,
            api,
            rx,
            address,
            task,
        }
    }
    fn proof(&mut self) -> SignedChallenge {
        self.proof_at(now())
    }
    fn proof_at(&mut self, time: i64) -> SignedChallenge {
        let challenge = self
            .store
            .challenge(
                ProofOperation::Connect {
                    machine_id: self.machine,
                    epoch: 1,
                },
                None,
                time,
            )
            .unwrap();
        SignedChallenge {
            signature: self.key.sign(&challenge).unwrap(),
            challenge,
            new_signature: None,
        }
    }
    fn request(&self) -> tokio_tungstenite::tungstenite::http::Request<()> {
        let mut request = format!("ws://{}/v2/attachment", self.address)
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "origin",
            format!("http://{}", self.address).parse().unwrap(),
        );
        request
            .headers_mut()
            .insert("x-voyage-request", "2".parse().unwrap());
        request.headers_mut().insert(
            "sec-websocket-protocol",
            "voyage.attachment.v2".parse().unwrap(),
        );
        request
    }
    async fn socket(
        &self,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        connect_async(self.request()).await.unwrap().0
    }
    async fn authenticate(
        &self,
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        proof: SignedChallenge,
    ) -> Uuid {
        let frame = Frame::Authenticate {
            version: 2,
            proof,
            features: Features::default(),
        };
        socket
            .send(ClientMessage::Text(frame.encode().unwrap().into()))
            .await
            .unwrap();
        let message = socket.next().await.unwrap().unwrap();
        let Frame::Welcome {
            connection_id,
            lease_ms,
            ..
        } = Frame::decode(message.into_text().unwrap().as_bytes()).unwrap()
        else {
            panic!("welcome expected")
        };
        assert_eq!(lease_ms, self.api.limits.lease.as_millis() as u32);
        connection_id
    }
}
async fn closed(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    let result = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("socket must terminate");
    assert!(matches!(
        result,
        None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))
    ));
}
#[tokio::test]
async fn real_socket_proof_replay_replacement_queue_and_revocation() {
    let mut f = Fixture::new().await;
    let proof = f.proof();
    let mut one = f.socket().await;
    let first = f.authenticate(&mut one, proof.clone()).await;
    assert!(f.api.is_current(f.machine, first).await);
    one.send(ClientMessage::Text(
        Frame::Result {
            connection_id: first,
            command_id: Uuid::new_v4(),
            reply: voyage_protocol::stream::Reply::Accepted {},
        }
        .encode()
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    let observed = f.rx.recv().await.unwrap();
    assert_eq!(observed.connection_id, first);
    let mut replay = f.socket().await;
    replay
        .send(ClientMessage::Text(
            Frame::Authenticate {
                version: 2,
                proof,
                features: Features::default(),
            }
            .encode()
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    closed(&mut replay).await;
    let fresh = f.proof();
    let mut two = f.socket().await;
    let second = f.authenticate(&mut two, fresh).await;
    assert_ne!(first, second);
    closed(&mut one).await;
    assert!(
        !f.api
            .is_current(observed.machine_id, observed.connection_id)
            .await
    );
    two.send(ClientMessage::Text(
        Frame::Heartbeat {
            connection_id: second,
        }
        .encode()
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    assert!(matches!(
        Frame::decode(
            two.next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap()
                .as_bytes()
        )
        .unwrap(),
        Frame::Lease { .. }
    ));
    f.store.revoke(f.machine, 1, Uuid::new_v4(), now()).unwrap();
    closed(&mut two).await;
    assert!(!f.api.is_current(f.machine, second).await);
}
#[tokio::test]
async fn unauthenticated_capacity_deadline_wrong_first_and_oversize() {
    let f = Fixture::new_with_limits(Limits {
        auth: Duration::from_millis(150),
        ..functional_limits()
    })
    .await;
    let mut one = f.socket().await;
    let mut two = f.socket().await;
    // Both unauthenticated sockets count; HTTP upgrade capacity is bounded.
    let mut request = format!("ws://{}/v2/attachment", f.address)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", format!("http://{}", f.address).parse().unwrap());
    request
        .headers_mut()
        .insert("x-voyage-request", "2".parse().unwrap());
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "voyage.attachment.v2".parse().unwrap(),
    );
    assert!(connect_async(request).await.is_err());
    one.send(ClientMessage::Text(
        Frame::Heartbeat {
            connection_id: Uuid::new_v4(),
        }
        .encode()
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    closed(&mut one).await;
    closed(&mut two).await;
    let mut huge = f.socket().await;
    let _ = huge
        .send(ClientMessage::Text("x".repeat(MAX_FRAME_BYTES + 1).into()))
        .await;
    closed(&mut huge).await;
}
#[tokio::test]
async fn upgrade_headers_are_exact_and_no_query() {
    let f = Fixture::new().await;
    for (path, origin, protocol, marker) in [
        (
            "/v2/attachment?secret=x",
            format!("http://{}", f.address),
            "voyage.attachment.v2",
            "2",
        ),
        (
            "/v2/attachment",
            "https://evil.example".into(),
            "voyage.attachment.v2",
            "2",
        ),
        (
            "/v2/attachment",
            format!("http://{}", f.address),
            "voyage.attachment.v2, other",
            "2",
        ),
        (
            "/v2/attachment",
            format!("http://{}", f.address),
            "voyage.attachment.v2",
            "1",
        ),
    ] {
        let mut request = format!("ws://{}{path}", f.address)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        request
            .headers_mut()
            .insert("x-voyage-request", marker.parse().unwrap());
        request
            .headers_mut()
            .insert("sec-websocket-protocol", protocol.parse().unwrap());
        assert!(connect_async(request).await.is_err());
    }
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
#[tokio::test]
async fn lost_welcome_requires_fresh_proof_and_idle_closes() {
    let mut f = Fixture::new_with_limits(Limits {
        lease: Duration::from_millis(1000),
        ..functional_limits()
    })
    .await;
    let proof = f.proof();
    let mut lost = f.socket().await;
    lost.send(ClientMessage::Text(
        Frame::Authenticate {
            version: 2,
            proof: proof.clone(),
            features: Features::default(),
        }
        .encode()
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if !f.api.registry.lock().await.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(lost);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !f.api.registry.lock().await.is_empty() || f.api.sockets.available_permits() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("lost socket must release its generation and admission slot");
    let mut retry = f.socket().await;
    retry
        .send(ClientMessage::Text(
            Frame::Authenticate {
                version: 2,
                proof,
                features: Features::default(),
            }
            .encode()
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    closed(&mut retry).await;
    let proof = f.proof();
    let mut fresh = f.socket().await;
    let id = f.authenticate(&mut fresh, proof).await;
    closed(&mut fresh).await;
    assert!(!f.api.is_current(f.machine, id).await);
}
#[tokio::test]
async fn malformed_binary_expired_and_unnegotiated_frames_close() {
    let mut f = Fixture::new().await;
    for text in [
        "{}".to_string(),
        "{\"type\":\"authenticate\",\"version\":1}".into(),
    ] {
        let mut socket = f.socket().await;
        socket.send(ClientMessage::Text(text.into())).await.unwrap();
        closed(&mut socket).await;
    }
    let mut binary = f.socket().await;
    binary
        .send(ClientMessage::Binary(vec![0; 8].into()))
        .await
        .unwrap();
    closed(&mut binary).await;
    let mut proof = f.proof();
    proof.challenge.expires_at_ms = now() - 1;
    proof.signature = f.key.sign(&proof.challenge).unwrap();
    let mut expired = f.socket().await;
    expired
        .send(ClientMessage::Text(
            Frame::Authenticate {
                version: 2,
                proof,
                features: Features::default(),
            }
            .encode()
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    closed(&mut expired).await;
    let proof = f.proof();
    let mut socket = f.socket().await;
    let id = f.authenticate(&mut socket, proof).await;
    let event = Frame::Event {
        connection_id: id,
        session_id: Uuid::new_v4(),
        event: voyage_protocol::events::SequencedEvent {
            cursor: voyage_protocol::events::EventCursor::new(1).unwrap(),
            run_id: Uuid::new_v4(),
            event: voyage_protocol::events::RunEvent::Running {},
        },
    };
    socket
        .send(ClientMessage::Text(event.encode().unwrap().into()))
        .await
        .unwrap();
    closed(&mut socket).await;
    assert!(f.rx.try_recv().is_err());
}
#[tokio::test]
async fn bounded_observation_queue_closes_without_execution() {
    let mut f = Fixture::new().await;
    let proof = f.proof();
    let mut socket = f.socket().await;
    let id = f.authenticate(&mut socket, proof).await;
    for _ in 0..3 {
        socket
            .send(ClientMessage::Text(
                Frame::Result {
                    connection_id: id,
                    command_id: Uuid::new_v4(),
                    reply: voyage_protocol::stream::Reply::Accepted {},
                }
                .encode()
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
    }
    closed(&mut socket).await;
    assert_eq!(f.rx.len(), 2);
    while let Ok(observation) = f.rx.try_recv() {
        assert!(
            !f.api
                .is_current(observation.machine_id, observation.connection_id)
                .await
        );
    }
}
#[tokio::test]
async fn outbound_owner_machine_and_connection_are_bound() {
    let mut f = Fixture::new().await;
    let proof = f.proof();
    let mut socket = f.socket().await;
    let id = f.authenticate(&mut socket, proof).await;
    let command = voyage_protocol::attachment::Command {
        version: 2,
        connection_id: id,
        machine_id: f.machine,
        principal_id: f.store.owner_id(),
        command_id: Uuid::new_v4(),
        expires_at_ms: now() + 10_000,
        operation: voyage_protocol::attachment::Operation::List {
            after: None,
            limit: 1,
        },
    };
    for field in [0, 1, 2] {
        let mut bad = command.clone();
        match field {
            0 => bad.connection_id = Uuid::new_v4(),
            1 => bad.machine_id = Uuid::new_v4(),
            _ => bad.principal_id = Uuid::new_v4(),
        };
        assert!(
            f.api
                .send(f.machine, Frame::Command { command: bad })
                .await
                .is_err()
        );
    }
    f.api
        .send(
            f.machine,
            Frame::Command {
                command: command.clone(),
            },
        )
        .await
        .unwrap();
    let received = socket.next().await.unwrap().unwrap();
    assert!(matches!(
        Frame::decode(received.into_text().unwrap().as_bytes()).unwrap(),
        Frame::Command { .. }
    ));
    socket
        .send(ClientMessage::Text(
            Frame::Command { command }.encode().unwrap().into(),
        ))
        .await
        .unwrap();
    closed(&mut socket).await;
    assert!(f.rx.try_recv().is_err());
}
#[tokio::test]
async fn genuinely_expired_server_mac_and_client_signature_are_rejected() {
    let seed = now() - 120_000;
    let mut f = Fixture::new_at(seed).await;
    let proof = f.proof_at(seed);
    assert!(proof.challenge.expires_at_ms < now());
    let mut socket = f.socket().await;
    socket
        .send(ClientMessage::Text(
            Frame::Authenticate {
                version: 2,
                proof,
                features: Features::default(),
            }
            .encode()
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    closed(&mut socket).await;
    assert!(f.api.registry.lock().await.is_empty());
}
#[tokio::test]
async fn stalled_writes_have_deadlines_and_replacement_cancels_them() {
    let token = CancellationToken::new();
    let start = tokio::time::Instant::now();
    assert_eq!(
        bounded_write(
            &token,
            Duration::from_millis(20),
            std::future::pending::<std::result::Result<(), ()>>()
        )
        .await,
        Err(TransportError::Unavailable)
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    token.cancel();
    assert_eq!(
        bounded_write(
            &token,
            Duration::from_secs(2),
            std::future::pending::<std::result::Result<(), ()>>()
        )
        .await,
        Err(TransportError::Stale)
    );
    assert_eq!(
        bounded_write(&CancellationToken::new(), Duration::from_secs(1), async {
            Err(())
        })
        .await,
        Err(TransportError::Unavailable)
    );
}
#[tokio::test]
async fn outbound_queue_full_and_stale_generation_fail_closed() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4();
    let (sender, mut receiver) = mpsc::channel(2);
    let connection = Connection {
        id,
        owner: f.store.owner_id(),
        epoch: 1,
        features: Features::default(),
        cancel: CancellationToken::new(),
        send: sender,
        heartbeat: tokio::sync::watch::channel(tokio::time::Instant::now()).0,
    };
    f.api
        .registry
        .lock()
        .await
        .insert(f.machine, connection.clone());
    let make = || Frame::Command {
        command: voyage_protocol::attachment::Command {
            version: 2,
            connection_id: id,
            machine_id: f.machine,
            principal_id: f.store.owner_id(),
            command_id: Uuid::new_v4(),
            expires_at_ms: now() + 10_000,
            operation: voyage_protocol::attachment::Operation::List {
                after: None,
                limit: 1,
            },
        },
    };
    f.api.send(f.machine, make()).await.unwrap();
    f.api.send(f.machine, make()).await.unwrap();
    assert_eq!(
        f.api.send(f.machine, make()).await,
        Err(TransportError::Full)
    );
    assert_eq!(receiver.len(), 2);
    connection.cancel.cancel();
    assert_eq!(
        f.api.send(f.machine, make()).await,
        Err(TransportError::Stale)
    );
    let queued = receiver.recv().await.unwrap();
    assert!(matches!(queued, Frame::Command { .. }));
    assert!(!f.api.is_current(f.machine, id).await);
}
#[tokio::test]
async fn authenticated_machine_limit_denies_new_machine_but_allows_replacement() {
    let mut f = Fixture::new().await;
    let proof = f.proof();
    let mut first = f.socket().await;
    let first_id = f.authenticate(&mut first, proof).await;
    let first_machine = f.machine;
    f.machine = Uuid::new_v4();
    f.key = SigningKey::generate().unwrap();
    let invite = f.store.invite(60_000, now()).unwrap();
    let operation = ProofOperation::Enroll {
        machine_id: f.machine,
        transaction_id: Uuid::new_v4(),
        invitation_id: invite.id,
        public_key: f.key.public_key(),
    };
    let challenge = f
        .store
        .challenge(operation, Some(&invite.key), now())
        .unwrap();
    let proof = SignedChallenge {
        signature: f.key.sign(&challenge).unwrap(),
        challenge,
        new_signature: None,
    };
    f.store.complete(&proof, Some(&invite.key), now()).unwrap();
    let proof = f.proof();
    let mut second = f.socket().await;
    second
        .send(ClientMessage::Text(
            Frame::Authenticate {
                version: 2,
                proof,
                features: Features::default(),
            }
            .encode()
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    closed(&mut second).await;
    assert!(f.api.is_current(first_machine, first_id).await);
    assert_eq!(f.api.registry.lock().await.len(), 1);
    drop(first);
    tokio::time::timeout(Duration::from_secs(6), async {
        while f.api.is_current(first_machine, first_id).await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let proof = f.proof();
    let mut next = f.socket().await;
    let id = f.authenticate(&mut next, proof).await;
    assert!(f.api.is_current(f.machine, id).await);
}
#[tokio::test]
async fn websocket_ping_is_supported_before_auth_and_does_not_renew_lease() {
    let mut f = Fixture::new_with_limits(Limits {
        lease: Duration::from_millis(1000),
        ..functional_limits()
    })
    .await;
    let proof = f.proof();
    let mut socket = f.socket().await;
    socket
        .send(ClientMessage::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        ClientMessage::Pong(vec![1, 2, 3].into())
    );
    let id = f.authenticate(&mut socket, proof).await;
    let granted_at = tokio::time::Instant::now();
    tokio::time::sleep(Duration::from_millis(700)).await;
    socket
        .send(ClientMessage::Ping(vec![4].into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        ClientMessage::Pong(vec![4].into())
    );
    assert!(f.api.is_current(f.machine, id).await);
    closed(&mut socket).await;
    assert!(
        granted_at.elapsed() < Duration::from_millis(1400),
        "Ping must not extend the 1000ms grant"
    );
    assert!(!f.api.is_current(f.machine, id).await);
}
#[tokio::test]
async fn unauthenticated_ping_does_not_extend_authentication_deadline() {
    let f = Fixture::new_with_limits(Limits {
        auth: Duration::from_millis(150),
        ..functional_limits()
    })
    .await;
    let mut socket = f.socket().await;
    let started = tokio::time::Instant::now();
    let pings = async {
        loop {
            if socket
                .send(ClientMessage::Ping(vec![0].into()))
                .await
                .is_err()
            {
                break;
            }
            match socket.next().await {
                Some(Ok(ClientMessage::Pong(_))) => (),
                _ => break,
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    };
    tokio::time::timeout(Duration::from_millis(500), pings)
        .await
        .expect("pings cannot keep an unauthenticated socket alive");
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(f.api.registry.lock().await.is_empty());
}
#[tokio::test]
async fn functional_fixture_accepts_authentication_within_production_deadline() {
    let mut f = Fixture::new().await;
    let proof = f.proof();
    let mut socket = f.socket().await;
    // A valid client may need more than the old fixture's150ms to schedule its
    // proof. This is well inside the actual five-second authentication contract.
    tokio::time::sleep(Duration::from_millis(250)).await;
    let id = f.authenticate(&mut socket, proof).await;
    assert!(f.api.is_current(f.machine, id).await);
}

#[tokio::test]
async fn current_connection_rejects_expired_lease_without_waiting_for_monitor() {
    let f = Fixture::new().await;
    let receipt = f
        .api
        .enrollment
        .attachment_current(f.machine, 1)
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let (send, _receiver) = mpsc::channel(1);
    let heartbeat =
        watch::channel(tokio::time::Instant::now() - f.api.limits.lease - Duration::from_secs(1)).0;
    f.api.registry.lock().await.insert(
        f.machine,
        Connection {
            id,
            owner: receipt.owner_id,
            epoch: 1,
            features: Features::default(),
            cancel: CancellationToken::new(),
            send,
            heartbeat,
        },
    );
    // No monitor task exists for this synthetic connection. Current observation
    // must enforce the lease itself, even when a monitor has not been scheduled.
    assert!(!f.api.is_current(f.machine, id).await);
}

#[tokio::test]
async fn shutdown_closes_authenticated_and_pending_sockets_and_future_admission() {
    let mut f = Fixture::new().await;
    let mut pending = f.socket().await;
    let mut active = f.socket().await;
    let proof = f.proof();
    let id = f.authenticate(&mut active, proof).await;
    let records = f.api.connections().await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].machine_id, f.machine);
    assert_eq!(records[0].connection_id, id);
    assert_eq!(records[0].epoch, 1);
    f.api.shutdown().await;
    assert!(!f.api.is_current(f.machine, id).await);
    assert!(f.api.connections().await.is_empty());
    closed(&mut pending).await;
    closed(&mut active).await;
    f.api.shutdown().await;
    let error = connect_async(f.request()).await.unwrap_err();
    assert!(
        matches!(error, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == StatusCode::SERVICE_UNAVAILABLE)
    );
}

#[tokio::test]
async fn presence_rejects_application_frames_without_consumer_or_disclosure() {
    let mut f = Fixture::new().await;
    let presence = AttachmentApi::presence(f.api.enrollment.clone()).unwrap();
    assert!(presence.observations.is_closed() && presence.supported.is_empty());
    // Use the same closed-consumer path with the real fixture's shorter capacity.
    f.rx.close();
    let mut socket = f.socket().await;
    let proof = f.proof();
    let id = f.authenticate(&mut socket, proof).await;
    socket
        .send(ClientMessage::Text(
            Frame::Result {
                connection_id: id,
                command_id: Uuid::new_v4(),
                reply: voyage_protocol::stream::Reply::Accepted {},
            }
            .encode()
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    closed(&mut socket).await;
    assert!(f.api.connections().await.is_empty());
    assert!(f.rx.recv().await.is_none());
}

mod presence_review;
