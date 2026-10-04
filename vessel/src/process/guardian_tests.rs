use super::*;

#[test]
fn ordinary_process_cannot_enter_privileged_guardian() {
    if unsafe { libc::geteuid() } != 0 {
        assert!(
            run(Args {
                directory: "/not-a-control-root".into(),
                session: Uuid::new_v4(),
                incarnation: Uuid::new_v4()
            })
            .unwrap_err()
            .to_string()
            .contains("requires root")
        );
    }
}

#[tokio::test]
#[ignore = "requires an explicitly disposable native-root fixture and Vessel/Voyage executables"]
async fn native_guardian_attests_pipe_launch_and_proves_descendant_cleanup() {
    use crate::process::{database, launch, registry, routing};
    use std::{
        fs,
        num::NonZeroU64,
        os::fd::{FromRawFd, OwnedFd},
        os::unix::fs::{MetadataExt, PermissionsExt},
    };
    use voyage_protocol::{execution_identity::*, process::*};
    assert_eq!(
        std::env::var("VOYAGE_DISPOSABLE_ROOT_FIXTURE").as_deref(),
        Ok("1")
    );
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let vessel = PathBuf::from(std::env::var_os("VOYAGE_TEST_VESSEL_EXECUTABLE").unwrap());
    let voyage = PathBuf::from(std::env::var_os("VOYAGE_TEST_CATALOGUE_EXECUTABLE").unwrap());
    launch::protected_binary(&vessel).unwrap();
    launch::protected_binary(&voyage).unwrap();
    let id = |flag| -> u32 {
        String::from_utf8(
            Command::new("/usr/bin/id")
                .args([flag, "voyageordinary"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .parse()
        .unwrap()
    };
    let uid = id("-u");
    let gid = id("-g");
    assert_ne!(uid, 0);
    struct Fixture(PathBuf, std::sync::Mutex<Vec<(OwnedFd, bool)>>);
    impl Fixture {
        fn owned(&self, mut child: std::process::Child, guardian: bool) -> std::process::Child {
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id() as i32, 0) };
            if fd < 0 {
                let _ = child.kill();
                let _ = child.wait();
                panic!("native fixture child identity pin unavailable");
            }
            self.1
                .lock()
                .unwrap()
                .push((unsafe { OwnedFd::from_raw_fd(fd as i32) }, guardian));
            child
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // These are fixture-owned protected metadata, never user journals.
            // Request every admitted owner stop before retiring the supervisor.
            let control = self.0.join("control");
            let mut admissions = Vec::new();
            if let Ok(sessions) = fs::read_dir(control.join("guardians")) {
                for session in sessions.flatten() {
                    let Ok(sid) = Uuid::parse_str(&session.file_name().to_string_lossy()) else {
                        continue;
                    };
                    if let Ok(incarnations) = fs::read_dir(session.path()) {
                        for incarnation in incarnations.flatten() {
                            let Ok(inc) =
                                Uuid::parse_str(&incarnation.file_name().to_string_lossy())
                            else {
                                continue;
                            };
                            if incarnation.path().join("admission.json").is_file() {
                                let _ = request_stop(&control, sid, inc);
                                admissions.push((sid, inc));
                            }
                        }
                    }
                }
            }
            use std::os::fd::AsRawFd;
            let children = self.1.get_mut().unwrap_or_else(|error| error.into_inner());
            for (fd, guardian) in children.iter() {
                if !guardian {
                    unsafe {
                        libc::syscall(
                            libc::SYS_pidfd_send_signal,
                            fd.as_raw_fd(),
                            libc::SIGTERM,
                            std::ptr::null::<libc::siginfo_t>(),
                            0,
                        );
                    }
                }
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(25);
            loop {
                let exited = children.iter().all(|(fd, _)| {
                    let mut poll = libc::pollfd {
                        fd: fd.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    unsafe { libc::poll(&mut poll, 1, 0) > 0 && poll.revents & libc::POLLIN != 0 }
                });
                if exited || std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            let mut unresolved = false;
            for (fd, _) in children.iter() {
                let mut poll = libc::pollfd {
                    fd: fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                if unsafe { libc::poll(&mut poll, 1, 0) } <= 0 {
                    unresolved = true;
                }
            }
            unresolved |= admissions
                .iter()
                .any(|(sid, inc)| !cleanup_observed(&control, *sid, *inc).unwrap_or(false));
            // Preserve exact receipts/diagnostics on panic or unresolved cleanup.
            // Native harness teardown may retire the guest; this is not cleanup proof.
            if std::thread::panicking() || unresolved {
                eprintln!(
                    "native fixture evidence retained at {}; cleanup_unresolved={unresolved}",
                    self.0.display()
                );
            } else {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }
    let fixture = Fixture(
        PathBuf::from(format!(
            "/opt/vg-{}",
            &Uuid::new_v4().simple().to_string()[..8]
        )),
        std::sync::Mutex::new(Vec::new()),
    );
    fs::create_dir(&fixture.0).unwrap();
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o755)).unwrap();
    let control = fixture.0.join("control");
    let runtime = fixture.0.join("runtime");
    let workspace = fixture.0.join("workspace");
    registry::private_directory(&control).unwrap();
    registry::private_directory(&control.join("sessions")).unwrap();
    fs::create_dir(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o711)).unwrap();
    registry::private_directory(&workspace).unwrap();
    std::os::unix::fs::chown(&workspace, Some(uid), Some(gid)).unwrap();
    database::initialize(&control).await.unwrap();
    RootDirectory::open(&control)
        .unwrap()
        .publish_new(
            "runtime-layout.json".as_ref(),
            &serde_json::to_vec(&serde_json::json!({"version":1,"runtime_root":runtime})).unwrap(),
            4096,
        )
        .unwrap();
    let identity = ConfiguredExecutionIdentity {
        identity: IdentityRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        label: "Guardian ordinary fixture".into(),
        user_name: "voyageordinary".into(),
        uid,
        gid,
        supplementary_groups: vec![],
        home: "/home/voyageordinary".into(),
        account_context: AccountContextRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        authority: AuthorityClass::Ordinary,
        enabled: true,
    };
    database::store_identity(&control, &identity).await.unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let (received, mut provider_requests) = tokio::sync::mpsc::unbounded_channel();
    let (disconnected, mut provider_closed) = tokio::sync::mpsc::unbounded_channel();
    struct HeldProvider(tokio::task::JoinHandle<()>);
    impl Drop for HeldProvider {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let provider_guard = HeldProvider(tokio::spawn(async move {
        let mut peers = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut stream, peer) = accepted.unwrap();
                    assert!(peer.ip().is_loopback());
                    let received = received.clone(); let disconnected = disconnected.clone();
                    peers.spawn(async move {
                        let mut bytes = Vec::new();
                        let mut chunk = [0; 4096];
                        let (header_end, length) = tokio::time::timeout(Duration::from_secs(10), async {
                            loop {
                                let n = stream.read(&mut chunk).await.unwrap();
                                assert!(n > 0);
                                bytes.extend_from_slice(&chunk[..n]);
                                assert!(bytes.len() <= 1024 * 1024);
                                if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                                    let header = std::str::from_utf8(&bytes[..end]).unwrap();
                                    let length = header.lines().find_map(|line| {
                                        line.to_ascii_lowercase().strip_prefix("content-length:")
                                            .map(|value| value.trim().parse::<usize>().unwrap())
                                    }).unwrap_or(0);
                                    assert!(length <= 1024 * 1024);
                                    if bytes.len() >= end + 4 + length {
                                        break (end, length);
                                    }
                                }
                            }
                        }).await.unwrap();
                        let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
                        if header.starts_with("GET /v1/models ") {
                            let body = r#"{"data":[{"id":"fixture-model"}]}"#;
                            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                            return;
                        }
                        assert!(header.starts_with("POST /v1/responses "));
                        assert!(header.to_ascii_lowercase().contains("authorization: bearer synthetic-native-fixture"));
                        let body: serde_json::Value = serde_json::from_slice(&bytes[header_end + 4..header_end + 4 + length]).unwrap();
                        assert_eq!(body["model"], "fixture-model");
                        // A real native provider dispatch is now pending. No
                        // fabricated journal row, goal or execution flag exists.
                        received.send(body).unwrap();
                        let mut probe = [0; 1];
                        let observed = tokio::time::timeout(Duration::from_secs(120), stream.read(&mut probe)).await;
                        let closed = matches!(observed, Ok(Ok(0)))
                            || matches!(observed, Ok(Err(ref error)) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe));
                        let _ = disconnected.send(closed);
                    });
                },
                completed=peers.join_next(), if !peers.is_empty() => {completed.unwrap().unwrap();},
            }
        }
    }));
    let key_name = format!(
        "VOYAGE_GUARDIAN_KEY_{}",
        Uuid::new_v4().simple().to_string().to_ascii_uppercase()
    );
    // Provision only this synthetic key in the executing ordinary user's own
    // private environment record; the root test never parses that record.
    let mut provision = Command::new("/usr/bin/python3");
    provision.arg("-c").arg(r#"import json,os,pathlib,stat,sys,tempfile
p=pathlib.Path(os.environ['HOME'])/'.config'/'helm'; p.mkdir(parents=True,exist_ok=True,mode=0o700)
for d in (p.parent,p):
 m=d.lstat(); assert stat.S_ISDIR(m.st_mode) and not stat.S_ISLNK(m.st_mode) and m.st_uid==os.geteuid(); assert d!=p or m.st_mode&0o077==0
path=p/'account-environment.json'; values={}
try:
 fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC); m=os.fstat(fd); assert stat.S_ISREG(m.st_mode) and m.st_uid==os.geteuid() and m.st_nlink==1 and m.st_mode&0o077==0 and m.st_size<=65536
 with os.fdopen(fd,'r') as f: values=json.load(f)
