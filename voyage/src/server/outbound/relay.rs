//! Enrollment transport owner only: no journal owner, model, tools, or agent loop.
use super::wake::wake;
use super::{
    authority::features,
    proxy::{self, ContextData, Hello, Identity, Message},
};
use crate::attachment::{client::EnrollmentClient, transport};
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::net::{UnixListener, UnixStream};
use tokio_util::sync::CancellationToken;
use voyage_protocol::{
    process::{self, RuntimeInitialization, read_frame, write_frame},
    stream::Frame,
};

pub async fn run(directory: PathBuf, binary: PathBuf) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let directory = crate::attachment::journal::prepare_directory(directory)?;
    let lock =
        crate::attachment::journal::open_private_file(&directory.join("outbound-relay.lock"))?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    let registration = super::super::transport::registration(&directory)?;
    let Some(RuntimeInitialization::Outbound {
        enrollment_directory,
        origin,
        allow_insecure_loopback,
        ..
    }) = &registration.initialize
    else {
        anyhow::bail!("not an outbound session")
    };
    let endpoint = directory.join("outbound-relay.sock");
    if endpoint.exists() {
        std::fs::remove_file(&endpoint)?;
    }
    let listener = UnixListener::bind(&endpoint)?;
    std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let shutdown = CancellationToken::new();
    let interrupted = shutdown.clone();
    let signal_task = tokio::spawn(async move {
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        interrupted.cancel();
    });
    loop {
        let client = EnrollmentClient::open_existing(
            enrollment_directory,
            origin,
            *allow_insecure_loopback,
        )?;
        let inspection = client.inspection();
        let identity = Identity {
            origin: client.origin().into(),
            machine_id: inspection.machine_id,
            owner_id: inspection.owner_id.context("enrollment inactive")?,
            epoch: inspection.epoch,
        };
        let candidate = directory.join(format!(".outbound-identity-{}", uuid::Uuid::new_v4()));
        let mut identity_file = crate::attachment::journal::open_private_file(&candidate)?;
        identity_file.set_len(0)?;
        serde_json::to_writer(&mut identity_file, &identity)?;
        identity_file.sync_all()?;
        std::fs::rename(candidate, directory.join("outbound-identity.json"))?;
        std::fs::File::open(&directory)?.sync_all()?;
        let candidate = directory.join(format!(".outbound-relay-pid-{}", uuid::Uuid::new_v4()));
        let mut pid_file = crate::attachment::journal::open_private_file(&candidate)?;
        use std::io::Write;
        write!(&mut pid_file, "{}", std::process::id())?;
        pid_file.sync_all()?;
        std::fs::rename(candidate, directory.join("outbound-relay.pid"))?;
        std::fs::File::open(&directory)?.sync_all()?;
        let mut connection = tokio::select! {
            _ = shutdown.cancelled() => break,
            result = transport::connect(client, features(), CancellationToken::new()) => result?,
        };
        let mut pending = None;
        let mut pending_since = tokio::time::Instant::now();
        let mut wake_retry = tokio::time::interval(Duration::from_millis(250));
        loop {
            let socket = tokio::select! {
                _ = shutdown.cancelled() => break,
                accepted = listener.accept() => Some(accepted?.0),
                frame = connection.receive(), if pending.is_none() => {
                    let Some(frame) = frame else { break };
                    if let Some(reply) = observe(&binary, &directory, &identity, &connection, &frame).await? {
                        if connection.is_active() { connection.send(reply)?; }
                        continue;
                    }
                    pending = Some(frame);
                    pending_since = tokio::time::Instant::now();
                    // Wake is local peer-authenticated and does not admit the frame.
                    // A refusal never retries or replays a remote external effect.
                    if wake(&directory, registration.session_id).await.is_err() { break; }
                    None
                },
                _ = wake_retry.tick() => {
                    if stop_requested(&directory)? { shutdown.cancel(); break; }
                    if pending.is_none() { continue; }
                    if !connection.is_active() || pending_since.elapsed() >= Duration::from_secs(25) { break; }
                    // The old worker may have acknowledged cleanup just before
                    // publishing its suspension marker. Wake itself admits no work.
                    if wake(&directory, registration.session_id).await.is_err() { break; }
                    None
                },
            };
            let Some(socket) = socket else { continue };
            if socket.peer_cred()?.uid() != unsafe { libc::geteuid() } {
                continue;
            }
            let result = bridge(
                socket,
                &directory,
                &identity,
                &mut connection,
                &mut pending,
                &shutdown,
            )
            .await;
            if result.is_err() || !connection.is_active() || shutdown.is_cancelled() {
                break;
            }
        }
        connection.close().await;
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
        }
    }
    signal_task.abort();
    let _ = signal_task.await;
    std::fs::remove_file(endpoint)?;
    Ok(())
}

