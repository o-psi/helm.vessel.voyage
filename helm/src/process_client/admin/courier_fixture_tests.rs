//! Correlated public duplex peers for courier transport/journal assertions only.
//! Artifact signatures are typed opaque fixtures, not cryptographic/native proof.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::tungstenite::Message;
use voyage_protocol::duplex::{ClientFrame, SUBPROTOCOL, ServerFrame};

pub const WAIT: Duration = Duration::from_secs(6);
enum Action {
    Reply(VesselResponse),
    Lose,
}
pub struct Packet {
    pub command: VesselCommand,
    response: oneshot::Sender<Action>,
}
impl Packet {
    pub fn ok(self, value: Value) {
        self.response
            .send(Action::Reply(VesselResponse {
                protocol: VESSEL_API_VERSION,
                result: value,
                error: None,
                outcome_unknown: false,
            }))
            .unwrap_or_else(|_| panic!("owned courier reply receiver ended"));
    }
    pub fn refused(self, unknown: bool) {
        self.response
            .send(Action::Reply(VesselResponse {
                protocol: VESSEL_API_VERSION,
                result: Value::Null,
                error: Some("synthetic courier refusal".into()),
                outcome_unknown: unknown,
            }))
            .unwrap_or_else(|_| panic!("owned courier reply receiver ended"));
    }
    pub fn lose(self) {
        self.response
            .send(Action::Lose)
            .unwrap_or_else(|_| panic!("owned courier reply receiver ended"));
    }
}
pub struct Endpoint {
    pub directory: PathBuf,
    pub client: Client,
    pub identity: VesselIdentity,
    pub calls: Arc<Mutex<Vec<Value>>>,
    address: std::net::SocketAddr,
    commands: mpsc::Receiver<Packet>,
    stop: Option<oneshot::Sender<()>>,
    job: Option<tokio::task::JoinHandle<()>>,
}
impl Endpoint {
    pub async fn new(root: &Path, name: &str, key_byte: u8) -> Self {
        let directory = root.join(name);
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        write(
            &directory.join("process-http.json"),
            &json!({"endpoint":format!("http://{address}"),"token":"a".repeat(64)}),
        );
        let identity = VesselIdentity {
            vessel_id: Uuid::new_v4(),
            public_key: STANDARD.encode([key_byte; 32]),
        };
        let client = Client::local(directory.clone());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let capture = calls.clone();
        let vessel = identity.vessel_id;
        let (tx, commands) = mpsc::channel(32);
        let (stop, mut retired) = oneshot::channel();
        let job = tokio::spawn(async move {
            let mut peers = tokio::task::JoinSet::new();
            let mut accepted = 0;
            loop {
                tokio::select! {
                    _=&mut retired=>break,
                    Some(result)=peers.join_next(),if !peers.is_empty()=>result.unwrap(),
                    accept=listener.accept()=>{
                        let(stream,_)=accept.unwrap();accepted+=1;assert!(accepted<=8,"bounded explicit fixture connections");
                        let tx=tx.clone();let calls=capture.clone();
                        peers.spawn(async move {
                            // Header callback type is fixed by tungstenite.
                            #[allow(clippy::result_large_err)]
                            fn upgrade(request:&tokio_tungstenite::tungstenite::handshake::server::Request,mut response:tokio_tungstenite::tungstenite::handshake::server::Response)->Result<tokio_tungstenite::tungstenite::handshake::server::Response,tokio_tungstenite::tungstenite::handshake::server::ErrorResponse>{
                                assert_eq!(request.uri().path(),voyage_protocol::duplex::SOCKET_PATH);
                                assert_eq!(request.headers()["authorization"],format!("Bearer {}","a".repeat(64)));
                                assert!(!request.headers().contains_key("x-voyage-grant"),"explicit local account transport, no invented scoped authority");
                                response.headers_mut().insert("sec-websocket-protocol",SUBPROTOCOL.parse().unwrap());Ok(response)
                            }
                            let mut socket=tokio::time::timeout(WAIT,tokio_tungstenite::accept_hdr_async(stream,upgrade)).await.unwrap().unwrap();
                            socket.send(Message::Text(serde_json::to_string(&ServerFrame::Hello{protocol:VESSEL_API_VERSION,socket_id:Uuid::new_v4(),vessel_id:vessel}).unwrap().into())).await.unwrap();
                            loop {
                                let message=tokio::time::timeout(Duration::from_secs(20),socket.next()).await.expect("bounded fixture socket lifetime");
                                let Some(Ok(message))=message else{break;};
                                match message {
                                    Message::Close(_)=>break,
                                    Message::Ping(bytes)=>socket.send(Message::Pong(bytes)).await.unwrap(),
                                    Message::Pong(_)=>{},
                                    Message::Text(text)=>{
                                        assert!(text.len()<=196608);
                                        let ClientFrame::Command{request_id,request}=serde_json::from_str(&text).unwrap()else{panic!("courier cannot subscribe, reverse-execute or send private input");};
                                        assert_eq!(request.protocol,VESSEL_API_VERSION);assert!(!request_id.is_nil());
                                        {let mut saved=calls.lock().unwrap();assert!(saved.len()<2048);saved.push(serde_json::to_value(&request.command).unwrap());}
                                        let(reply,response)=oneshot::channel();
                                        tx.send(Packet{command:request.command,response:reply}).await.unwrap();
                                        match tokio::time::timeout(WAIT,response).await.unwrap().unwrap(){
                                            Action::Reply(response)=>socket.send(Message::Text(serde_json::to_string(&ServerFrame::Reply{request_id,response}).unwrap().into())).await.unwrap(),
                                            Action::Lose=>break,
                                        }
                                    }
                                    _=>panic!("only typed public text frames belong to a courier"),
                                }
                            }
                        });
                    }
                }
            }
            drop(listener);
            while let Some(result) = peers.join_next().await {
                result.unwrap();
            }
        });
        Self {
            directory,
            client,
            identity,
            calls,
            address,
            commands,
            stop: Some(stop),
            job: Some(job),
        }
    }
    pub async fn next(&mut self) -> Packet {
        tokio::time::timeout(WAIT, self.commands.recv())
            .await
            .unwrap()
            .expect("owned courier command")
    }
    pub fn count(&self, op: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|value| value["op"] == op)
            .count()
    }
    pub async fn finish(&mut self) {
        self.client.disconnect();
        let _ = self.stop.take().unwrap().send(());
        tokio::time::timeout(WAIT, self.job.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(tokio::net::TcpStream::connect(self.address).await.is_err());
        assert!(self.commands.try_recv().is_err());
        assert!(self.client.connection_state().borrow().socket_id.is_none());
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.client.disconnect();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(job) = self.job.take() {
            job.abort();
        }
    }
}
pub fn write(path: &Path, value: &impl Serialize) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .unwrap();
    file.write_all(&serde_json::to_vec(value).unwrap()).unwrap();
    file.sync_all().unwrap();
}
pub fn journal(args: &MoveArgs) -> Value {
    let metadata = fs::symlink_metadata(&args.journal).unwrap();
    assert!(metadata.is_file());
    assert!(metadata.len() <= 65536);
    serde_json::from_slice(&fs::read(&args.journal).unwrap()).unwrap()
}
pub struct Journey {
    pub source: Endpoint,
    pub destination: Endpoint,
    pub args: MoveArgs,
    pub root: PathBuf,
}
impl Journey {
    pub async fn new() -> Self {
        // Failed fixtures retain their private operation journal for diagnosis.
        let root = tempfile::tempdir().unwrap().keep();
        let source = Endpoint::new(&root, "source", 7).await;
        let destination = Endpoint::new(&root, "destination", 8).await;
        let workspace = root.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let args = MoveArgs {
            session: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            expected_revision: 19,
            destination_directory: destination.directory.clone(),
            workspace,
            config_path: None,
            journal: root.join("move.json"),
        };
        Self {
            source,
            destination,
            args,
            root,
        }
    }
    pub fn begin(&self, args: MoveArgs) -> tokio::task::JoinHandle<Result<Value>> {
        let source = self.source.client.clone();
        tokio::spawn(async move { run(&source, args).await })
    }
    pub async fn identities(&mut self) {
        let packet = self.source.next().await;
        assert!(matches!(packet.command, VesselCommand::Identity));
        packet.ok(serde_json::to_value(&self.source.identity).unwrap());
        let packet = self.destination.next().await;
        assert!(matches!(packet.command, VesselCommand::Identity));
        packet.ok(serde_json::to_value(&self.destination.identity).unwrap());
    }
    pub fn preparation(&self) -> SignedArtifact<TransferPreparation> {
        let saved = journal(&self.args);
        SignedArtifact {
            payload: TransferPreparation {
                transfer_id: serde_json::from_value(saved["transfer_id"].clone()).unwrap(),
                source_vessel_id: self.source.identity.vessel_id,
                destination_vessel_id: self.destination.identity.vessel_id,
                session_id: self.args.session,
                nonce: serde_json::from_value(saved["prepare_command"].clone()).unwrap(),
                workspace: self.args.workspace.clone(),
                expires_at_ms: saved["expires_at_ms"].as_u64().unwrap(),
            },
            signature: STANDARD.encode([1; 64]),
        }
    }
    pub fn manifest(&self, bytes: &[u8]) -> SignedArtifact<TransferManifest> {
        use sha2::{Digest, Sha256};
        let preparation = self.preparation();
        SignedArtifact {
            payload: TransferManifest {
                transfer_id: preparation.payload.transfer_id,
                source_vessel_id: self.source.identity.vessel_id,
                destination_vessel_id: self.destination.identity.vessel_id,
                session_id: self.args.session,
                source_incarnation: self.args.incarnation,
                prepare_digest: format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&preparation).unwrap())
                ),
                artifact_sha256: format!("{:x}", Sha256::digest(bytes)),
                artifact_bytes: bytes.len() as u64,
                generation: 2,
            },
            signature: STANDARD.encode([2; 64]),
        }
    }
    pub async fn prepare(&mut self) {
        let packet = self.destination.next().await;
        let saved = journal(&self.args);
        let expected = json!({"op":"prepare_transfer","command_id":saved["prepare_command"],"transfer_id":saved["transfer_id"],"source_vessel_id":self.source.identity.vessel_id,"session_id":self.args.session,"workspace":self.args.workspace,"config_path":self.args.config_path,"expires_at_ms":saved["expires_at_ms"]});
        assert_eq!(serde_json::to_value(&packet.command).unwrap(), expected);
        assert!(
            saved["preparation"].is_null()
                && saved["manifest"].is_null()
                && saved["result"].is_null()
        );
        packet.ok(serde_json::to_value(self.preparation()).unwrap());
    }
    pub async fn export(&mut self, bytes: &[u8]) {
        let packet = self.source.next().await;
        let saved = journal(&self.args);
        let expected = json!({"op":"export_transfer","command_id":saved["export_command"],"session_id":self.args.session,"incarnation":self.args.incarnation,"expected_revision":self.args.expected_revision,"expires_at_ms":saved["expires_at_ms"],"preparation":self.preparation()});
        assert_eq!(serde_json::to_value(&packet.command).unwrap(), expected);
        assert!(!saved["preparation"].is_null() && saved["manifest"].is_null());
        packet.ok(serde_json::to_value(self.manifest(bytes)).unwrap());
    }
    pub async fn accept(&mut self, bytes: &[u8]) {
        let packet = self.destination.next().await;
        let saved = journal(&self.args);
        assert_eq!(
            serde_json::to_value(&packet.command).unwrap(),
            json!({"op":"accept_transfer","manifest":self.manifest(bytes)})
        );
        assert!(!saved["manifest"].is_null() && saved["result"].is_null());
        packet.ok(json!({"transfer_id":saved["transfer_id"],"state":"receiving"}));
    }
    pub async fn activate_packet(&mut self) -> Packet {
        let packet = self.destination.next().await;
        let saved = journal(&self.args);
        assert_eq!(
            serde_json::to_value(&packet.command).unwrap(),
            json!({"op":"activate_transfer","command_id":saved["activate_command"],"transfer_id":saved["transfer_id"]})
        );
        packet
    }
    pub async fn recovery_status(
        &mut self,
        state: TransferStatusState,
        received_bytes: Option<u64>,
        completion: Option<Uuid>,
    ) {
        use sha2::{Digest, Sha256};
        let packet = self.destination.next().await;
        assert!(matches!(packet.command, VesselCommand::Capabilities));
        packet.ok(json!({"features":["signed_transfer_status_v1"]}));
        let saved = journal(&self.args);
        let manifest: SignedArtifact<TransferManifest> =
            serde_json::from_value(saved["manifest"].clone()).unwrap();
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&manifest).unwrap())
        );
        let packet = self.destination.next().await;
        assert_eq!(
            serde_json::to_value(&packet.command).unwrap(),
            json!({"op":"transfer_status",
            "transfer_id":saved["transfer_id"],"activate_command_id":saved["activate_command"],
            "expected_manifest_digest":digest,"manifest":manifest})
        );
        let status = TransferStatus {
            transfer_id: manifest.payload.transfer_id,
            activate_command_id: serde_json::from_value(saved["activate_command"].clone()).unwrap(),
            manifest_digest: digest,
            source_vessel_id: self.source.identity.vessel_id,
            destination_vessel_id: self.destination.identity.vessel_id,
            session_id: self.args.session,
            artifact_sha256: manifest.payload.artifact_sha256,
            artifact_bytes: manifest.payload.artifact_bytes,
            received_bytes,
            state,
            completion: completion.map(|incarnation| TransferCompletion {
                session_id: self.args.session,
                incarnation,
            }),
        };
        packet.ok(serde_json::to_value(status).unwrap());
    }
    pub async fn chunk(&mut self, offset: u64, bytes: &[u8], total: u64) {
        let packet = self.source.next().await;
        let saved = journal(&self.args);
        assert_eq!(
            serde_json::to_value(&packet.command).unwrap(),
            json!({"op":"transfer_chunk","transfer_id":saved["transfer_id"],"offset":offset,"limit":65536})
        );
        packet.ok(json!({"transfer_id":saved["transfer_id"],"offset":offset,"total_bytes":total,"next_offset":offset+bytes.len()as u64,"data":STANDARD.encode(bytes)}));
        let packet = self.destination.next().await;
        assert_eq!(
            serde_json::to_value(&packet.command).unwrap(),
            json!({"op":"upload_transfer_chunk","transfer_id":saved["transfer_id"],"offset":offset,"data":STANDARD.encode(bytes)})
        );
        packet.ok(json!({"transfer_id":saved["transfer_id"],"next_offset":offset+bytes.len()as u64,"stored_bytes":offset+bytes.len()as u64}));
    }
    pub async fn finish(mut self) {
        self.source.finish().await;
        self.destination.finish().await;
        // The operation lock must no longer be held after its future returns.
        let _lock = lock(&self.args.journal).unwrap();
        drop(_lock);
        fs::remove_dir_all(&self.root).unwrap();
    }
}
pub async fn result(task: tokio::task::JoinHandle<Result<Value>>) -> Result<Value> {
    tokio::time::timeout(WAIT, task)
        .await
        .expect("bounded courier completion")
        .unwrap()
}
