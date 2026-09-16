//! A bounded scripted peer: every request must match, and every script must drain.
use super::Client;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use uuid::Uuid;
use voyage_protocol::{
    duplex::{ClientFrame, SUBPROTOCOL, ServerFrame},
    vessel::{VESSEL_API_VERSION, VesselCommand, VesselResponse},
};

pub struct Peer {
    pub client: Client,
    pub root: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}
pub fn info(session: Uuid, incarnation: Uuid) -> Value {
    json!({"session_id":session,"incarnation":incarnation,"workspace":"/synthetic","state":"live"})
}
pub fn wire(command: VesselCommand) -> Value {
    serde_json::to_value(command).unwrap()
}
/// Null expected values are wildcards for generated UUIDs/deadlines only.
fn matches(expected: &Value, actual: &Value) {
    if expected.is_null() {
        return;
    }
    if let Some(object) = expected.as_object() {
        for (key, value) in object {
            matches(
                value,
                actual
                    .get(key)
                    .unwrap_or_else(|| panic!("missing {key}: {actual}")),
            );
        }
    } else {
        assert_eq!(expected, actual);
    }
}
impl Peer {
    pub async fn new(script: Vec<(Value, Value)>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("vessel");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let credential = directory.join("process-http.json");
        std::fs::write(&credential, json!({"endpoint":format!("http://{}",listener.local_addr().unwrap()),"token":"a".repeat(64)}).to_string()).unwrap();
        std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
        let task = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(8), async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_hdr_async(stream, |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    response.headers_mut().insert("sec-websocket-protocol", SUBPROTOCOL.parse().unwrap()); Ok(response)
                }).await.unwrap();
                socket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&ServerFrame::Hello { protocol: VESSEL_API_VERSION, socket_id: Uuid::new_v4(), vessel_id: Uuid::new_v4() }).unwrap().into())).await.unwrap();
                for (expected, result) in script {
                    let frame = loop {
                        match socket.next().await.unwrap().unwrap() {
                            tokio_tungstenite::tungstenite::Message::Text(text) => break serde_json::from_str::<ClientFrame>(&text).unwrap(),
                            tokio_tungstenite::tungstenite::Message::Ping(data) => socket.send(tokio_tungstenite::tungstenite::Message::Pong(data)).await.unwrap(),
                            other => panic!("unexpected frame {other:?}"),
                        }
                    };
                    let ClientFrame::Command { request_id, request } = frame else { panic!("expected command") };
                    matches(&expected, &serde_json::to_value(&request.command).unwrap());
                    let result = if let VesselCommand::Voyage(request) = &request.command {
                        json!({"session_id":request.session_id,"incarnation":request.incarnation.unwrap_or(Uuid::from_u128(2)),"result":result})
                    } else { result };
                    let reply = ServerFrame::Reply { request_id, response: VesselResponse { protocol: VESSEL_API_VERSION, result, error: None, outcome_unknown: false } };
                    socket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&reply).unwrap().into())).await.unwrap();
                }
                // Keep the final reply readable until the test retires its client.
                while let Some(Ok(message)) = socket.next().await {
                    if message.is_close() { break; }
                    assert!(!message.is_text(), "unexpected request after script drained: {message}");
                    if let tokio_tungstenite::tungstenite::Message::Ping(data) = message { let _ = socket.send(tokio_tungstenite::tungstenite::Message::Pong(data)).await; }
                }
            }).await.expect("script deadline");
        });
        Self {
            client: Client::local(directory),
            root,
            task,
        }
    }
    pub async fn finish(self) {
        self.client.disconnect();
        self.task.await.unwrap();
    }
}

pub fn voyage(
    session: Uuid,
    incarnation: Uuid,
    command: voyage_protocol::vessel::VoyageCommand,
) -> Value {
    wire(VesselCommand::Voyage(
        voyage_protocol::vessel::VoyageRequest {
            session_id: session,
            incarnation: command.requires_incarnation().then_some(incarnation),
            command,
        },
    ))
}