async fn bridge(
    mut socket: UnixStream,
    directory: &Path,
    identity: &Identity,
    connection: &mut transport::Connection,
    pending: &mut Option<Frame>,
    shutdown: &CancellationToken,
) -> Result<()> {
    let hello: Hello =
        tokio::time::timeout(Duration::from_secs(5), read_frame(&mut socket)).await??;
    let current = super::super::transport::registration(directory)?;
    ensure!(
        hello.session_id == current.session_id
            && hello.incarnation == current.incarnation
            && hello.token == current.token,
        "outbound runtime authentication failed"
    );
    let context = connection.context();
    write_frame(
        &mut socket,
        &ContextData {
            identity: identity.clone(),
            connection_id: context.connection_id,
            features: context.features.clone(),
        },
    )
    .await?;
    let lease = connection.lease();
    send_lease(&mut socket, &lease).await?;
    if let Some(frame) = pending.take() {
        write_frame(&mut socket, &Message::Frame(Box::new(frame))).await?;
    }
    let (mut reader, mut writer) = socket.into_split();
    let (messages, mut incoming) = tokio::sync::mpsc::channel(8);
    let reading = tokio::spawn(async move {
        while let Ok(message) = read_frame::<Message>(&mut reader).await {
            if messages.send(message).await.is_err() {
                break;
            }
        }
    });
    let mut heartbeat = tokio::time::interval(Duration::from_millis(50));
    let mut cleanup_acknowledged = None;
    let result: Result<()> = async {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => anyhow::bail!("outbound relay stopping"),
                message = incoming.recv() => match message {
                    Some(Message::Frame(frame)) => connection.send(*frame)?,
                    Some(Message::Closed { cleanup_observed }) => {
                        cleanup_acknowledged=Some(cleanup_observed);
                        ensure!(cleanup_observed,"outbound worker cleanup unconfirmed");
                        return Ok(());
                    },
                    _ => anyhow::bail!("outbound runtime disconnected without cleanup acknowledgment"),
                },
                frame = connection.receive() => {
                    let Some(frame) = frame else {
                        anyhow::bail!("outbound connection authority ended");
                    };
                    tokio::time::timeout(Duration::from_secs(2), write_frame(&mut writer, &Message::Frame(Box::new(frame)))).await??;
                },
                _ = heartbeat.tick() => {
                    if stop_requested(directory)? { shutdown.cancel(); anyhow::bail!("outbound session stopped"); }
                    send_lease(&mut writer, &lease).await?;
                },
            }
        }
    }.await;
    if result.is_err() && cleanup_acknowledged.is_none() {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            write_frame(&mut writer, &Message::Withdraw),
        )
        .await;
        // Retain the enrollment lock while the worker observes lease withdrawal
        // and completes bounded cleanup. EOF means the worker is no longer alive;
        // its durable unresolved obligations still prevent automatic replay.
        let _ = tokio::time::timeout(Duration::from_secs(75), async {
            while let Some(message) = incoming.recv().await {
                if let Message::Closed { cleanup_observed } = message {
                    cleanup_acknowledged = Some(cleanup_observed);
                    break;
                }
            }
        })
        .await;
    }
    if cleanup_acknowledged != Some(true) {
        // Unknown cleanup is not a reconnection opportunity. A later explicit
        // local recovery must resolve the worker's durable resource obligations.
        shutdown.cancel();
    }
    reading.abort();
    let _ = reading.await;
    result
}

fn stop_requested(directory: &Path) -> Result<bool> {
    let current = super::super::transport::registration(directory)?;
    if matches!(
        current.state,
        process::ProcessState::Stopped
            | process::ProcessState::CleanupUnconfirmed
            | process::ProcessState::Relinquished
    ) {
        return Ok(true);
    }
    let stopped = directory.join("stopped.json");
    if !stopped.exists() {
        return Ok(false);
    }
    let file = crate::attachment::journal::open_private_file(&stopped)?;
    let evidence: serde_json::Value = serde_json::from_reader(file)?;
    Ok(evidence["incarnation"] == current.incarnation.to_string()
        && (evidence["suspended"] != true
            || !evidence["archive"].is_null()
            || !evidence["deletion"].is_null()))
}

async fn send_lease(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    lease: &transport::ConnectionLease,
) -> Result<()> {
    // Sample clock before remaining authority: rounding only shortens the lease.
    let sampled = proxy::now();
    let until = sampled.saturating_add(lease.remaining().as_millis().try_into().unwrap_or(0));
    tokio::time::timeout(
        Duration::from_secs(2),
        write_frame(writer, &Message::Lease { until }),
    )
    .await??;
    Ok(())
}

async fn observe(
    binary: &Path,
    directory: &Path,
    identity: &Identity,
    connection: &transport::Connection,
    frame: &Frame,
) -> Result<Option<Frame>> {
    use voyage_protocol::{
        attachment::Operation,
        stream::{DenialCode, Reply},
    };
    let request_id = match frame {
        Frame::Command { command }
            if matches!(
                command.operation,
                Operation::List { .. } | Operation::Inspect { .. }
            ) =>
        {
            command.command_id
        }
        Frame::ReplayRequest { request_id, .. } => *request_id,
        _ => return Ok(None),
    };
    let registration = super::super::transport::registration(directory)?;
    if super::super::suspended::check_suspended(directory, &registration).is_err() {
        return Ok(None);
    }
    let lease = connection.lease();
    let sampled = proxy::now();
    let request = super::observe::Request {
        context: ContextData {
            identity: identity.clone(),
            connection_id: connection.context().connection_id,
            features: connection.context().features.clone(),
        },
        incarnation: registration.incarnation,
        token: registration.token,
        until: sampled.saturating_add(lease.remaining().as_millis().try_into().unwrap_or(0)),
        frame: frame.clone(),
    };
    let mut child = tokio::process::Command::new(binary)
        .arg("outbound-observe")
        .arg(directory)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut input = child
        .stdin
        .take()
        .context("observation input unavailable")?;
    let mut output = child
        .stdout
        .take()
        .context("observation output unavailable")?;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        write_frame(&mut input, &request).await?;
        drop(input);
        let frame: Option<Frame> = read_frame(&mut output).await?;
        ensure!(child.wait().await?.success(), "observation helper failed");
        Ok::<_, anyhow::Error>(frame)
    })
    .await;
    if !matches!(result, Ok(Ok(_))) {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    Ok(Some(match result {
        Ok(Ok(Some(frame))) => frame,
        _ => Frame::Result {
            connection_id: connection.context().connection_id,
            command_id: request_id,
            reply: Reply::Denied {
                code: DenialCode::Unauthorized,
            },
        },
    }))
}
