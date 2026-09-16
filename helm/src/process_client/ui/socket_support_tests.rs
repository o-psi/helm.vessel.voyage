//! Real loopback Vessel socket, with scripted public replies and request capture.
//! Credentials and listeners are private to the temporary directory.
use super::{
    Client,
    state::{Route, Target},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use uuid::Uuid;
use voyage_protocol::{
    duplex::{ClientFrame, SUBPROTOCOL, ServerFrame},
    vessel::{VESSEL_API_VERSION, VesselCommand, VesselResponse},
};

pub(super) struct Server {
    pub client: Client,
    pub target: Target,
    pub incarnation: Uuid,
    pub requests: tokio::sync::mpsc::UnboundedReceiver<VesselCommand>,
    task: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.client.disconnect();
        self.task.abort();
    }
}
impl Server {
    // The handshake callback error type is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    pub async fn new(
        mut reply: impl FnMut(&VesselCommand) -> Result<Value, String> + Send + 'static,
    ) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let path = directory.path().join("process-http.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"endpoint":endpoint,"token":"a".repeat(64)}))
                .unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let client = Client::local(directory.path().to_owned());
        let target = Target {
            route: Route::of(&client),
            session: Uuid::new_v4(),
        };
        let incarnation = Uuid::new_v4();
        let (sender, requests) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(stream,
                |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    response.headers_mut().insert("sec-websocket-protocol", SUBPROTOCOL.parse().unwrap());
                    Ok(response)
                }).await.unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::to_string(&ServerFrame::Hello {
                        protocol: VESSEL_API_VERSION,
                        socket_id: Uuid::new_v4(),
                        vessel_id: Uuid::new_v4(),
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                use tokio_tungstenite::tungstenite::Message;
                match message {
                    Message::Text(text) => {
                        let frame: ClientFrame = serde_json::from_str(&text).unwrap();
                        let ClientFrame::Command {
                            request_id,
                            request,
                        } = frame
                        else {
                            continue;
                        };
                        let result = reply(&request.command);
                        let _ = sender.send(request.command);
                        let (result, error) = match result {
                            Ok(value) => (value, None),
                            Err(error) => (Value::Null, Some(error)),
                        };
                        let frame = ServerFrame::Reply {
                            request_id,
                            response: VesselResponse {
                                protocol: VESSEL_API_VERSION,
                                result,
                                error,
                                outcome_unknown: false,
                            },
                        };
                        if socket
                            .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Message::Ping(bytes) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        });
        Self {
            client,
            target,
            incarnation,
            requests,
            task,
            _directory: directory,
        }
    }
}

pub(super) fn voyage(command: &VesselCommand, result: Value) -> Value {
    let VesselCommand::Voyage(request) = command else {
        panic!("expected voyage request")
    };
    serde_json::json!({"session_id":request.session_id,"incarnation":request.incarnation.unwrap_or(Uuid::nil()),"result":result})
}
