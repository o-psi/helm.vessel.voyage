//! Synthetic peers speak the real HTTP/WebSocket protocols, never launch a Vessel.
use super::transport::Client;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
use uuid::Uuid;
use voyage_protocol::{duplex::*, vessel::*};

pub(super) struct Peer {
    pub client: Client,
    pub socket: WebSocketStream<TcpStream>,
    _root: tempfile::TempDir,
}
impl Peer {
    // The handshake callback error type is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    pub async fn open() -> Self {
        let root = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let vessel = Uuid::new_v4();
        let credential = json!({"schema_version":1,"kind":"workspace","endpoint":endpoint,"grant_id":Uuid::new_v4(),"principal_id":Uuid::new_v4(),"vessel_id":vessel,"token":"synthetic-only"});
        let path = root.path().join("credential.json");
        write_private(&path, &credential);
        let client = Client::access(path);
        let c = client.clone();
        let connect = tokio::spawn(async move { c.request(VesselCommand::Capabilities).await });
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(stream, |request: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            assert_eq!(request.uri().path(), SOCKET_PATH);
            assert_eq!(request.headers()["authorization"], "Bearer synthetic-only");
            response.headers_mut().insert("sec-websocket-protocol", SUBPROTOCOL.parse().unwrap());
            Ok(response)
        }).await.unwrap();
        send(
            &mut socket,
            ServerFrame::Hello {
                protocol: VESSEL_API_VERSION,
                socket_id: Uuid::new_v4(),
                vessel_id: vessel,
            },
        )
        .await;
        let ClientFrame::Command {
            request_id,
            request,
        } = frame(&mut socket).await
        else {
            panic!("expected command")
        };
        assert!(matches!(request.command, VesselCommand::Capabilities));
        send(
            &mut socket,
            ServerFrame::Reply {
                request_id,
                response: response(json!({"fixture":true})),
            },
        )
        .await;
        assert_eq!(connect.await.unwrap().unwrap(), json!({"fixture":true}));
        Self {
            client,
            socket,
            _root: root,
        }
    }
    pub async fn command(&mut self) -> (Uuid, VesselCommand) {
        let ClientFrame::Command {
            request_id,
            request,
        } = frame(&mut self.socket).await
        else {
            panic!("expected command")
        };
        assert_eq!(request.protocol, VESSEL_API_VERSION);
        (request_id, request.command)
    }
    pub async fn reply(&mut self, id: Uuid, result: Value) {
        send(
            &mut self.socket,
            ServerFrame::Reply {
                request_id: id,
                response: response(result),
            },
        )
        .await;
    }
    pub async fn voyage_reply(
        &mut self,
        id: Uuid,
        session: Uuid,
        incarnation: Uuid,
        result: Value,
    ) {
        self.reply(
            id,
            json!({"session_id":session,"incarnation":incarnation,"result":result}),
        )
        .await;
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.client.disconnect();
    }
}
pub(super) fn write_private(path: &std::path::Path, value: &Value) {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .unwrap()
        .write_all(&serde_json::to_vec(value).unwrap())
        .unwrap();
}
pub(super) fn response(result: Value) -> VesselResponse {
    VesselResponse {
        protocol: VESSEL_API_VERSION,
        result,
        error: None,
        outcome_unknown: false,
    }
}
pub(super) async fn frame(peer: &mut WebSocketStream<TcpStream>) -> ClientFrame {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match peer.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(bytes) => peer.send(Message::Pong(bytes)).await.unwrap(),
                Message::Pong(_) => {}
                other => panic!("unexpected frame {other:?}"),
            }
        }
    })
    .await
    .expect("synthetic peer deadline")
}
pub(super) async fn send(peer: &mut WebSocketStream<TcpStream>, value: ServerFrame) {
    peer.send(Message::Text(serde_json::to_string(&value).unwrap().into()))
        .await
        .unwrap();
}

// One request per connection and an explicit close avoid relying on HTTP keepalive.
pub(super) async fn http_peer(
    responses: Vec<(u16, Value)>,
) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut bodies = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let byte = stream.read_u8().await.unwrap();
                bytes.push(byte);
                assert!(bytes.len() < 65536);
                if bytes.ends_with(b"\r\n\r\n") {
                    break bytes.len();
                }
            };
            let headers = String::from_utf8(bytes.clone()).unwrap();
            let len = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(|n| n.parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            bytes.resize(header_end + len, 0);
            stream.read_exact(&mut bytes[header_end..]).await.unwrap();
            bodies.push(if len == 0 {
                json!(null)
            } else {
                serde_json::from_slice(&bytes[header_end..]).unwrap()
            });
            let body = serde_json::to_vec(&body).unwrap();
            stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
            stream.shutdown().await.unwrap();
        }
        bodies
    });
    (endpoint, task)
}
