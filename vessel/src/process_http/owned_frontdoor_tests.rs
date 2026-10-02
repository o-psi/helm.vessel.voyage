//! Real private ordinary Vessel + production public HTTP/native duplex in an
//! independent owned binary-test child. No Voyage execution/provider/root scope.
use super::*;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use voyage_protocol::{
    duplex::{ClientFrame, ServerFrame},
    process::{ApprovedWorkspace, ConnectionGrant, ProcessRight},
};
const ROLE: &str = "VESSEL_FRONTDOOR_OWNED_ROLE_353";
const TEST: &str =
    "process_http::owned_frontdoor_tests::owned_ordinary_public_grant_and_socket_lifecycle";
const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ORIGIN: &str = "https://private-fixture.invalid";
fn start(pid: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()?
        .rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}
struct Owner {
    child: Option<Child>,
    pid: u32,
    ticks: String,
    root: PathBuf,
}
impl Owner {
    fn spawn(root: &std::path::Path) -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        let output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("owned-child.log"))
            .unwrap();
        command
            .current_dir(root)
            .args(["--exact", TEST, "--nocapture"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", root.join("home"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_STATE_HOME", root.join("private-state"))
            .env(ROLE, root)
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output);
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let child = command.spawn().unwrap();
        let pid = child.id();
        let ticks = start(pid).expect("owned child start identity");
        assert_eq!(
            fs::metadata(format!("/proc/{pid}")).unwrap().uid(),
            unsafe { libc::geteuid() }
        );
        assert_eq!(
            fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
            std::env::current_exe().unwrap()
        );
        Self {
            child: Some(child),
            pid,
            ticks,
            root: root.into(),
        }
    }
    async fn finish(&mut self) {
        assert_eq!(start(self.pid).as_ref(), Some(&self.ticks));
        assert_eq!(
            fs::read_link(format!("/proc/{}/exe", self.pid)).unwrap(),
            std::env::current_exe().unwrap()
        );
        assert_eq!(unsafe { libc::kill(self.pid as i32, libc::SIGTERM) }, 0);
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let child = self.child.as_mut().unwrap();
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "private child evidence retained");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owned service retirement unconfirmed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.child.take();
        assert!(!std::path::Path::new(&format!("/proc/{}", self.pid)).exists());
        assert!(!self.root.join("vessel/process-http.json").exists());
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if start(self.pid).as_ref() == Some(&self.ticks)
                && fs::read_link(format!("/proc/{}/exe", self.pid)).ok()
                    == std::env::current_exe().ok()
            {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
fn write(path: &std::path::Path, value: &impl serde::Serialize) {
    let bytes = serde_json::to_vec(value).unwrap();
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    use std::io::Write;
    f.write_all(&bytes).unwrap();
    f.sync_all().unwrap();
}
async fn actor(root: PathBuf) {
    assert_ne!(unsafe { libc::geteuid() }, 0, "ordinary fixture only");
    let mut terminate =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
    let current = std::env::current_exe().unwrap();
    let voyage = current.parent().unwrap().parent().unwrap().join("voyage");
    assert!(
        voyage.is_file(),
        "current instrumented Voyage entrypoint required; no stale binary"
    );
    let directory = root.join("vessel");
    let service_root = directory.clone();
    let mut service =
        tokio::spawn(async move { vessel::process::serve(service_root, voyage).await });
    let capabilities = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(r) = vessel::process::exchange(
                &directory,
                &VesselRequest {
                    protocol: 1,
                    command: VesselCommand::Capabilities,
                },
            )
            .await
            {
                if r.error.is_none() {
                    break r.result;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let vessel_id = Uuid::parse_str(capabilities["vessel_id"].as_str().unwrap()).unwrap();
    let state = AppState {
        process_route: Some(ProcessRoute::Local(directory)),
        database: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        browser_credentials: Default::default(),
        operator_token_hash: None,
        public_origin: Some(ORIGIN.into()),
    };
    let app = axum::Router::new()
        .route(
            voyage_protocol::vessel::COMMAND_PATH,
            axum::routing::post(command).layer(axum::middleware::from_fn_with_state(
                state.clone(),
                boundary,
            )),
        )
        .route(
            voyage_protocol::duplex::SOCKET_PATH,
            axum::routing::get(socket).layer(axum::middleware::from_fn_with_state(
                state.clone(),
                boundary,
            )),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, rx) = tokio::sync::oneshot::channel();
    let mut public = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
    });
    write(
        &root.join("ready.json"),
        &serde_json::json!({"address":address.to_string(),"vessel_id":vessel_id,"pid":std::process::id()}),
    );
    tokio::select! {_ = terminate.recv()=>{}, _ = tokio::time::sleep(Duration::from_secs(90))=>panic!("owned frontdoor lifetime exceeded")};
    let _ = shutdown.send(());
    tokio::time::timeout(Duration::from_secs(10), &mut public)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), &mut service)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
