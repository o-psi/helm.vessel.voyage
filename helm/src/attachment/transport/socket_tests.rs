//! Real HTTP enrollment and WebSocket regression fixtures. No provider effects.
#![cfg(unix)]
use super::*;
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message as AxumMessage, WebSocket},
    },
    routing::get,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use vessel::{
    attachment_transport::AttachmentApi, enrollment::EnrollmentStore,
    enrollment_http::EnrollmentApi,
};
use voyage_protocol::{
    attachment::{Command, Operation},
    enrollment::ProofOperation,
    events::Feature,
    stream::Reply,
};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
#[derive(Clone, Copy)]
enum Script {
    Ping,
    Owner,
    Epoch,
    Features,
    Minimum,
    Shrink,
    DelayedWelcome,
    DelayedLease,
    Flood,
    Idle,
}
#[derive(Clone)]
struct ScriptState {
    mode: Script,
    owner: Uuid,
    heartbeats: Arc<AtomicUsize>,
    proofs: Arc<Mutex<Vec<Uuid>>>,
    results: Arc<Mutex<Vec<Uuid>>>,
}
async fn upgrade(
    State(state): State<ScriptState>,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    ws.protocols([PROTOCOL])
        .on_upgrade(move |socket| scripted(socket, state))
}
async fn scripted(mut socket: WebSocket, state: ScriptState) {
    let Some(Ok(AxumMessage::Text(text))) = socket.recv().await else {
        return;
    };
    let Frame::Authenticate {
        proof, features, ..
    } = Frame::decode(text.as_bytes()).unwrap()
    else {
        panic!("authenticate first");
    };
    state.proofs.lock().unwrap().push(proof.challenge.id);
    let ProofOperation::Connect { machine_id, epoch } = proof.challenge.operation else {
        panic!("fresh connect proof");
    };
    let connection_id = Uuid::new_v4();
    if matches!(state.mode, Script::Ping) {
        socket
            .send(AxumMessage::Ping(b"before-welcome".to_vec().into()))
            .await
            .unwrap();
        match tokio::time::timeout(Duration::from_secs(2), socket.recv()).await {
            Ok(Some(Ok(AxumMessage::Pong(payload)))) if payload.as_ref() == b"before-welcome" => (),
            _ => return,
        }
    }
    if matches!(state.mode, Script::DelayedWelcome) {
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }
    let lease_ms = match state.mode {
        Script::Minimum | Script::DelayedWelcome | Script::DelayedLease => 1000,
        Script::Shrink => 4000,
        _ => 10000,
    };
    let welcome = Frame::Welcome {
        version: VERSION,
        connection_id,
        machine_id,
        owner_id: if matches!(state.mode, Script::Owner) {
            Uuid::new_v4()
        } else {
            state.owner
        },
        epoch: epoch + u64::from(matches!(state.mode, Script::Epoch)),
        lease_ms,
        features: if matches!(state.mode, Script::Features) {
            Features::new(vec![Feature::SequencedEvents]).unwrap()
        } else {
            features
        },
    };
    if socket
        .send(AxumMessage::Text(welcome.encode().unwrap().into()))
        .await
        .is_err()
    {
        return;
    }
    if matches!(state.mode, Script::Flood) {
        for _ in 0..QUEUE + 1 {
            let frame = command(connection_id, machine_id, state.owner);
            if socket
                .send(AxumMessage::Text(frame.encode().unwrap().into()))
                .await
                .is_err()
            {
                return;
            }
        }
    }
    while let Some(Ok(message)) = socket.recv().await {
        match message {
            AxumMessage::Text(text) => {
                if let Ok(Frame::Result { command_id, .. }) = Frame::decode(text.as_bytes()) {
                    state.results.lock().unwrap().push(command_id);
                }
                if matches!(Frame::decode(text.as_bytes()), Ok(Frame::Heartbeat { .. })) {
                    state.heartbeats.fetch_add(1, Ordering::SeqCst);
                    if matches!(state.mode, Script::DelayedLease) {
                        tokio::time::sleep(Duration::from_millis(1100)).await;
                    }
                    let lease_ms = if matches!(state.mode, Script::Shrink) {
                        1000
                    } else {
                        lease_ms
                    };
                    let frame = Frame::Lease {
                        connection_id,
                        lease_ms,
                    };
                    if socket
                        .send(AxumMessage::Text(frame.encode().unwrap().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            AxumMessage::Ping(payload) => {
                if socket.send(AxumMessage::Pong(payload)).await.is_err() {
                    return;
                }
            }
            AxumMessage::Close(_) => return,
            _ => (),
        }
    }
}
fn command(connection_id: Uuid, machine_id: Uuid, owner: Uuid) -> Frame {
    Frame::Command {
        command: Command {
            version: VERSION,
            connection_id,
            machine_id,
            principal_id: owner,
            command_id: Uuid::new_v4(),
            expires_at_ms: now() + 20000,
            operation: Operation::List {
                after: None,
                limit: 10,
            },
        },
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    origin: String,
    client_dir: PathBuf,
    owner: Uuid,
    client: Option<EnrollmentClient>,
    server: JoinHandle<()>,
    script: Option<ScriptState>,
    api: Option<AttachmentApi>,
    observations: Option<mpsc::Receiver<vessel::attachment_transport::AuthenticatedFrame>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new(mode: Option<Script>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let mut store = EnrollmentStore::open(&root.path().join("server"), &origin, true).unwrap();
        let owner = store.owner_id();
        let invitation = store.invite(60000, now()).unwrap();
        let enrollment = EnrollmentApi::new(store, &"o".repeat(40)).unwrap();
        let mut router = enrollment.clone().router();
        let (script, api, observations) = if let Some(mode) = mode {
            let state = ScriptState {
                mode,
                owner,
                heartbeats: Arc::new(AtomicUsize::new(0)),
                proofs: Arc::new(Mutex::new(Vec::new())),
                results: Arc::new(Mutex::new(Vec::new())),
            };
            router = router.merge(
                Router::new()
                    .route("/v2/attachment", get(upgrade))
                    .with_state(state.clone()),
            );
            (Some(state), None, None)
        } else {
            let (api, observations) = AttachmentApi::new(enrollment, Features::default()).unwrap();
            router = router.merge(api.clone().router());
            (None, Some(api), Some(observations))
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client_dir = root.path().join("client");
        let mut client = EnrollmentClient::open(&client_dir, &origin, true).unwrap();
        client.enroll(invitation.id, &invitation.key).await.unwrap();
        Self {
            _root: root,
            origin,
            client_dir,
            owner,
            client: Some(client),
            server,
            script,
            api,
            observations,
        }
    }
    async fn connect(&mut self) -> Result<Connection> {
        connect(
            self.client.take().unwrap(),
            Features::default(),
            CancellationToken::new(),
        )
        .await
    }
    fn reopen(&self) -> EnrollmentClient {
        EnrollmentClient::open(&self.client_dir, &self.origin, true).unwrap()
    }
}

#[tokio::test]
async fn ping_before_welcome_is_answered_within_original_handshake_deadline() {
    let mut fixture = Fixture::new(Some(Script::Ping)).await;
    let connection = fixture
        .connect()
        .await
        .expect("legal Ping must not reject Welcome");
    assert!(connection.is_active());
    connection.close().await;
}

#[tokio::test]
async fn minimum_lease_renews_and_shortened_lease_reschedules_heartbeat() {
    for mode in [Script::Minimum, Script::Shrink] {
        let mut fixture = Fixture::new(Some(mode)).await;
        let connection = fixture.connect().await.unwrap();
        let until = Instant::now() + Duration::from_secs(6);
        while Instant::now() < until
            && fixture
                .script
                .as_ref()
                .unwrap()
                .heartbeats
                .load(Ordering::SeqCst)
                < 4
        {
            assert!(
                connection.is_active(),
                "conforming short lease expired before renewal"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert!(
            fixture
                .script
                .as_ref()
                .unwrap()
                .heartbeats
                .load(Ordering::SeqCst)
                >= 4
        );
        assert!(connection.is_active());
        connection.close().await;
    }
}

#[tokio::test]
async fn drop_retains_enrollment_lock_until_socket_task_observes_cancellation() {
    let mut fixture = Fixture::new(Some(Script::Idle)).await;
    let connection = fixture.connect().await.unwrap();
    drop(connection);
    // No await on this current-thread runtime: the socket task has not exited yet.
    assert!(
        EnrollmentClient::open(&fixture.client_dir, &fixture.origin, true).is_err(),
        "enrollment unlocked while socket task still exists"
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(client) = EnrollmentClient::open(&fixture.client_dir, &fixture.origin, true) {
                drop(client);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn wrong_identity_epoch_or_unoffered_features_rejects_handshake() {
    for mode in [Script::Owner, Script::Epoch, Script::Features] {
        let mut fixture = Fixture::new(Some(mode)).await;
        assert!(matches!(
            fixture.connect().await,
            Err(TransportError::Invalid)
        ));
        drop(fixture.reopen());
    }
}

#[tokio::test]
async fn late_welcome_or_lease_cannot_resurrect_expired_authority() {
    let mut fixture = Fixture::new(Some(Script::DelayedWelcome)).await;
    assert!(matches!(
        fixture.connect().await,
        Err(TransportError::Timeout)
    ));
    let mut fixture = Fixture::new(Some(Script::DelayedLease)).await;
    let mut connection = fixture.connect().await.unwrap();
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert!(!connection.is_active());
    assert!(connection.receive().await.is_none());
    assert!(matches!(
        connection.send(Frame::Result {
            connection_id: connection.context().connection_id,
            command_id: Uuid::new_v4(),
            reply: Reply::Accepted {}
        }),
        Err(TransportError::Closed)
    ));
    connection.close().await;
}

#[tokio::test]
async fn inbound_and_outbound_queue_saturation_close_without_delivering_stale_frames() {
    let mut fixture = Fixture::new(Some(Script::Flood)).await;
    let mut connection = fixture.connect().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while connection.is_active() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(connection.receive().await.is_none());
    connection.close().await;
    let mut fixture = Fixture::new(Some(Script::Idle)).await;
    let mut connection = fixture.connect().await.unwrap();
    for _ in 0..QUEUE {
        connection
            .send(Frame::Result {
                connection_id: connection.context().connection_id,
                command_id: Uuid::new_v4(),
                reply: Reply::Accepted {},
            })
            .unwrap();
    }
    assert_eq!(
        connection.send(Frame::Result {
            connection_id: connection.context().connection_id,
            command_id: Uuid::new_v4(),
            reply: Reply::Accepted {}
        }),
        Err(TransportError::Busy)
    );
    assert!(!connection.is_active());
    assert!(connection.receive().await.is_none());
    connection.close().await;
}

#[tokio::test]
async fn reconnect_uses_fresh_proof_and_drops_old_outgoing_queue() {
    let mut fixture = Fixture::new(Some(Script::Idle)).await;
    let first = fixture.connect().await.unwrap();
    let old_id = first.context().connection_id;
    first
        .send(Frame::Result {
            connection_id: old_id,
            command_id: Uuid::new_v4(),
            reply: Reply::Accepted {},
        })
        .unwrap();
    first.close().await;
    fixture.client = Some(fixture.reopen());
    let second = fixture.connect().await.unwrap();
    assert_ne!(second.context().connection_id, old_id);
    assert!(matches!(
        second.send(Frame::Result {
            connection_id: old_id,
            command_id: Uuid::new_v4(),
            reply: Reply::Accepted {}
        }),
        Err(TransportError::Invalid)
    ));
    let proofs = fixture
        .script
        .as_ref()
        .unwrap()
        .proofs
        .lock()
        .unwrap()
        .clone();
    assert_eq!(proofs.len(), 2);
    assert_ne!(proofs[0], proofs[1]);
    second.close().await;
    assert!(
        fixture
            .script
            .as_ref()
            .unwrap()
            .results
            .lock()
            .unwrap()
            .is_empty(),
        "cancelled outgoing frame was replayed or delivered"
    );
}

#[tokio::test]
async fn real_enrollment_attachment_command_reply_and_revocation() {
    let mut fixture = Fixture::new(None).await;
    let mut connection = fixture.connect().await.unwrap();
    let context = connection.context().clone();
    let frame = command(context.connection_id, context.machine_id, fixture.owner);
    let Frame::Command { command: sent } = &frame else {
        unreachable!()
    };
    let command_id = sent.command_id;
    fixture
        .api
        .as_ref()
        .unwrap()
        .send(context.machine_id, frame)
        .await
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(2), connection.receive())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(received, Frame::Command { command } if command.command_id == command_id));
    connection
        .send(Frame::Result {
            connection_id: context.connection_id,
            command_id,
            reply: Reply::Accepted {},
        })
        .unwrap();
    let observed = tokio::time::timeout(
        Duration::from_secs(2),
        fixture.observations.as_mut().unwrap().recv(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(observed.machine_id, context.machine_id);
    assert!(matches!(observed.frame, Frame::Result { command_id: id, .. } if id == command_id));
    assert!(
        fixture
            .api
            .as_ref()
            .unwrap()
            .is_current(context.machine_id, context.connection_id)
            .await
    );
    reqwest::Client::new().post(format!("{}/v2/enrollment/revoke", fixture.origin)).header("origin", &fixture.origin).header("x-voyage-request", "2").bearer_auth("o".repeat(40))
        .json(&serde_json::json!({"machine_id":context.machine_id,"expected_epoch":context.epoch,"transaction_id":Uuid::new_v4()})).send().await.unwrap().error_for_status().unwrap();
    assert!(
        !fixture
            .api
            .as_ref()
            .unwrap()
            .is_current(context.machine_id, context.connection_id)
            .await
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(3), connection.receive())
            .await
            .unwrap()
            .is_none()
    );
    assert!(!connection.is_active());
    connection.close().await;
    let client = fixture.reopen();
    assert!(
        connect(client, Features::default(), CancellationToken::new())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_during_handshake_and_active_receive_releases_connection() {
    let mut fixture = Fixture::new(Some(Script::DelayedWelcome)).await;
    let cancellation = CancellationToken::new();
    let connecting = tokio::spawn(connect(
        fixture.client.take().unwrap(),
        Features::default(),
        cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(30)).await;
    cancellation.cancel();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), connecting)
            .await
            .unwrap()
            .unwrap(),
        Err(TransportError::Closed)
    ));
    drop(fixture.reopen());
    let mut fixture = Fixture::new(Some(Script::Idle)).await;
    let cancellation = CancellationToken::new();
    let mut connection = connect(
        fixture.client.take().unwrap(),
        Features::default(),
        cancellation.clone(),
    )
    .await
    .unwrap();
    cancellation.cancel();
    assert!(!connection.is_active());
    assert!(
        tokio::time::timeout(Duration::from_secs(1), connection.receive())
            .await
            .unwrap()
            .is_none()
    );
    connection.close().await;
    drop(fixture.reopen());
}