except FileNotFoundError: pass
assert isinstance(values,dict); values[sys.argv[1]]='synthetic-native-fixture'; raw=json.dumps(values).encode(); assert len(raw)<=65536
fd,temp=tempfile.mkstemp(dir=p); os.fchmod(fd,0o600)
with os.fdopen(fd,'wb') as f: f.write(raw); f.flush(); os.fsync(f.fileno())
os.replace(temp,path); fd=os.open(p,os.O_RDONLY|os.O_DIRECTORY|os.O_CLOEXEC); os.fsync(fd); os.close(fd)
"#).arg(&key_name);
    launch::configure_identity(&mut provision, &identity).unwrap();
    assert!(provision.status().unwrap().success());
    let ordinary = |args: &[&str]| -> Vec<u8> {
        let mut command = Command::new(&vessel);
        command.args(["auth", "accounts"]).args(args);
        launch::configure_identity(&mut command, &identity).unwrap();
        command.env(&key_name, "synthetic-native-fixture");
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    let label = format!("guardian-{}", Uuid::new_v4());
    let connection: serde_json::Value = serde_json::from_slice(&ordinary(&[
        "connect",
        "--label",
        &label,
        "--endpoint",
        &endpoint,
        "--transports",
        "openai-responses",
    ]))
    .unwrap();
    let account: serde_json::Value = serde_json::from_slice(&ordinary(&[
        "add",
        "--connection",
        connection["id"].as_str().unwrap(),
        "--account",
        &label,
        "--env",
        &key_name,
    ]))
    .unwrap();
    let config = workspace.join("launch.json");
    fs::write(&config,serde_json::to_vec(&serde_json::json!({"version":1,"workspace":workspace,"config":{"provider":"openai-responses","model":"fixture-model","api_key_required":false,"base_url":endpoint,"account":{"account_id":account["id"],"connection_id":connection["id"],"identity_generation":account["identity_generation"],"connection_revision":connection["revision"],"transport":"openai_responses"},"access":"read-only","provider_retry_attempts":1,"provider_response_timeout_ms":120000,"context_window":0,"command_timeout_secs":2},"explicit":{"access":"read-only"},"selection":null,"confirmation":null})).unwrap()).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::chown(&config, Some(uid), Some(gid)).unwrap();
    let registration = |binary: PathBuf| ProcessRegistration {
        protocol: PROCESS_PROTOCOL,
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        restart_from: None,
        initialize: None,
        config_path: Some(config.clone()),
        token: "f".repeat(64),
        peer_uids: Some(ProcessPeerUids {
            supervisor: 0,
            runtime: uid,
        }),
        workspace: workspace.clone(),
        state: ProcessState::Starting,
        name: None,
        executable: Some(binary),
    };
    let binding = |r: &ProcessRegistration| ExecutionBinding {
        session_id: r.session_id,
        incarnation: r.incarnation,
        identity: identity.identity.clone(),
        account_context: identity.account_context.clone(),
        peer_uids: r.peer_uids.clone().unwrap(),
        administrator_grant_id: None,
        host_identity_digest: "a".repeat(64),
        policy_digest: "b".repeat(64),
    };
    let spawn = |r: &ProcessRegistration| {
        let mut command = Command::new(&vessel);
        command
            .arg("guard-bound")
            .arg("--directory")
            .arg(&control)
            .arg("--session")
            .arg(r.session_id.to_string())
            .arg("--incarnation")
            .arg(r.incarnation.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(control.join(format!("guardian-{}.stderr", r.incarnation)))
                    .unwrap(),
            ));
        // Seed a real inheritable root capability; the ordinary child must
        // clear it, rather than merely inherit the fixture's usual empty set.
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                #[repr(C)]
                struct Header {
                    version: u32,
                    pid: i32,
                }
                #[repr(C)]
                #[derive(Clone, Copy)]
                struct Data {
                    effective: u32,
                    permitted: u32,
                    inheritable: u32,
                }
                let header = Header {
                    version: 0x2008_0522,
                    pid: 0,
                };
                let mut data = [Data {
                    effective: 0,
                    permitted: 0,
                    inheritable: 0,
                }; 2];
                if libc::syscall(libc::SYS_capget, &header, data.as_mut_ptr()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                data[0].inheritable |= 1;
                if libc::syscall(libc::SYS_capset, &header, data.as_ptr()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        fixture.owned(command.spawn().unwrap(), true)
    };
    async fn wait(child: &mut std::process::Child) -> std::process::ExitStatus {
        for _ in 0..500 {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("guardian did not finish");
    }
    let r = registration(voyage.clone());
    database::admit_with_binding(&control, &r, vec![1], Some(&binding(&r)))
        .await
        .unwrap();
    let directory = runtime.join(r.session_id.to_string());
    let mut guardian = spawn(&r);
    let mut healthy = false;
    for _ in 0..100 {
        if let Ok(response) = routing::forward(&directory, &r, RuntimeCommand::Health).await
            && response.error.is_none()
        {
            healthy = true;
            break;
        }
        if guardian.try_wait().unwrap().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(
        healthy,
        "actual bound Voyage did not become healthy: guardian={}, runtime={}",
        fs::read_to_string(control.join(format!("guardian-{}.stderr", r.incarnation)))
            .unwrap_or_default(),
        fs::read_to_string(directory.join("fixture-runtime.stderr")).unwrap_or_default()
    );
    let initial_snapshot = routing::forward(&directory, &r, RuntimeCommand::Snapshot)
        .await
        .unwrap();
    assert!(initial_snapshot.error.is_none());
    let renamed = routing::forward(
        &directory,
        &r,
        RuntimeCommand::Rename {
            command_id: Uuid::new_v4(),
            expected_revision: initial_snapshot.result["revision"].as_u64().unwrap(),
            expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60_000) as u64,
            name: "Retained bound conversation".into(),
        },
    )
    .await
    .unwrap();
    assert!(renamed.error.is_none());
    assert_ne!(renamed.result["status"], "rejected");
    async fn active_turn(
        directory: &Path,
        registration: &ProcessRegistration,
        requests: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
        prompt: &str,
    ) -> Uuid {
        let snapshot = routing::forward(directory, registration, RuntimeCommand::Snapshot)
            .await
            .unwrap();
        assert!(snapshot.error.is_none());
        let response = routing::forward(
            directory,
            registration,
            RuntimeCommand::Submit {
                budget: None,
                coordination: None,
                command_id: Uuid::new_v4(),
                expected_revision: snapshot.result["revision"].as_u64().unwrap(),
                expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60_000) as u64,
                prompt: prompt.into(),
            },
        )
        .await
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        let run: Uuid = serde_json::from_value(response.result["run_id"].clone()).unwrap();
        let dispatched = tokio::time::timeout(Duration::from_secs(10), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(dispatched.to_string().contains(prompt));
        let running = routing::forward(directory, registration, RuntimeCommand::Snapshot)
            .await
            .unwrap();
        assert_eq!(running.result["run"]["run_id"], run.to_string());
        assert_eq!(running.result["run"]["state"], "running");
        run
    }
    let active_run = active_turn(
        &directory,
        &r,
        &mut provider_requests,
        "guardian active independence fixture",
    )
    .await;
    assert!(
        fs::read_to_string(format!("/proc/{}/status", guardian.id()))
            .unwrap()
            .lines()
            .any(|line| line.starts_with("CapInh:") && line.ends_with("0000000000000001"))
    );
    let observed: ObservedExecution = serde_json::from_slice(
        &RootDirectory::open(&control)
            .unwrap()
            .child("guardians".as_ref())
            .unwrap()
            .child(r.session_id.to_string().as_ref())
            .unwrap()
            .child(r.incarnation.to_string().as_ref())
            .unwrap()
            .read("observed.json".as_ref(), 8192)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(observed.uid, uid);
    assert_eq!(observed.gid, gid);
    assert!(observed.supplementary_groups.is_empty());
    assert_eq!(
        (
            observed.effective_capabilities,
            observed.permitted_capabilities,
            observed.inheritable_capabilities,
            observed.ambient_capabilities
        ),
        (0, 0, 0, 0)
    );
    assert!(observed.no_new_privileges);
    assert!(observed.process_start_ticks > 0);
    assert_eq!(
        fs::metadata(directory.join("registration.json"))
            .unwrap()
            .uid(),
        uid
    );
    assert!(cleanup_observed(&control, r.session_id, r.incarnation).is_err());
    assert!(!wait(&mut spawn(&r)).await.success());
    let guardian_record = RootDirectory::open(&control)
        .unwrap()
        .child("guardians".as_ref())
        .unwrap()
        .child(r.session_id.to_string().as_ref())
        .unwrap()
        .child(r.incarnation.to_string().as_ref())
        .unwrap();
    assert!(
        !stop_requested(
            &guardian_record,
            r.session_id,
            r.incarnation,
            boot().unwrap()
        )
        .unwrap()
    );
    let spawn_service = || {
        let child = Command::new(&vessel)
            .arg("local-serve")
            .arg("--directory")
            .arg(&control)
            .arg("--voyage-binary")
            .arg(&voyage)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(control.join("service.stderr")).unwrap(),
            ))
            .spawn()
            .unwrap();
        fixture.owned(child, false)
    };
    async fn service_request(
        control: &Path,
        command: VesselCommand,
    ) -> voyage_protocol::vessel::VesselResponse {
        crate::process::exchange::exchange(
            control,
            &voyage_protocol::vessel::VesselRequest {
                protocol: voyage_protocol::vessel::VESSEL_API_VERSION,
                command,
            },
        )
        .await
        .unwrap()
    }
    async fn service_ready(control: &Path, session: Uuid, service: &mut std::process::Child) {
        for _ in 0..100 {
            assert!(
                service.try_wait().unwrap().is_none(),
                "service exited: {}",
                fs::read_to_string(control.join("service.stderr")).unwrap_or_default()
            );
            let result = crate::process::exchange::exchange(
                control,
                &voyage_protocol::vessel::VesselRequest {
                    protocol: voyage_protocol::vessel::VESSEL_API_VERSION,
                    command: VesselCommand::Inspect {
                        session_id: session,
                    },
                },
            )
            .await;
            if result.is_ok_and(|reply| reply.error.is_none() && reply.result["state"] == "live") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("bound service did not become ready");
    }
    let mut service = spawn_service();
    service_ready(&control, r.session_id, &mut service).await;
    assert!(
        !control
            .join("sessions")
            .join(r.session_id.to_string())
            .join("registration.json")
            .exists()
    );
    let blocked = service_request(
        &control,
        VesselCommand::Accounts {
            workspace: workspace.clone(),
            transport: None,
        },
    )
    .await;
    assert!(
        blocked.error.is_some() && !blocked.outcome_unknown,
        "missing protected ordinary default must be definitively refused: {:?}",
        blocked.error
    );
    let unconfigured_session = Uuid::new_v4();
    let blocked = service_request(
        &control,
        VesselCommand::Start {
            command_id: Uuid::new_v4(),
            session_id: unconfigured_session,
            workspace: workspace.clone(),
        },
    )
    .await;
    assert!(
        blocked.error.is_some() && !blocked.outcome_unknown,
        "missing protected ordinary default must be definitively refused: {:?}",
        blocked.error
    );
    assert!(
        database::registration(&control, unconfigured_session)
            .await
            .is_err()
    );
    assert!(!runtime.join(unconfigured_session.to_string()).exists());
    // Public configured creation uses the protected ordinary default. It never
    // reads root's account registry or accepts client-selected UID/grant facts.
    RootDirectory::open(&control)
        .unwrap()
        .publish_new(
            "default-execution.json".as_ref(),
            &serde_json::to_vec(&identity).unwrap(),
            16384,
        )
        .unwrap();
    let public_session = Uuid::new_v4();
    let public_command = Uuid::new_v4();
    let public_start = VesselCommand::StartConfigured {
        command_id: public_command,
        session_id: public_session,
        workspace: workspace.clone(),
        config_path: config.clone(),
    };
    let owner_token = "e".repeat(64);
    let owner = ConnectionGrant {
        schema_version: 1,
        full_access: true,
        grant_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        vessel_id: crate::process::identity::public(&control)
            .unwrap()
            .vessel_id,
        revision: 1,
        rights: ProcessRight::all(),
        accounts: vec![],
        enrollment_connections: vec![],
        expires_at_ms: u64::MAX,
        revoked: false,
        token_hash: crate::process::access::store::hash(&owner_token),
        workspaces: vec![],
    };
    registry::private_directory(&control.join("access").join("connections")).unwrap();
    crate::process::access::store::save(
        &crate::process::access::store::connection_path(&control, owner.grant_id),
        &owner,
    )
    .unwrap();
    let owner_command = |command| VesselCommand::Granted {
        expected_authority_fingerprint: None,
        expected_vessel_id: Some(owner.vessel_id),
        grant_id: owner.grant_id,
        token: owner_token.clone(),
        command: Box::new(command),
    };
    let public_created = service_request(&control, owner_command(public_start.clone())).await;
    assert!(public_created.error.is_none(), "{:?}", public_created.error);
    assert_eq!(public_created.result["state"], "live");
    let public_registration = database::registration(&control, public_session)
        .await
        .unwrap();
    assert_eq!(public_registration.peer_uids.as_ref().unwrap().runtime, uid);
    let public_binding = database::execution_binding(&control, public_session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(public_binding.identity, identity.identity);
    assert_eq!(public_binding.account_context, identity.account_context);
    assert!(public_binding.administrator_grant_id.is_none());
    // Receipt replay does not consult a newly missing/default identity file.
    // It observes the original incarnation rather than selecting or launching.
    fs::rename(
        control.join("default-execution.json"),
        control.join("default-execution.saved"),
    )
    .unwrap();
    let public_duplicate = service_request(&control, owner_command(public_start)).await;
    fs::rename(
        control.join("default-execution.saved"),
        control.join("default-execution.json"),
    )
    .unwrap();
    assert!(
        public_duplicate.error.is_none(),
        "{:?}",
        public_duplicate.error
    );
    assert_eq!(
        public_duplicate.result["incarnation"],
        public_created.result["incarnation"]
    );
    let public_resolved = service_request(
        &control,
        VesselCommand::ResolveStart {
            command_id: public_command,
            session_id: public_session,
            workspace: workspace.clone(),
            config_path: Some(config.clone()),
        },
    )
    .await;
    assert!(
        public_resolved.error.is_none(),
        "{:?}",
        public_resolved.error
    );
    assert_eq!(public_resolved.result["status"], "created");
    assert_eq!(
        public_resolved.result["process"]["incarnation"],
        public_created.result["incarnation"]
    );
    // Resolving a not-admitted creation prevents its delayed public start.
    let fenced_session = Uuid::new_v4();
    let fenced_command = Uuid::new_v4();
    let fenced = service_request(
        &control,
        VesselCommand::ResolveStart {
            command_id: fenced_command,
            session_id: fenced_session,
            workspace: workspace.clone(),
            config_path: Some(config.clone()),
        },
    )
    .await;
    assert_eq!(fenced.result["status"], "not_admitted");
    let delayed = service_request(
        &control,
        VesselCommand::StartConfigured {
            command_id: fenced_command,
            session_id: fenced_session,
            workspace: workspace.clone(),
            config_path: config.clone(),
        },
    )
    .await;
    assert!(delayed.error.is_some());
    assert!(
        database::registration(&control, fenced_session)
            .await
            .is_err()
    );
    // This separate owner has no run or held resource. It must retire cleanly
    // through the real idle suspension path, not stay alive for our assertions.
    for _ in 0..200 {
        if cleanup_observed(&control, public_session, public_registration.incarnation)
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(cleanup_observed(&control, public_session, public_registration.incarnation).unwrap());
    let public_record = RootDirectory::open(&control)
        .unwrap()
        .child("guardians".as_ref())
        .unwrap()
        .child(public_session.to_string().as_ref())
        .unwrap()
        .child(public_registration.incarnation.to_string().as_ref())
        .unwrap();
    let public_completion: Completion = serde_json::from_slice(
        &public_record
            .read("completion.json".as_ref(), 8192)
            .unwrap(),
    )
    .unwrap();
    assert!(public_completion.child_exited_successfully);
    assert_eq!(public_completion.child_exit_code, Some(0));
    assert!(public_completion.stop_reason.is_none());
    service.kill().unwrap();
    service.wait().unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(guardian.try_wait().unwrap().is_none());
    assert!(cleanup_observed(&control, r.session_id, r.incarnation).is_err());
    let independent = routing::forward(&directory, &r, RuntimeCommand::Snapshot)
        .await
        .unwrap();
    assert!(independent.error.is_none());
    assert_eq!(independent.result["run"]["run_id"], active_run.to_string());
    assert_eq!(independent.result["run"]["state"], "running");
    assert!(matches!(
        provider_closed.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    assert!(
        !provider_guard.0.is_finished(),
        "held provider task stopped early"
    );
    assert!(
        routing::forward(&directory, &r, RuntimeCommand::Health)
            .await
            .unwrap()
            .error
            .is_none()
    );
    let mut service = spawn_service();
    service_ready(&control, r.session_id, &mut service).await;
    // Tampering with the runtime projection and stop marker cannot change either
    // the in-memory launch authority or the protected cleanup decision.
    let mut attack = Command::new("/usr/bin/python3");
    attack.arg("-c").arg("import pathlib,sys; p=pathlib.Path(sys.argv[1]); (p/'registration.json').write_text('{}'); (p/'stopped.json').write_text('{\"cleanup_observed\":true}');\ntry: pathlib.Path(sys.argv[2]).read_bytes(); raise AssertionError('control leaked')\nexcept PermissionError: pass").arg(&directory).arg(control.join("catalogue.sqlite3"));
    launch::configure_identity(&mut attack, &identity).unwrap();
    assert!(attack.status().unwrap().success());
    assert!(
        routing::forward(&directory, &r, RuntimeCommand::Health)
            .await
            .unwrap()
            .error
            .is_none()
    );
    assert!(cleanup_observed(&control, r.session_id, r.incarnation).is_err());
    let snapshot = service_request(
        &control,
        VesselCommand::Voyage(voyage_protocol::vessel::VoyageRequest {
            session_id: r.session_id,
            incarnation: Some(r.incarnation),
            command: voyage_protocol::vessel::VoyageCommand::Snapshot,
        }),
    )
    .await;
    assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
    assert_eq!(
        snapshot.result["result"]["session_id"],
        r.session_id.to_string()
    );
    assert_eq!(
        snapshot.result["result"]["run"]["run_id"],
        active_run.to_string()
    );
    assert_eq!(
        snapshot.result["result"]["name"],
        "Retained bound conversation"
    );
    let stopped = service_request(
        &control,
        VesselCommand::Stop {
            session_id: r.session_id,
            incarnation: r.incarnation,
        },
    )
    .await;
    assert!(stopped.error.is_none(), "{:?}", stopped.error);
    assert!(guardian_record.read("stop.json".as_ref(), 4096).is_ok());
    assert!(wait(&mut guardian).await.success());
    assert!(
        tokio::time::timeout(Duration::from_secs(5), provider_closed.recv())
            .await
            .unwrap()
            .unwrap(),
        "held provider socket did not observe runtime cancellation"
    );
    assert!(cleanup_observed(&control, r.session_id, r.incarnation).unwrap());
    assert_eq!(
        service_request(
            &control,
            VesselCommand::Inspect {
                session_id: r.session_id
            }
        )
        .await
        .result["state"],
        "stopped"
    );
    let restart_id = Uuid::new_v4();
    let restart = VesselCommand::Restart {
        command_id: restart_id,
        session_id: r.session_id,
        incarnation: r.incarnation,
    };
    let restarted = service_request(&control, restart.clone()).await;
    assert!(restarted.error.is_none(), "{:?}", restarted.error);
    assert_eq!(restarted.result["state"], "live");
    let next = database::registration(&control, r.session_id)
        .await
        .unwrap();
    assert_ne!(next.incarnation, r.incarnation);
    assert_ne!(next.token, r.token);
    assert_eq!(next.restart_from, Some(r.incarnation));
    assert_eq!(
        database::execution_binding(&control, r.session_id)
            .await
            .unwrap()
            .unwrap()
            .incarnation,
        next.incarnation
    );
    let restarted_run = active_turn(
        &directory,
        &next,
        &mut provider_requests,
        "guardian restarted active independence fixture",
    )
    .await;
    let duplicate = service_request(&control, restart.clone()).await;
    assert!(duplicate.error.is_none());
    assert_eq!(
        duplicate.result["incarnation"],
        next.incarnation.to_string()
    );
    let conflict = service_request(
        &control,
        VesselCommand::Restart {
            command_id: restart_id,
            session_id: r.session_id,
            incarnation: next.incarnation,
        },
    )
    .await;
    assert!(conflict.error.is_some());
    let stale = service_request(
        &control,
        VesselCommand::Stop {
            session_id: r.session_id,
            incarnation: r.incarnation,
        },
    )
    .await;
    assert!(stale.error.is_some());
    let overlap = service_request(
        &control,
        VesselCommand::Restart {
            command_id: Uuid::new_v4(),
            session_id: r.session_id,
            incarnation: next.incarnation,
        },
    )
    .await;
    assert!(overlap.error.is_some());
    service.kill().unwrap();
    service.wait().unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(cleanup_observed(&control, next.session_id, next.incarnation).is_err());
    let independent = routing::forward(&directory, &next, RuntimeCommand::Snapshot)
        .await
        .unwrap();
    assert!(independent.error.is_none());
    assert_eq!(
        independent.result["run"]["run_id"],
        restarted_run.to_string()
    );
    assert_eq!(independent.result["run"]["state"], "running");
    let mut service = spawn_service();
    service_ready(&control, r.session_id, &mut service).await;
    let duplicate = service_request(&control, restart).await;
    assert!(duplicate.error.is_none());
    assert_eq!(
        duplicate.result["incarnation"],
        next.incarnation.to_string()
    );
    assert!(
        routing::forward(&directory, &r, RuntimeCommand::Health)
            .await
            .is_err()
    );
    let snapshot_after = routing::forward(&directory, &next, RuntimeCommand::Snapshot)
        .await
        .unwrap();
    assert!(snapshot_after.error.is_none());
    assert_eq!(snapshot_after.result["name"], "Retained bound conversation");
    assert_eq!(
        snapshot_after.result["session_id"],
        snapshot.result["result"]["session_id"]
    );
    let stopped = service_request(
        &control,
        VesselCommand::Stop {
            session_id: next.session_id,
            incarnation: next.incarnation,
        },
    )
    .await;
    assert!(stopped.error.is_none());
    let mut cleaned = false;
    for _ in 0..500 {
        if cleanup_observed(&control, next.session_id, next.incarnation).unwrap_or(false) {
            cleaned = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(cleaned, "service-started guardian did not confirm cleanup");
    assert!(
        tokio::time::timeout(Duration::from_secs(5), provider_closed.recv())
            .await
            .unwrap()
            .unwrap()
    );
    // Simulate a crash after atomic admission but before the launch effect.
    // Retrying its receipt must not invent a live owner or spawn again.
    let previous = database::registration(&control, next.session_id)
        .await
        .unwrap();
    let mut pending = previous.clone();
    pending.command_id = Uuid::new_v4();
    pending.restart_from = Some(previous.incarnation);
    pending.incarnation = Uuid::new_v4();
    pending.token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    pending.state = ProcessState::Starting;
    let pending_command = VesselCommand::Restart {
        command_id: pending.command_id,
        session_id: pending.session_id,
        incarnation: previous.incarnation,
    };
    database::restart_bound(
        &control,
        &previous,
        &pending,
        serde_json::to_vec(&pending_command).unwrap(),
    )
    .await
    .unwrap();
    let uncertain = service_request(&control, pending_command).await;
    assert!(uncertain.error.is_none());
    assert_eq!(
        uncertain.result["incarnation"],
        pending.incarnation.to_string()
    );
    assert_eq!(uncertain.result["state"], "unavailable");
    assert!(
        !control
            .join("guardians")
            .join(pending.session_id.to_string())
            .join(pending.incarnation.to_string())
            .exists()
    );
    assert!(cleanup_observed(&control, pending.session_id, pending.incarnation).is_err());

    service.kill().unwrap();
    service.wait().unwrap();
    assert!(!wait(&mut spawn(&r)).await.success());
    // An ordinary pipe cannot impersonate the privileged launch handoff.
    let mut wrong = Command::new("/usr/bin/python3");
    wrong.arg("-c").arg("import os,sys,struct; r,w=os.pipe(); os.write(w,struct.pack('!I',0)); os.close(w); os.dup2(r,0); os.close(r); os.execv(sys.argv[1],sys.argv[1:])")
        .arg(&voyage).arg("serve-bound").arg("--directory").arg(&directory)
        .arg("--session").arg(r.session_id.to_string()).arg("--incarnation").arg(r.incarnation.to_string())
        .arg("--workspace").arg(&workspace);
    launch::configure_identity(&mut wrong, &identity).unwrap();
    let refused = wrong.output().unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("root-owned private pipe"));
    // A genuine root pipe still requires the exact execution identity. Wrong
    // identity is rejected before interpreting child-owned configuration.
    let mut wrong_identity = Command::new(&voyage);
    wrong_identity
        .arg("serve-bound")
        .arg("--directory")
        .arg(&directory)
        .arg("--session")
        .arg(r.session_id.to_string())
        .arg("--incarnation")
        .arg(r.incarnation.to_string())
        .arg("--workspace")
        .arg(&workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    launch::configure_identity(&mut wrong_identity, &identity).unwrap();
    let mut wrong_child = wrong_identity.spawn().unwrap();
    let mut invalid = r.clone();
    invalid.peer_uids.as_mut().unwrap().runtime += 1;
    send_registration(wrong_child.stdin.take().unwrap(), &invalid).unwrap();
    assert!(!wait(&mut wrong_child).await.success());
    let mut stale_incarnation = r.clone();
    stale_incarnation.incarnation = Uuid::new_v4();
    assert!(!wait(&mut spawn(&stale_incarnation)).await.success());
    // A controlled runtime fixture leaves a double-forked setsid descendant.
    // The guardian must kill and reap it through kernel child ownership.
    let fake = fixture.0.join("runtime-fixture");
    fs::write(
        &fake,
        r#"#!/usr/bin/python3
import json,os,pathlib,struct,sys,time
n=struct.unpack('!I',sys.stdin.buffer.read(4))[0]
r=json.loads(sys.stdin.buffer.read(n)); p=pathlib.Path(sys.argv[sys.argv.index('--directory')+1])
if '--hold' in pathlib.Path(__file__).read_text().splitlines()[1:2]:
 (p/'fixture-ready').write_text(str(os.getpid())); time.sleep(30); sys.exit(0)
pid=os.fork()
if pid==0:
 os.setsid()
 if os.fork(): os._exit(0)
 (p/'escaped-pid').write_text(str(os.getpid()))
 time.sleep(30); os._exit(0)
for _ in range(100):
 if (p/'escaped-pid').exists() and (p/'escaped-pid').read_text().isdigit(): break
 time.sleep(.01)
os._exit(0)
"#,
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let escaped = registration(fake.clone());
    database::admit_with_binding(&control, &escaped, vec![2], Some(&binding(&escaped)))
        .await
        .unwrap();
    assert!(wait(&mut spawn(&escaped)).await.success());
    let pid = fs::read_to_string(
        runtime
            .join(escaped.session_id.to_string())
            .join("escaped-pid"),
    )
    .unwrap();
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    assert!(cleanup_observed(&control, escaped.session_id, escaped.incarnation).unwrap());
    // Kill a live guardian. Pin its exact kernel child before the kill so test
    // teardown never targets a recycled numeric PID. Runtime markers cannot
    // manufacture a completion record after guardian death.
    let source = fs::read_to_string(&fake)
        .unwrap()
        .replacen("#!/usr/bin/python3\n", "#!/usr/bin/python3\n--hold\n", 1)
        .replace("--hold\nimport", "# --hold\nimport")
        .replace(
            "if '--hold' in pathlib.Path(__file__).read_text().splitlines()[1:2]:",
            "if True:",
        );
    fs::write(&fake, source).unwrap();
    let abandoned = registration(fake);
    database::admit_with_binding(&control, &abandoned, vec![3], Some(&binding(&abandoned)))
        .await
        .unwrap();
    let mut guard = spawn(&abandoned);
    let abandoned_directory = runtime.join(abandoned.session_id.to_string());
    for _ in 0..100 {
        if abandoned_directory.join("fixture-ready").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(abandoned_directory.join("fixture-ready").exists());
    let children =
        fs::read_to_string(format!("/proc/{}/task/{}/children", guard.id(), guard.id())).unwrap();
    let pid: u32 = children.trim().parse().unwrap();
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    assert!(raw >= 0);
    let child = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    guard.kill().unwrap();
    guard.wait().unwrap();
    fs::write(
        abandoned_directory.join("stopped.json"),
        b"{\"cleanup_observed\":true}",
    )
    .unwrap();
    assert!(cleanup_observed(&control, abandoned.session_id, abandoned.incarnation).is_err());
    assert!(!wait(&mut spawn(&abandoned)).await.success());
    assert_eq!(
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                child.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        },
        0
    );
    for _ in 0..100 {
        if !PathBuf::from(format!("/proc/{pid}")).exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    assert!(cleanup_observed(&control, abandoned.session_id, abandoned.incarnation).is_err());
    // Initial creation uses the same protected admission/guardian path as a
    // restart. The command receipt and binding precede the external launch;
    // an exact retry observes the owner without spawning a second guardian.
    let fresh = registration(voyage.clone());
    let new_supervisor = || crate::process::service::Supervisor {
        directory: control.clone(),
        binary: voyage.clone(),
        model_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
        devices: voyage_runtime::accounts::device::DeviceService::new(
            voyage_runtime::accounts::Registry::new(control.join("unused-test-accounts")),
            std::sync::Arc::new(|_, _| false),
        ),
        enrollment_workers: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        #[cfg(target_os = "linux")]
        identity_enrollment_starts: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        assignment_locks: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        lifecycle_locks: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        suspended_observations: Default::default(),
        registrations: database::Registrations::new(control.clone()),
    };
    let supervisor = new_supervisor();
    let created = supervisor
        .start_bound_configured(
            fresh.command_id,
            fresh.session_id,
            workspace.clone(),
            config.clone(),
            binding(&fresh),
        )
        .await
        .unwrap();
    assert_eq!(created["state"], "live");
    assert_eq!(created["incarnation"], fresh.incarnation.to_string());
    let admitted = database::registration(&control, fresh.session_id)
        .await
        .unwrap();
    assert_ne!(admitted.token, fresh.token);
    assert_eq!(admitted.peer_uids, fresh.peer_uids);
    assert!(
        !control
            .join("sessions")
            .join(fresh.session_id.to_string())
            .join("registration.json")
            .exists()
    );
    let duplicate = supervisor
        .start_bound_configured(
            fresh.command_id,
            fresh.session_id,
            workspace.clone(),
            config.clone(),
            binding(&fresh),
        )
        .await
        .unwrap();
    assert_eq!(duplicate["incarnation"], created["incarnation"]);
    let mut changed = binding(&fresh);
    changed.policy_digest = "c".repeat(64);
    assert!(
        supervisor
            .start_bound_configured(
                fresh.command_id,
                fresh.session_id,
                workspace.clone(),
                config.clone(),
                changed,
            )
            .await
            .is_err()
    );
    request_stop(&control, fresh.session_id, fresh.incarnation).unwrap();
    let mut fresh_cleaned = false;
    for _ in 0..200 {
        if cleanup_observed(&control, fresh.session_id, fresh.incarnation).unwrap_or(false) {
            fresh_cleaned = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(fresh_cleaned);
    // A crash after the atomic admission and before the guardian effect leaves
    // an exact command receipt. Recovery may inspect it, but must not launch.
    let pending = registration(voyage.clone());
    let pending_command = VesselCommand::StartConfigured {
        command_id: pending.command_id,
        session_id: pending.session_id,
        workspace: workspace.clone(),
        config_path: config.clone(),
    };
    database::admit_with_binding(
        &control,
        &pending,
        serde_json::to_vec(&pending_command).unwrap(),
        Some(&binding(&pending)),
    )
    .await
    .unwrap();
    let recovered = new_supervisor()
        .start_bound_configured(
            pending.command_id,
            pending.session_id,
            workspace.clone(),
            config.clone(),
            binding(&pending),
        )
        .await
        .unwrap();
    assert_eq!(recovered["state"], "unavailable");
    assert_eq!(recovered["incarnation"], pending.incarnation.to_string());
    assert!(
        database::creation_receipt(&control, pending.command_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !control
            .join("guardians")
            .join(pending.session_id.to_string())
            .join(pending.incarnation.to_string())
            .exists()
    );
    // A newly admitted launch cannot survive a configured-identity revision
    // change, even when its saved numeric UID remains the same.
    let stale = registration(voyage.clone());
    database::admit_with_binding(&control, &stale, vec![4], Some(&binding(&stale)))
        .await
        .unwrap();
    let live = registration(voyage.clone());
    database::admit_with_binding(&control, &live, vec![5], Some(&binding(&live)))
        .await
        .unwrap();
    let live_directory = runtime.join(live.session_id.to_string());
    let mut revoked_guardian = spawn(&live);
    let mut ready = false;
    for _ in 0..100 {
        if routing::forward(&live_directory, &live, RuntimeCommand::Health)
            .await
            .is_ok_and(|reply| reply.error.is_none())
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(ready);
    let mut revised = identity.clone();
    revised.identity.revision = NonZeroU64::new(2).unwrap();
    database::store_identity(&control, &revised).await.unwrap();
    assert!(wait(&mut revoked_guardian).await.success());
    assert!(cleanup_observed(&control, live.session_id, live.incarnation).unwrap());
    assert!(
        routing::forward(&live_directory, &live, RuntimeCommand::Health)
            .await
            .is_err()
    );
    let completion = RootDirectory::open(&control)
        .unwrap()
        .child("guardians".as_ref())
        .unwrap()
        .child(live.session_id.to_string().as_ref())
        .unwrap()
        .child(live.incarnation.to_string().as_ref())
        .unwrap()
        .read("completion.json".as_ref(), 4096)
        .unwrap();
    let completion: serde_json::Value = serde_json::from_slice(&completion).unwrap();
    assert_eq!(completion["stop_reason"], "authority_changed");

    assert!(!wait(&mut spawn(&stale)).await.success());
    assert!(!runtime.join(stale.session_id.to_string()).exists());
    let _ = ordinary(&[
        "logout",
        "--account",
        account["id"].as_str().unwrap(),
        "--remove",
    ]);
}