struct Wire(tokio::net::TcpStream);
impl Wire {
    async fn open(address: std::net::SocketAddr, headers: &str) -> (Self, u16) {
        tokio::time::timeout(Duration::from_secs(4),async{let mut s=tokio::net::TcpStream::connect(address).await.unwrap();s.write_all(format!("GET {} HTTP/1.1\r\nHost: {address}\r\n{headers}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: voyage.vessel.v1\r\n\r\n",voyage_protocol::duplex::SOCKET_PATH).as_bytes()).await.unwrap();let mut out=Vec::new();while !out.ends_with(b"\r\n\r\n"){out.push(s.read_u8().await.unwrap());assert!(out.len()<8192);}let status=String::from_utf8(out).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();(Self(s),status)}).await.unwrap()
    }
    async fn frame(&mut self, opcode: u8, bytes: &[u8]) {
        assert!(bytes.len() <= voyage_protocol::duplex::MAX_FRAME_BYTES + 1);
        let mut packet = vec![0x80 | opcode];
        if bytes.len() < 126 {
            packet.push(0x80 | bytes.len() as u8);
        } else if bytes.len() <= u16::MAX as usize {
            packet.push(0xfe);
            packet.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        } else {
            packet.push(0xff);
            packet.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        }
        let mask = [0x12, 0x34, 0x56, 0x78];
        packet.extend_from_slice(&mask);
        packet.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        tokio::time::timeout(Duration::from_secs(3), self.0.write_all(&packet))
            .await
            .unwrap()
            .unwrap();
    }
    async fn send(&mut self, frame: &ClientFrame) {
        self.frame(1, &serde_json::to_vec(frame).unwrap()).await;
    }
    async fn next(&mut self) -> Option<ServerFrame> {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let first = match self.0.read_u8().await {
                    Ok(b) => b,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                        ) =>
                    {
                        return None;
                    }
                    Err(e) => panic!("private socket read {e}"),
                };
                let second = self.0.read_u8().await.unwrap();
                assert_eq!(second & 0x80, 0);
                let len = match second & 127 {
                    126 => self.0.read_u16().await.unwrap() as usize,
                    127 => usize::try_from(self.0.read_u64().await.unwrap()).unwrap(),
                    n => n as usize,
                };
                assert!(len <= voyage_protocol::duplex::MAX_FRAME_BYTES);
                let mut bytes = vec![0; len];
                self.0.read_exact(&mut bytes).await.unwrap();
                match first & 15 {
                    1 => return Some(serde_json::from_slice(&bytes).unwrap()),
                    8 => return None,
                    9 => self.frame(10, &bytes).await,
                    10 => {}
                    _ => panic!("unsupported private frame"),
                }
            }
        })
        .await
        .unwrap()
    }
    async fn hello(&mut self, expected: Uuid) {
        assert!(
            matches!(self.next().await,Some(ServerFrame::Hello{vessel_id,..})if vessel_id==expected)
        );
    }
    async fn capabilities(&mut self, id: Uuid) {
        self.send(&ClientFrame::Command {
            request_id: id,
            request: Box::new(VesselRequest {
                protocol: 1,
                command: VesselCommand::Capabilities,
            }),
        })
        .await;
        assert!(
            matches!(self.next().await,Some(ServerFrame::Reply{request_id,response})if request_id==id&&response.error.is_none()&&!response.outcome_unknown)
        );
    }
}
fn grant(root: &std::path::Path, vessel_id: Uuid) -> ConnectionGrant {
    let workspace = root.join("workspace");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&workspace)
        .unwrap();
    let g = ConnectionGrant {
        full_access: false,
        schema_version: 1,
        grant_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        vessel_id,
        revision: 1,
        rights: vec![ProcessRight::Catalogue, ProcessRight::Observe],
        accounts: vec![],
        enrollment_connections: vec![],
        expires_at_ms: u64::MAX,
        revoked: false,
        token_hash: format!("{:x}", Sha256::digest(TOKEN.as_bytes())),
        workspaces: vec![ApprovedWorkspace {
            id: Uuid::new_v4(),
            name: "Owned workspace".into(),
            path: workspace,
            provider_ready: None,
        }],
    };
    let dir = root.join("vessel/access/connections");
    fs::create_dir_all(&dir).unwrap();
    write(&dir.join(format!("{}.json", g.grant_id)), &g);
    g
}
fn headers(g: &ConnectionGrant) -> String {
    format!(
        "Authorization: Bearer {TOKEN}\r\nx-voyage-grant: {}\r\nx-voyage-vessel: {}\r\n",
        g.grant_id, g.vessel_id
    )
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_ordinary_public_grant_and_socket_lifecycle() {
    if let Some(root) = std::env::var_os(ROLE) {
        actor(root.into()).await;
        return;
    }
    assert_ne!(unsafe { libc::geteuid() }, 0);
    for case in 0..19 {
        let root = std::env::temp_dir().join(format!("vfd-{}", Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        for d in ["home", "data", "private-state"] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(root.join(d))
                .unwrap();
        }
        let mut owner = Owner::spawn(&root);
        let ready = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                if let Ok(b) = fs::read(root.join("ready.json")) {
                    break serde_json::from_slice::<serde_json::Value>(&b).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(ready["pid"], owner.pid);
        let address = ready["address"].as_str().unwrap().parse().unwrap();
        let vessel_id = Uuid::parse_str(ready["vessel_id"].as_str().unwrap()).unwrap();
        let mut g = grant(&root, vessel_id);
        let before = fs::read(
            root.join("vessel/access/connections")
                .join(format!("{}.json", g.grant_id)),
        )
        .unwrap();
        let (mut wire, status) = Wire::open(address, &headers(&g)).await;
        assert_eq!(status, 101);
        wire.hello(vessel_id).await;
        wire.capabilities(Uuid::new_v4()).await;
        if case < 8 {
            match case {
                0 => g.revision += 1,
                1 => g.principal_id = Uuid::new_v4(),
                2 => g.rights.push(ProcessRight::History),
                3 => g.revoked = true,
                4 => g.expires_at_ms = 0,
                5 => g.workspaces[0].id = Uuid::new_v4(),
                6 => g.accounts.push(Uuid::new_v4()),
                _ => g.enrollment_connections.push(Uuid::new_v4()),
            };
            write(
                &root
                    .join("vessel/access/connections")
                    .join(format!("{}.json", g.grant_id)),
                &g,
            );
            wire.send(&ClientFrame::Command {
                request_id: Uuid::new_v4(),
                request: Box::new(VesselRequest {
                    protocol: 1,
                    command: VesselCommand::Capabilities,
                }),
            })
            .await;
            assert!(
                wire.next().await.is_none(),
                "changed authority cannot widen a live socket"
            );
        } else {
            match case {
                8 => {
                    let id = Uuid::new_v4();
                    wire.capabilities(id).await;
                    wire.send(&ClientFrame::Command {
                        request_id: id,
                        request: Box::new(VesselRequest {
                            protocol: 1,
                            command: VesselCommand::Capabilities,
                        }),
                    })
                    .await;
                    assert!(wire.next().await.is_none(), "correlation cannot replay");
                }
                9 => {
                    wire.send(&ClientFrame::Command {
                        request_id: Uuid::nil(),
                        request: Box::new(VesselRequest {
                            protocol: 1,
                            command: VesselCommand::Capabilities,
                        }),
                    })
                    .await;
                    assert!(wire.next().await.is_none());
                }
                10 => {
                    wire.send(&ClientFrame::Command {
                        request_id: Uuid::new_v4(),
                        request: Box::new(VesselRequest {
                            protocol: 99,
                            command: VesselCommand::Capabilities,
                        }),
                    })
                    .await;
                    assert!(wire.next().await.is_none());
                }
                11 => {
                    wire.frame(2, b"binary-private-sentinel").await;
                    assert!(wire.next().await.is_none());
                }
                12 => {
                    wire.frame(1, &vec![b'x'; voyage_protocol::duplex::MAX_FRAME_BYTES + 1])
                        .await;
                    assert!(wire.next().await.is_none());
                }
                13 => {
                    let id = Uuid::new_v4();
                    wire.send(&ClientFrame::Command {
                        request_id: id,
                        request: Box::new(VesselRequest {
                            protocol: 1,
                            command: VesselCommand::Granted {
                                expected_authority_fingerprint: None,
                                expected_vessel_id: Some(vessel_id),
                                grant_id: g.grant_id,
                                token: TOKEN.into(),
                                command: Box::new(VesselCommand::Capabilities),
                            },
                        }),
                    })
                    .await;
                    assert!(
                        matches!(wire.next().await,Some(ServerFrame::Reply{request_id,response})if request_id==id&&response.error.as_deref()==Some("private envelope refused")&&!response.outcome_unknown)
                    );
                }
                14 => {
                    let session = Uuid::new_v4();
                    let incarnation = Uuid::new_v4();
                    let sub = voyage_protocol::vessel::VesselEventSubscription {
                        session_id: session,
                        incarnation,
                        after: 0,
                        projection: None,
                    };
                    wire.send(&ClientFrame::Subscribe {
                        request_id: Uuid::new_v4(),
                        request: voyage_protocol::vessel::VesselEventRequest {
                            protocol: 1,
                            subscriptions: vec![sub.clone(), sub],
                        },
                    })
                    .await;
                    assert!(
                        wire.next().await.is_none(),
                        "duplicate subscription target refuses before observation"
                    );
                }
                15 | 16 => {
                    let id = Uuid::new_v4();
                    let session = Uuid::new_v4();
                    wire.send(&ClientFrame::Subscribe {
                        request_id: id,
                        request: voyage_protocol::vessel::VesselEventRequest {
                            protocol: 1,
                            subscriptions: vec![voyage_protocol::vessel::VesselEventSubscription {
                                session_id: session,
                                incarnation: Uuid::new_v4(),
                                after: 0,
                                projection: None,
                            }],
                        },
                    })
                    .await;
                    assert!(
                        matches!(wire.next().await,Some(ServerFrame::Subscribed{request_id})if request_id==id)
                    );
                    assert!(
                        matches!(wire.next().await,Some(ServerFrame::Event{subscription_id,event})if subscription_id==id&&event.session_id==session&&event.error.is_some()&&event.result.is_null()&&!event.outcome_unknown)
                    );
                    if case == 16 {
                        wire.send(&ClientFrame::Unsubscribe {
                            subscription_id: id,
                        })
                        .await;
                    }
                    wire.capabilities(Uuid::new_v4()).await;
                }
                17 => {
                    wire.send(&ClientFrame::Unsubscribe {
                        subscription_id: Uuid::new_v4(),
                    })
                    .await;
                    assert!(wire.next().await.is_none());
                }
                _ => {
                    tokio::time::sleep(Duration::from_secs(
                        voyage_protocol::duplex::DEADLINE_SECONDS
                            + voyage_protocol::duplex::HEARTBEAT_SECONDS
                            + 1,
                    ))
                    .await;
                    assert!(
                        wire.next().await.is_none(),
                        "application liveness expires without incoming control traffic"
                    );
                }
            };
            assert_eq!(
                fs::read(
                    root.join("vessel/access/connections")
                        .join(format!("{}.json", g.grant_id))
                )
                .unwrap(),
                before
            );
        }
        drop(wire);
        assert!(
            fs::read_dir(root.join("vessel/sessions"))
                .unwrap()
                .next()
                .is_none()
        );
        owner.finish().await;
        assert!(fs::metadata(root.join("owned-child.log")).unwrap().len() < 1024 * 1024);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn current_frozen_pin_cannot_disappear_and_legacy_documents_do_not_serialize_an_expectation() {
    let make = |authority| SocketBackend {
        route: ProcessRoute::Local(PathBuf::from("/private/not-opened")),
        expected_vessel_id: Some(Uuid::new_v4()),
        grant_id: Uuid::new_v4(),
        token: TOKEN.into(),
        authority,
        browser_sockets: Default::default(),
    };
    let pin = "f".repeat(64);
    let current = make(
        serde_json::json!({"authorization_fingerprint":pin,"features":["saved_authority_pin"]}),
    );
    assert_eq!(current.expected_authority().unwrap(), Some(pin.clone()));
    for value in [
        serde_json::json!({"features":["saved_authority_pin"]}),
        serde_json::json!({"authorization_fingerprint":"bad","features":[]}),
        serde_json::json!({"authorization_fingerprint":null,"features":[]}),
    ] {
        assert!(make(value).expected_authority().is_err());
    }
    assert_eq!(
        make(serde_json::json!({"scope":"owner","features":[]}))
            .expected_authority()
            .unwrap(),
        None
    );
    let original = vessel::process::gateway_ipc::GrantAuth {
        expected_authority_fingerprint: Some(pin),
        expected_vessel_id: Some(Uuid::new_v4()),
        grant_id: Uuid::new_v4(),
        token: TOKEN.into(),
    };
    let changed = original.clone();
    assert!(original == changed);
    let mut other = original.clone();
    other.expected_authority_fingerprint = None;
    assert!(
        original != other,
        "private open/socket bound context equality is not relaxed"
    );
    let legacy = vessel::process::gateway_ipc::GrantAuth {
        expected_authority_fingerprint: None,
        expected_vessel_id: None,
        grant_id: Uuid::new_v4(),
        token: TOKEN.into(),
    };
    let bytes = serde_json::to_value(&legacy).unwrap();
    assert!(bytes.get("expected_authority_fingerprint").is_none());
    let decoded: vessel::process::gateway_ipc::GrantAuth = serde_json::from_value(bytes).unwrap();
    assert!(decoded == legacy);
}
