//! Explicit user→system ownership handoff. Legacy authority is not root authority;
//! private state is exported/imported in the original identity through trusted code.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::Digest;
use std::{
    collections::BTreeMap,
    os::{
        fd::AsRawFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
};
use uuid::Uuid;
use voyage_protocol::{
    execution_identity::ConfiguredExecutionIdentity,
    execution_transition::*,
    migration::{ControlRequest, Frame, Import, Snapshot, UserRequest},
};

const REVIEWS: &str = "/etc/voyage-adoption";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    schema_version: u32,
    operation_id: Uuid,
    phase: String,
    source_directory: PathBuf,
    source_device: u64,
    source_inode: u64,
    source_manifest_digest: String,
    original_user: String,
    uid: u32,
    gid: u32,
    home: PathBuf,
    groups: Vec<u32>,
    source_boot: Uuid,
    bin: PathBuf,
    release: String,
    planned_installation: Record,
    gateway_user: String,
    origin: String,
    key: PathBuf,
    provisioner: String,
    source_credential_key: PathBuf,
    receipt_digest: String,
    snapshot_digest: Option<String>,
    provider_fingerprint: Option<String>,
    target_incarnations: BTreeMap<Uuid, Uuid>,
    freeze_commands: BTreeMap<Uuid, Uuid>,
    commit_commands: BTreeMap<Uuid, Uuid>,
    freezes: BTreeMap<Uuid, PreparedTransitionReceipt>,
    source_units: Vec<String>,
    source_definitions: BTreeMap<String, String>,
}
fn root() -> Result<PathBuf> {
    root_host()?;
    let path = PathBuf::from(REVIEWS);
    service::files::directory(&path, 0, true)?;
    Ok(path)
}
fn directory(id: Uuid) -> Result<PathBuf> {
    ensure!(!id.is_nil(), "adoption identity must be nonnil");
    Ok(root()?.join(id.to_string()))
}
fn load(id: Uuid) -> Result<Review> {
    let path = directory(id)?.join("review.json");
    service::files::check_path(&path, 0)?;
    let m = fs::symlink_metadata(&path)?;
    ensure!(
        m.is_file() && m.uid() == 0 && m.nlink() == 1 && m.mode() & 0o077 == 0,
        "adoption review is not root-private"
    );
    let review: Review = serde_json::from_slice(&files::read(&path, 1024 * 1024)?)?;
    ensure!(
        review.schema_version == 1 && review.operation_id == id,
        "adoption receipt identity conflict"
    );
    Ok(review)
}
fn save(review: &Review) -> Result<()> {
    files::atomic_json(&directory(review.operation_id)?.join("review.json"), review)
}
fn hash<T: Serialize>(value: &T) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn output(review: &Review) {
    println!(
        "{}",
        serde_json::json!({"operation_id":review.operation_id,"phase":review.phase,"review_digest":review.receipt_digest,"source_uid":review.uid,"execution_user":review.original_user,"release_id":review.release,"snapshot_digest":review.snapshot_digest,"sessions":review.target_incarnations.len(),"provider_credentials":"retained in original account; not copied","legacy_pairings":"ordinary only; new root pairing required for administrator authority","process_survival":false})
    );
}
fn boot() -> Result<Uuid> {
    Ok(Uuid::parse_str(
        system_preflight::proc_value("/proc/sys/kernel/random/boot_id", 128)?.trim(),
    )?)
}
fn recheck(review: &Review) -> Result<()> {
    let current = system_preflight::account(&review.original_user)?;
    ensure!(
        current.uid == review.uid
            && current.gid == review.gid
            && current.home == review.home
            && current.groups == review.groups,
        "original account changed since adoption review"
    );
    let gateway = system_preflight::account(&review.gateway_user)?;
    let pinned = &review.planned_installation;
    ensure!(
        gateway.uid == pinned.gateway_uid
            && gateway.gid == pinned.gateway_gid
            && gateway.home == pinned.gateway_home
            && gateway.groups == pinned.gateway_groups,
        "gateway account changed since adoption review"
    );
    let retained = directory(review.operation_id)?.join("retained-user-vessel");
    let source = if retained.try_exists()? {
        retained.as_path()
    } else {
        review.source_directory.as_path()
    };
    let metadata = fs::symlink_metadata(source)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == review.uid
            && metadata.dev() == review.source_device
            && metadata.ino() == review.source_inode,
        "original source directory changed since adoption review"
    );
    service::files::check_path(&review.bin, 0)?;
    let manifest = Manifest::inspect(&review.bin)?;
    ensure!(
        manifest.id()? == review.release && hash(&manifest)? == review.source_manifest_digest,
        "reviewed root-owned archive or compatibility metadata changed"
    );
    Ok(())
}
fn drop_identity(command: &mut Command, review: &Review) -> Result<()> {
    recheck(review)?;
    let uid = review.uid;
    let gid = review.gid;
    let groups = review.groups.clone();
    command
        .env_clear()
        .env("HOME", &review.home)
        .env("USER", &review.original_user)
        .env("LOGNAME", &review.original_user)
        .env("PATH", "/usr/bin:/bin")
        .env("XDG_RUNTIME_DIR", format!("/run/user/{uid}"))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path=/run/user/{uid}/bus"),
        );
    unsafe {
        command.pre_exec(move || {
            if libc::setgroups(groups.len(), groups.as_ptr()) != 0
                || libc::setresgid(gid, gid, gid) != 0
                || libc::prctl(libc::PR_SET_KEEPCAPS, 0, 0, 0, 0) != 0
                || libc::prctl(
                    libc::PR_CAP_AMBIENT,
                    libc::PR_CAP_AMBIENT_CLEAR_ALL,
                    0,
                    0,
                    0,
                ) != 0
                || libc::setresuid(uid, uid, uid) != 0
                || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
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
                version: 0x20080522,
                pid: 0,
            };
            let data = [Data {
                effective: 0,
                permitted: 0,
                inheritable: 0,
            }; 2];
            if libc::syscall(libc::SYS_capset, &header, data.as_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}
// Every early pipe/parse failure retires this owned child. Uncertain journal
// effects remain in receipts and are reconciled by their command UUID.
struct OwnedChild(Child);
impl std::ops::Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl std::ops::DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn peer(review: &Review, control: bool, path: &Path) -> Result<(OwnedChild, UnixStream)> {
    let (parent, child) = UnixStream::pair()?;
    parent.set_read_timeout(Some(Duration::from_secs(300)))?;
    parent.set_write_timeout(Some(Duration::from_secs(45)))?;
    let fd = child.as_raw_fd();
    let executable = review.bin.join("vessel");
    service::files::executable(&executable, 0)?;
    let mut command = Command::new(executable);
    command
        .arg(if control {
            "migration-control"
        } else {
            "migration-user"
        })
        .arg("--directory")
        .arg(path)
        .arg("--pipe-fd")
        .arg(fd.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !control {
        drop_identity(&mut command, review)?;
        command.env("VOYAGE_CREDENTIAL_KEY_FILE", &review.source_credential_key);
    } else {
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("VOYAGE_CREDENTIAL_KEY_FILE", &review.key);
    }
    unsafe {
        command.pre_exec(move || {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let running = command.spawn()?;
    drop(child);
    Ok((OwnedChild(running), parent))
}
fn write<T: Serialize>(pipe: &mut UnixStream, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= 128 * 1024 * 1024,
        "migration request exceeds bound"
    );
    pipe.write_all(&(bytes.len() as u32).to_be_bytes())?;
    pipe.write_all(&bytes)?;
    Ok(())
}
fn read<T: serde::de::DeserializeOwned>(pipe: &mut UnixStream) -> Result<T> {
    let mut len = [0; 4];
    pipe.read_exact(&mut len)?;
    let n = u32::from_be_bytes(len) as usize;
    ensure!(
        n > 0 && n <= 128 * 1024 * 1024,
        "migration response exceeds bound"
    );
    let mut bytes = vec![0; n];
    pipe.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid private migration response"))
}
fn finish(child: &mut Child) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "migration helper failed");
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("migration helper exit unconfirmed");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn user_request(review: &Review, request: &UserRequest) -> Result<Frame> {
    let (mut child, mut channel) = peer(review, false, &review.source_directory)?;
    write(&mut channel, request)?;
    let result = read(&mut channel);
    drop(channel);
    let exited = finish(&mut child);
    let frame = result?;
    exited?;
    Ok(frame)
}
fn options(review: &Review) -> Options {
    Options {
        bin: review.bin.clone(),
        execution_user: review.original_user.clone(),
        gateway_user: review.gateway_user.clone(),
        origin: review.origin.clone(),
        credential_key: review.key.clone(),
        credential_unit: review.provisioner.clone(),
        start: false,
        dry_run: false,
        adoption_source: Some(review.source_directory.clone()),
    }
}
struct SourceOwnership {
    child: OwnedChild,
    channel: UnixStream,
    started: Instant,
    directory: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
}
impl SourceOwnership {
    fn acquire(review: &Review) -> Result<Self> {
        let (child, mut channel) = peer(review, false, &review.source_directory)?;
        write(&mut channel, &UserRequest::HoldOwner)?;
        ensure!(
            matches!(read::<Frame>(&mut channel)?, Frame::OwnerHeld),
            "original supervisor ownership lease unavailable"
        );
        Ok(Self {
            child,
            channel,
            started: Instant::now(),
            directory: review.source_directory.clone(),
            device: review.source_device,
            inode: review.source_inode,
            uid: review.uid,
        })
    }
    fn check(&mut self) -> Result<()> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(600) && self.child.try_wait()?.is_none(),
            "original namespace ownership lease ended; no further migration effects permitted"
        );
        let metadata = fs::symlink_metadata(&self.directory)?;
        ensure!(
            metadata.is_dir()
                && metadata.uid() == self.uid
                && metadata.dev() == self.device
                && metadata.ino() == self.inode,
            "leased original supervisor directory was replaced"
        );
        Ok(())
    }
}
struct SourceLease {
    child: OwnedChild,
    input: Option<std::process::ChildStdin>,
    _output: std::process::ChildStdout,
    started: Instant,
    directory: PathBuf,
    session: Uuid,
    device: u64,
    inode: u64,
    uid: u32,
}
impl SourceLease {
    fn acquire(
        review: &Review,
        freeze: TransitionRequest,
    ) -> Result<(Self, PreparedTransitionReceipt)> {
        let source_directory = freeze.operation.directory().to_owned();
        let mut command = Command::new(review.bin.join("voyage"));
        service::files::executable(&review.bin.join("voyage"), 0)?;
        command
            .arg("transition-helper")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        drop_identity(&mut command, review)?;
        let mut child = OwnedChild(command.spawn()?);
        let mut input = child.stdin.take().context("source lease pipe missing")?;
        let request = TransitionRequest {
            schema: SCHEMA,
            session_id: freeze.session_id,
            source_incarnation: freeze.source_incarnation,
            operation: TransitionOperation::SourceLease {
                directory: freeze.operation.directory().to_owned(),
                freeze: Box::new(freeze),
            },
        };
        let bytes = serde_json::to_vec(&request)?;
        ensure!(
            bytes.len() <= MAX_REQUEST,
            "source lease request exceeds bound"
        );
        input.write_all(&(bytes.len() as u32).to_be_bytes())?;
        input.write_all(&bytes)?;
        let mut output = child
            .stdout
            .take()
            .context("source lease reply pipe missing")?;
        use std::os::fd::{FromRawFd, OwnedFd};
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id() as i32, 0) };
        ensure!(raw >= 0, "source lease process pin unavailable");
        let pidfd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let killer = std::thread::spawn(move || {
            if done_rx.recv_timeout(Duration::from_secs(10)).is_err() {
                unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        pidfd.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    );
                }
            }
        });
        let result = (|| {
            let mut header = [0; 4];
            output.read_exact(&mut header)?;
            let length = u32::from_be_bytes(header) as usize;
            ensure!(
                length > 0 && length <= MAX_RESPONSE,
                "source lease reply bound"
            );
            let mut bytes = vec![0; length];
            output.read_exact(&mut bytes)?;
            match serde_json::from_slice::<TransitionResponse>(&bytes)? {
                TransitionResponse::Prepared { receipt } => Ok(receipt),
                _ => bail!("source lease freeze unavailable"),
            }
        })();
        let _ = done_tx.send(());
        let _ = killer.join();
        let receipt = result?;
        ensure!(
            child.try_wait()?.is_none(),
            "source lease ended before publication"
        );
        Ok((
            Self {
                child,
                input: Some(input),
                _output: output,
                started: Instant::now(),
                directory: source_directory,
                session: receipt.session_id,
                device: receipt.source_directory_device,
                inode: receipt.source_directory_inode,
                uid: receipt.source_uid,
            },
            receipt,
        ))
    }
    fn check(&mut self) -> Result<()> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(600) && self.child.try_wait()?.is_none(),
            "original startup/execution lease ended; no further adoption effects permitted"
        );
        let metadata = fs::symlink_metadata(&self.directory)?;
        ensure!(
            metadata.is_dir()
                && metadata.uid() == self.uid
                && metadata.dev() == self.device
                && metadata.ino() == self.inode,
            "leased original runtime directory was replaced"
        );
        Ok(())
    }
    fn release(mut self) -> Result<()> {
        drop(self.input.take());
        finish(&mut self.child)
    }
}
fn leases_current(owner: &mut SourceOwnership, leases: &mut [SourceLease]) -> Result<()> {
    owner.check()?;
    for lease in leases {
        lease.check()?;
    }
    Ok(())
}
fn observe_snapshot(
    review: &mut Review,
    label: &str,
    owner: Option<&mut SourceOwnership>,
) -> Result<Snapshot> {
    ensure!(
        ["reviewed-source", "current-source", "frozen-source"].contains(&label),
        "invalid private snapshot phase"
    );
    let capture = directory(review.operation_id)?.join(label);
    service::files::directory(&capture, 0, true)?;
    let (mut child, mut channel) = peer(review, false, &review.source_directory)?;
    if let Some(owner) = owner {
        owner.check()?;
        write(&mut channel, &UserRequest::ExportHeld)?;
    } else {
        write(&mut channel, &UserRequest::Export)?;
    }
    let snapshot = match read::<Frame>(&mut channel)? {
        Frame::Snapshot { snapshot } => snapshot,
        _ => bail!("ordinary source snapshot missing"),
    };
    ensure!(
        snapshot.uid == review.uid && snapshot.home == review.home && snapshot.schema_version == 1,
        "ordinary source account mismatch"
    );
    let mut stream_digest = sha2::Sha256::new();
    stream_digest.update(serde_json::to_vec(&snapshot)?);
    let mut artifact_digest = sha2::Sha256::new();
    artifact_digest.update(serde_json::to_vec(&snapshot)?);
    files::write_new(
        &capture.join("snapshot.json"),
        &serde_json::to_vec(&snapshot)?,
    )?;
    let mut total = 0u64;
    let mut index = 0usize;
    loop {
        let frame: Frame = read(&mut channel)?;
        let terminal = matches!(frame, Frame::End { .. });
        match &frame {
            Frame::File {
                session_id,
                relative,
                bytes,
                ..
            } => {
                ensure!(
                    snapshot
                        .sessions
                        .iter()
                        .any(|s| s.session_id == *session_id)
                        && relative.is_relative()
                        && relative
                            .components()
                            .all(|c| matches!(c, std::path::Component::Normal(_))),
                    "source artifact escaped reviewed ordinary session"
                );
                let data = STANDARD.decode(bytes)?;
                ensure!(data.len() <= 65536, "source artifact chunk exceeds bound");
                artifact_digest.update(session_id.as_bytes());
                artifact_digest.update(relative.as_os_str().as_encoded_bytes());
                artifact_digest.update(data);
            }
            Frame::End { sha256 } => ensure!(
                format!("{:x}", artifact_digest.clone().finalize()) == *sha256,
                "source stream checksum mismatch"
            ),
            _ => bail!("unexpected private migration stream frame"),
        }
        let bytes = serde_json::to_vec(&frame)?;
        total += bytes.len() as u64;
        ensure!(
            total <= 12 * 1024 * 1024 * 1024u64 && index <= 200000,
            "migration stream exceeds bound"
        );
        stream_digest.update(&bytes);
        files::write_new(&capture.join(format!("frame-{index:06}.json")), &bytes)?;
        index += 1;
        if terminal {
            break;
        }
    }
    drop(channel);
    finish(&mut child)?;
    review.snapshot_digest = Some(format!("{:x}", stream_digest.finalize()));
    review.provider_fingerprint = Some(snapshot.provider_fingerprint.clone());
    for session in &snapshot.sessions {
        review
            .target_incarnations
            .entry(session.session_id)
            .or_insert_with(Uuid::new_v4);
        review
            .freeze_commands
            .entry(session.session_id)
            .or_insert_with(Uuid::new_v4);
        review
            .commit_commands
            .entry(session.session_id)
            .or_insert_with(Uuid::new_v4);
    }
    Ok(snapshot)
}
fn transition(review: &Review, request: &TransitionRequest) -> Result<TransitionResponse> {
    let mut command = Command::new(review.bin.join("voyage"));
    service::files::executable(&review.bin.join("voyage"), 0)?;
    command
        .arg("transition-helper")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    drop_identity(&mut command, review)?;
    let mut child = OwnedChild(command.spawn()?);
    let mut input = child
        .stdin
        .take()
        .context("private handoff input missing")?;
    let bytes = serde_json::to_vec(request)?;
    ensure!(bytes.len() <= MAX_REQUEST, "handoff request exceeds bound");
    input.write_all(&(bytes.len() as u32).to_be_bytes())?;
    input.write_all(&bytes)?;
    drop(input);
    let stdout = child
        .stdout
        .take()
        .context("private handoff output missing")?;
    let mut output = stdout.take((MAX_RESPONSE + 5) as u64);
    // Child owns no external execution. A bounded parent killer observes exit;
    // an uncertain reply is reconciled by exact command UUID, never replayed.
    use std::os::fd::{FromRawFd, OwnedFd};
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id() as i32, 0) };
    ensure!(raw >= 0, "handoff process identity fence unavailable");
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let killer = std::thread::spawn(move || {
        if done_rx.recv_timeout(Duration::from_secs(10)).is_err() {
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    pidfd.as_raw_fd(),
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            };
        }
    });
    let mut data = Vec::new();
    let read_result = output.read_to_end(&mut data);
    let _ = done_tx.send(());
    let _ = killer.join();
    read_result?;
    finish(&mut child)?;
    ensure!(
        data.len() >= 4
            && u32::from_be_bytes(data[..4].try_into()?) as usize == data.len() - 4
            && data.len() - 4 <= MAX_RESPONSE,
        "handoff reply framing invalid"
    );
    Ok(serde_json::from_slice(&data[4..])?)
}
fn freeze(
    review: &mut Review,
    snapshot: &Snapshot,
    owner: &mut SourceOwnership,
) -> Result<Vec<SourceLease>> {
    ensure!(
        snapshot.sessions.len() <= 64,
        "adoption exceeds bounded 64-session lease capacity; source is retained unchanged"
    );
    let mut leases = Vec::new();
    for session in &snapshot.sessions {
        leases_current(owner, &mut leases)?;
        ensure!(
            !review.freezes.contains_key(&session.session_id),
            "lost source lease requires preactivation rollback; source freeze is not replayed"
        );
        let path = review
            .source_directory
            .join("sessions")
            .join(session.session_id.to_string());
        let facts = match transition(
            review,
            &TransitionRequest {
                schema: SCHEMA,
                session_id: session.session_id,
                source_incarnation: session.incarnation,
                operation: TransitionOperation::Observe {
                    directory: path.clone(),
                },
            },
        )? {
            TransitionResponse::Facts { facts } => facts,
            _ => bail!("retired source journal unavailable"),
        };
        let cfg = match user_request(
            review,
            &UserRequest::PrepareConfig {
                session_id: session.session_id,
            },
        )? {
            Frame::Config { sha256 } => sha256,
            _ => bail!("private original configuration unavailable"),
        };
        ensure!(
            cfg == facts.frozen_config_digest,
            "source configuration changed during adoption review"
        );
        review.phase = "freezing-source".into();
        save(review)?;
        let (lease, receipt) = SourceLease::acquire(
            review,
            TransitionRequest {
                schema: SCHEMA,
                session_id: session.session_id,
                source_incarnation: session.incarnation,
                operation: TransitionOperation::SourceFreeze {
                    directory: path,
                    command_id: review.freeze_commands[&session.session_id],
                    transition_id: review.operation_id,
                    target_incarnation: review.target_incarnations[&session.session_id],
                    expected: facts,
                    target_uid: review.uid,
                    target_gid: review.gid,
                    target_config_digest: cfg,
                    review_digest: review.receipt_digest.clone(),
                },
            },
        )?;
        leases.push(lease);
        review.freezes.insert(session.session_id, receipt);
        save(review)?;
    }
    Ok(leases)
}
fn apply(review: &mut Review) -> Result<()> {
    ensure!(
        review.phase == "ready" && boot()? != review.source_boot,
        "adoption needs exact ready review and observed host boot retirement"
    );
    recheck(review)?;
    review.phase = "capturing-retired-source".into();
    save(review)?;
    let approved_snapshot = review.snapshot_digest.clone();
    let mut owner = SourceOwnership::acquire(review)?;
    let first = observe_snapshot(review, "current-source", Some(&mut owner))?;
    ensure!(
        review.snapshot_digest == approved_snapshot,
        "retired source changed since exact reviewed snapshot; no publication applied"
    );
    let mut leases = freeze(review, &first, &mut owner)?;
    leases_current(&mut owner, &mut leases)?;
    let snapshot = observe_snapshot(review, "frozen-source", Some(&mut owner))?;
    ensure!(
        hash(&first)? == hash(&snapshot)?,
        "source authority/catalogue/preferences changed after reviewed freeze"
    );
    leases_current(&mut owner, &mut leases)?;
    review.phase = "installing-dormant-system".into();
    save(review)?;
    let dir = directory(review.operation_id)?.join("frozen-source");
    let opts = options(review);
    let plan = Plan::prepare(&opts)?;
    leases_current(&mut owner, &mut leases)?;
    apply_install(plan, &opts)?;
    leases_current(&mut owner, &mut leases)?;
    let layout = voyage_storage::protected_linux::RuntimeRoot::open(Path::new(RUNTIME))?;
    for session in &snapshot.sessions {
        layout.create_session(
            session.session_id.to_string().as_ref(),
            review.uid,
            review.gid,
        )?;
    }
    let (mut child, mut channel) = peer(review, false, &review.source_directory)?;
    write(
        &mut channel,
        &UserRequest::Restore {
            runtime_root: RUNTIME.into(),
        },
    )?;
    let mut frames = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|e| e == "json")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("frame-"))
        })
        .collect::<Vec<_>>();
    frames.sort();
    for frame in frames {
        leases_current(&mut owner, &mut leases)?;
        let value: Frame = serde_json::from_slice(&files::read(&frame, 128 * 1024 * 1024)?)?;
        write(&mut channel, &value)?;
    }
    ensure!(
        matches!(read::<Frame>(&mut channel)?, Frame::Complete),
        "ordinary target restore unconfirmed"
    );
    drop(channel);
    finish(&mut child)?;
    for session in &snapshot.sessions {
        leases_current(&mut owner, &mut leases)?;
        let target = Path::new(RUNTIME).join(session.session_id.to_string());
        let request = TransitionRequest {
            schema: SCHEMA,
            session_id: session.session_id,
            source_incarnation: session.incarnation,
            operation: TransitionOperation::TargetCommit {
                directory: target.clone(),
                command_id: review.commit_commands[&session.session_id],
                expected: review.freezes[&session.session_id].clone(),
                target_config_path: target.join("migration-config.json"),
            },
        };
        ensure!(
            matches!(
                transition(review, &request)?,
                TransitionResponse::Committed { .. }
            ),
            "target journal handoff unconfirmed; reconcile exact commit command identity"
        );
    }
    leases_current(&mut owner, &mut leases)?;
    ctl(&["start", &review.provisioner])?;
    let mapped: ConfiguredExecutionIdentity = serde_json::from_slice(&files::read(
        &Path::new(CONTROL).join("default-execution.json"),
        16384,
    )?)?;
    let (mut child, mut channel) = peer(review, true, Path::new(CONTROL))?;
    write(
        &mut channel,
        &ControlRequest::Import {
            import: Import {
                operation_id: review.operation_id,
                review_digest: review.receipt_digest.clone(),
                source_boot: review.source_boot,
                target_boot: boot()?,
                identity: mapped,
                snapshot,
                executable: Path::new(RELEASE_ROOT)
                    .join("releases")
                    .join(&review.release)
                    .join("bin/voyage"),
                target_incarnations: review.target_incarnations.clone(),
            },
        },
    )?;
    ensure!(
        matches!(read::<Frame>(&mut channel)?, Frame::Complete),
        "protected ordinary control import unconfirmed"
    );
    drop(channel);
    finish(&mut child)?;
    // Fence the old supervisor namespace. Preserve compatibility aliases only
    // for session artifacts; no old control/authentication record is exposed.
    let retained = directory(review.operation_id)?.join("retained-user-vessel");
    leases_current(&mut owner, &mut leases)?;
    fs::rename(&review.source_directory, &retained)?;
    owner.directory = retained.clone();
    for lease in &mut leases {
        lease.directory = retained.join("sessions").join(lease.session.to_string());
    }
    leases_current(&mut owner, &mut leases)?;
    fs::DirBuilder::new()
        .mode(0o711)
        .create(&review.source_directory)?;
    let aliases = review.source_directory.join("sessions");
    fs::DirBuilder::new().mode(0o711).create(&aliases)?;
    for session in review.target_incarnations.keys() {
        std::os::unix::fs::symlink(
            Path::new(RUNTIME).join(session.to_string()),
            aliases.join(session.to_string()),
        )?;
    }
    files::write_new(
        &review.source_directory.join("system-adoption-fence.json"),
        &serde_json::to_vec(
            &serde_json::json!({"schema_version":1,"operation_id":review.operation_id,"control_root":CONTROL,"legacy_owner_retired":true}),
        )?,
    )?;
    leases_current(&mut owner, &mut leases)?;
    for lease in leases {
        lease.release()?;
    }
    drop(owner.channel);
    finish(&mut owner.child)?;
    review.phase = "dormant-installed".into();
    save(review)
}
fn apply_install(plan: Plan, options: &Options) -> Result<()> {
    super::apply(plan, options)
}

pub(super) fn run(args: &[String]) -> Result<()> {
    ensure!(
        args.len() >= 2,
        "adopt-user requires action and operation UUID"
    );
    let action = &args[0];
    let id = Uuid::parse_str(&args[1])?;
    let dir = directory(id)?;
    match action.as_str() {
        "prepare" => {
            ensure!(
                !dir.try_exists()?,
                "adoption identity already reserved; inspect status, do not repeat quiescence"
            );
            // Reuse strict explicit installation arguments, then add only the
            // named original-user source. There is no login/SUDO_USER inference.
            let mut install = vec!["install".into()];
            let mut source_credential_key = None;
            let mut supplied = args[2..].iter();
            while let Some(argument) = supplied.next() {
                if argument == "--source-credential-key" {
                    ensure!(
                        source_credential_key.is_none(),
                        "repeated source credential key"
                    );
                    source_credential_key = Some(PathBuf::from(
                        supplied.next().context("source key path required")?,
                    ));
                } else {
                    install.push(argument.clone());
                }
            }
            let source_credential_key=source_credential_key.context("adoption needs original user connection-key path; provider credentials are never imported")?;
            ensure!(
                source_credential_key.is_absolute() && source_credential_key.starts_with("/run"),
                "original user connection key must stay on runtime tmpfs"
            );
            let mut opts = Options::parse(&install)?;
            ensure!(
                !opts.start,
                "adoption stages inactive; use --no-start and explicit activation"
            );
            let account = system_preflight::account(&opts.execution_user)?;
            let source = account.home.join(".local/state/voyage/vessel");
            opts.adoption_source = Some(source.clone());
            let plan = Plan::prepare(&opts)?;
            let source_metadata = fs::symlink_metadata(&source)?;
            ensure!(
                source_metadata.is_dir()
                    && source_metadata.uid() == account.uid
                    && source_metadata.mode() & 0o077 == 0,
                "original user source directory is not private and owned"
            );
            ensure!(
                source_metadata.dev() == fs::metadata(root()?)?.dev(),
                "user adoption requires atomic same-filesystem retention between original source and /etc/voyage-adoption; different mounts require an explicit relocation backend, and source/services are unchanged"
            );
            let source_manifest_digest = hash(&plan.manifest)?;
            service::files::directory(&dir, 0, true)?;
            let mut review = Review {
                schema_version: 1,
                operation_id: id,
                phase: "prepared".into(),
                source_directory: source,
                source_device: source_metadata.dev(),
                source_inode: source_metadata.ino(),
                source_manifest_digest,
                original_user: opts.execution_user,
                uid: account.uid,
                gid: account.gid,
                home: account.home,
                groups: account.groups,
                source_boot: boot()?,
                bin: opts.bin,
                release: plan.record.release.clone(),
                planned_installation: plan.record,
                gateway_user: opts.gateway_user,
                origin: opts.origin,
                key: opts.credential_key,
                provisioner: opts.credential_unit,
                source_credential_key,
                receipt_digest: String::new(),
                snapshot_digest: None,
                provider_fingerprint: None,
                target_incarnations: BTreeMap::new(),
                freeze_commands: BTreeMap::new(),
                commit_commands: BTreeMap::new(),
                freezes: BTreeMap::new(),
                source_units: Vec::new(),
                source_definitions: BTreeMap::new(),
            };
            review.receipt_digest = hash(&(
                &review.operation_id,
                &review.original_user,
                review.uid,
                review.gid,
                &review.home,
                &review.groups,
                review.source_boot,
                &review.release,
                &review.planned_installation,
                review.source_device,
                review.source_inode,
                &review.source_manifest_digest,
                &review.gateway_user,
                &review.origin,
                &review.key,
                &review.provisioner,
                &review.source_credential_key,
            ))?;
            save(&review)?;
            review.phase = "quiescence-requested".into();
            save(&review)?;
            match user_request(&review, &UserRequest::Quiesce)? {
                Frame::Quiesced {
                    services,
                    definitions,
                    ..
                } => {
                    review.source_units = services;
                    review.source_definitions = definitions;
                }
                _ => bail!("legacy quiescence request unconfirmed"),
            };
            review.phase = "awaiting-boot-retirement".into();
            save(&review)?;
            output(&review);
        }
        "review" => {
            ensure!(
                args.len() == 2,
                "adoption review takes only its operation identity"
            );
            let mut review = load(id)?;
            ensure!(
                review.phase == "awaiting-boot-retirement" && boot()? != review.source_boot,
                "review requires observed host boot retirement of original owners"
            );
            recheck(&review)?;
            let _snapshot = observe_snapshot(&mut review, "reviewed-source", None)?;
            review.receipt_digest = hash(&(
                &review.operation_id,
                &review.original_user,
                review.uid,
                review.gid,
                &review.home,
                &review.groups,
                review.source_boot,
                &review.release,
                &review.planned_installation,
                review.source_device,
                review.source_inode,
                &review.source_manifest_digest,
                &review.gateway_user,
                &review.origin,
                &review.key,
                &review.provisioner,
                &review.source_credential_key,
                &review.source_definitions,
                &review.snapshot_digest,
                &review.target_incarnations,
            ))?;
            review.phase = "ready".into();
            save(&review)?;
            output(&review);
        }
        "status" => {
            ensure!(args.len() == 2, "adoption status takes only identity");
            let mut review = load(id)?;
            if review.phase == "activation-requested" {
                if let Ok(mut plan) = lifecycle::installed_for_adoption() {
                    let release = Path::new(RELEASE_ROOT)
                        .join("releases")
                        .join(&review.release);
                    if plan.record.release == review.release
                        && verify_effective(&plan).is_ok()
                        && verify_provisioner(&plan).is_ok()
                        && pid(ROOT_UNIT, 0, &release.join("bin/vessel")).is_ok()
                        && pid(
                            GATEWAY_UNIT,
                            plan.record.gateway_uid,
                            &release.join("bin/vessel"),
                        )
                        .is_ok()
                        && preflight().is_ok()
                    {
                        plan.record.phase = "active".into();
                        plan.record.start_requested = true;
                        lifecycle::save_for_adoption(&plan.record)?;
                        review.phase = "active".into();
                        save(&review)?;
                    }
                }
            }
            if review.phase == "rollback-services-requested"
                && !directory(id)?.join("retained-user-vessel").try_exists()?
                && !review
                    .source_directory
                    .join("system-adoption-fence.json")
                    .try_exists()?
                && !Path::new(UNIT_ROOT).join(ROOT_UNIT).try_exists()?
                && !Path::new(UNIT_ROOT).join(GATEWAY_UNIT).try_exists()?
                && matches!(
                    user_request(
                        &review,
                        &UserRequest::ObserveRestoredServices {
                            definitions: review.source_definitions.clone()
                        }
                    )?,
                    Frame::RestoredServices { ready: true }
                )
            {
                review.phase = "rolled-back".into();
                save(&review)?;
            }
            // Lookup only: never repeat a freeze, commit, install, activation or
            // user-service restoration after a lost reply.
            let mut observed = BTreeMap::new();
            for (session, command_id) in &review.freeze_commands {
                let path = if review
                    .source_directory
                    .join("system-adoption-fence.json")
                    .try_exists()?
                {
                    directory(id)?
                        .join("retained-user-vessel/sessions")
                        .join(session.to_string())
                } else {
                    review
                        .source_directory
                        .join("sessions")
                        .join(session.to_string())
                };
                // The retained tree is root-private after fencing, so source
                // lookup is intentionally unavailable to the dropped helper.
                let lookup = if path.starts_with(REVIEWS) {
                    None
                } else {
                    transition(
                        &review,
                        &TransitionRequest {
                            schema: SCHEMA,
                            session_id: *session,
                            source_incarnation: review
                                .freezes
                                .get(session)
                                .map(|p| p.source_incarnation)
                                .unwrap_or(Uuid::nil()),
                            operation: TransitionOperation::Lookup {
                                directory: path,
                                command_id: *command_id,
                            },
                        },
                    )
                    .ok()
                };
                observed.insert(session.to_string(),serde_json::json!({"freeze_command":command_id,
                    "freeze_observed":matches!(lookup,Some(TransitionResponse::Prepared{..})),
                    "commit_command":review.commit_commands.get(session),"no_effect_replayed":true}));
            }
            output(&review);
            println!(
                "{}",
                serde_json::json!({"journal_observations":observed,"partial_adoption_requires_operator_recovery":!matches!(review.phase.as_str(),"ready"|"awaiting-boot-retirement"|"dormant-installed"|"active"|"rolled-back")})
            );
        }
        "apply" => {
            ensure!(
                args.len() == 3,
                "adoption requires exact approved review digest"
            );
            let mut review = load(id)?;
            ensure!(
                review.receipt_digest == args[2],
                "adoption approval digest conflict"
            );
            if matches!(
                review.phase.as_str(),
                "dormant-installed" | "active" | "rolled-back"
            ) {
                output(&review);
                return Ok(());
            }
            apply(&mut review)?;
            output(&review);
        }
        "activate" => {
            ensure!(args.len() == 3, "activation requires exact adoption review");
            let mut review = load(id)?;
            ensure!(
                review.receipt_digest == args[2] && review.phase == "dormant-installed",
                "adoption activation is not ready"
            );
            recheck(&review)?;
            review.phase = "activation-requested".into();
            save(&review)?;
            let mut plan = lifecycle::installed_for_adoption()?;
            ensure!(
                plan.record.phase == "inactive" && plan.record.release == review.release,
                "adoption installation changed"
            );
            activate(&plan)?;
            plan.record.phase = "active".into();
            plan.record.start_requested = true;
            lifecycle::save_for_adoption(&plan.record)?;
            review.phase = "active".into();
            save(&review)?;
            output(&review);
        }
        "rollback" => {
            ensure!(args.len() == 3, "rollback requires exact adoption review");
            let mut review = load(id)?;
            ensure!(
                review.receipt_digest == args[2]
                    && matches!(
                        review.phase.as_str(),
                        "ready"
                            | "capturing-retired-source"
                            | "freezing-source"
                            | "installing-dormant-system"
                            | "dormant-installed"
                            | "rollback-requested"
                    ),
                "legacy rollback is restricted to exact reviewed preactivation adoption; no active or uncertain activation fallback"
            );
            recheck(&review)?;
            let current_provider = match user_request(&review, &UserRequest::ProviderFingerprint)? {
                Frame::ProviderFingerprint { sha256 } => sha256,
                _ => bail!("original credential namespace cannot be verified"),
            };
            ensure!(
                Some(current_provider) == review.provider_fingerprint,
                "original provider namespace changed; retained old source must not guess credential/state compatibility"
            );
            // Every accepted phase precedes any service activation. An observed
            // live service or guardian invalidates this preactivation recovery.
            let target_guardians = Path::new(CONTROL).join("guardians");
            for (session, incarnation) in &review.target_incarnations {
                ensure!(
                    !target_guardians
                        .join(session.to_string())
                        .join(incarnation.to_string())
                        .join("admission.json")
                        .try_exists()?,
                    "target voyage was admitted; legacy rollback requires format compatibility rather than preactivation recovery"
                );
            }
            for (name, content) in [
                (ROOT_UNIT, &review.planned_installation.root_unit),
                (GATEWAY_UNIT, &review.planned_installation.gateway_unit),
            ] {
                let unit = Path::new(UNIT_ROOT).join(name);
                if unit.try_exists()? {
                    service::files::check_path(&unit, 0)?;
                    ensure!(
                        files::read(&unit, 128 * 1024)? == content.as_bytes(),
                        "partial adoption unit differs from pinned review"
                    );
                    ensure!(
                        query(name, "MainPID")? == "0" && query(name, "DropInPaths")?.is_empty(),
                        "partial adoption service retirement/effective unit uncertain"
                    );
                }
            }
            review.phase = "rollback-requested".into();
            save(&review)?;
            for (name, content) in [
                (GATEWAY_UNIT, &review.planned_installation.gateway_unit),
                (ROOT_UNIT, &review.planned_installation.root_unit),
            ] {
                let path = Path::new(UNIT_ROOT).join(name);
                if path.try_exists()? {
                    ctl(&["disable", name])?;
                    service::files::remove_reviewed(&path, content)?;
                }
            }
            ctl(&["daemon-reload"])?;
            if Path::new(CONFIG_ROOT)
                .join("system-install.json")
                .try_exists()?
            {
                let mut retained = review.planned_installation.clone();
                retained.phase = "uninstalled-retained".into();
                lifecycle::save_for_adoption(&retained)?;
            }
            let retained = directory(review.operation_id)?.join("retained-user-vessel");
            if retained.try_exists()? {
                if review.source_directory.try_exists()? {
                    service::files::check_path(&review.source_directory, 0)?;
                    let aliases = review.source_directory.join("sessions");
                    if aliases.try_exists()? {
                        for entry in fs::read_dir(&aliases)? {
                            let path = entry?.path();
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .context("invalid compatibility alias")?;
                            let session = Uuid::parse_str(name)?;
                            ensure!(
                                review.target_incarnations.contains_key(&session)
                                    && fs::symlink_metadata(&path)?.file_type().is_symlink()
                                    && fs::read_link(&path)?
                                        == Path::new(RUNTIME).join(session.to_string()),
                                "unexpected legacy alias during rollback"
                            );
                            fs::remove_file(path)?;
                        }
                        fs::remove_dir(aliases)?;
                    }
                    let fence = review.source_directory.join("system-adoption-fence.json");
                    if fence.try_exists()? {
                        let value: serde_json::Value =
                            serde_json::from_slice(&files::read(&fence, 16384)?)?;
                        ensure!(
                            value["operation_id"] == serde_json::to_value(review.operation_id)?,
                            "legacy fence identity changed"
                        );
                        fs::remove_file(fence)?;
                    }
                    fs::remove_dir(&review.source_directory)?;
                }
                fs::rename(retained, &review.source_directory)?;
            }
            review.phase = "rollback-services-requested".into();
            save(&review)?;
            // The original user control/history installation is retained, frozen
            // and never overwritten by a root backup. Restore its reviewed units
            // under that original account, keeping interrupted work interrupted.
            ensure!(
                matches!(
                    user_request(
                        &review,
                        &UserRequest::RestoreServices {
                            definitions: review.source_definitions.clone()
                        }
                    )?,
                    Frame::Complete
                ),
                "original user source restore unconfirmed"
            );
            let deadline = Instant::now() + Duration::from_secs(45);
            loop {
                if matches!(
                    user_request(
                        &review,
                        &UserRequest::ObserveRestoredServices {
                            definitions: review.source_definitions.clone()
                        }
                    )?,
                    Frame::RestoredServices { ready: true }
                ) {
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "original user readiness remains unconfirmed; status observes without starting again"
                );
                std::thread::sleep(Duration::from_millis(200));
            }
            review.phase = "rolled-back".into();
            save(&review)?;
            output(&review);
        }
        _ => bail!("unknown adoption action"),
    }
    Ok(())
}

pub(super) fn system_quiescent(source: &Path, drain: bool) -> Result<()> {
    let (parent, child) = UnixStream::pair()?;
    parent.set_read_timeout(Some(Duration::from_secs(300)))?;
    let fd = child.as_raw_fd();
    let mut command = Command::new(source.join("vessel"));
    service::files::executable(&source.join("vessel"), 0)?;
    command
        .args(["migration-control", "--directory", CONTROL, "--pipe-fd"])
        .arg(fd.to_string())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut running = OwnedChild(command.spawn()?);
    drop(child);
    let mut parent = parent;
    write(&mut parent, &ControlRequest::Quiescence { drain })?;
    ensure!(
        matches!(
            read::<Frame>(&mut parent)?,
            Frame::Quiescent {
                incarnations_retired: true
            }
        ),
        "system independent owner retirement is not observed"
    );
    drop(parent);
    finish(&mut running)
}
