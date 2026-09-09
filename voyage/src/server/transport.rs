use super::*;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use tokio::net::{UnixListener, UnixStream};
use voyage_protocol::process::{
    PROCESS_PROTOCOL, RuntimeRequest, RuntimeResponse, read_frame, write_frame,
};

pub(super) fn registration(directory: &std::path::Path) -> Result<ProcessRegistration> {
    use std::io::Read;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("registration.json"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1
            && metadata.len() <= 16384,
        "unsafe runtime registration"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let registration: ProcessRegistration = serde_json::from_slice(&bytes)?;
    ensure!(
        registration.protocol == PROCESS_PROTOCOL && registration.token.len() >= 32,
        "invalid runtime registration"
    );
    Ok(registration)
}
pub(super) async fn listen(directory: PathBuf, state: Arc<State>) -> Result<()> {
    let endpoint = directory.join("runtime.sock");
    // Only the exclusive execution owner may retire a previous endpoint.
    match std::fs::symlink_metadata(&endpoint) {
        Ok(metadata) => {
            use std::os::unix::fs::FileTypeExt;
            ensure!(
                metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() },
                "unsafe runtime endpoint"
            );
            std::fs::remove_file(&endpoint)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = UnixListener::bind(&endpoint)?;
    std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))?;
    let capacity = Arc::new(tokio::sync::Semaphore::new(16));
    let mut clients = tokio::task::JoinSet::new();
    let mut suspensions = tokio::task::JoinSet::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut idle = tokio::time::interval_at(
        tokio::time::Instant::now() + std::time::Duration::from_secs(2),
        std::time::Duration::from_secs(2),
    );
    loop {
        tokio::select! {
            _=idle.tick() => {
                if suspensions.is_empty() {
                    let state = state.clone();
                    suspensions.spawn(async move { super::suspension::suspend(&state).await });
                }
            },
            _=state.shutdown.cancelled()=>break,
            _=tokio::signal::ctrl_c()=>{state.shutdown.cancel();break},
            _=terminate.recv()=>{state.shutdown.cancel();break},
            Some(_)=clients.join_next()=>{},
            Some(_)=suspensions.join_next()=>{},
            connection=listener.accept()=>{
                let (socket,_)=connection?;
                if socket.peer_cred()?.uid()!=unsafe{libc::geteuid()} {continue}
                let Ok(permit)=capacity.clone().try_acquire_owned() else {continue};
                let state=state.clone();let directory=directory.clone();clients.spawn(async move{let _permit=permit;let _=tokio::time::timeout(std::time::Duration::from_secs(30),respond(socket,state,directory)).await;});
            }
        }
    }
    // A connected socket can already be queued when suspension wins select!.
    // Drain that finite backlog into authenticated not-dispatched responses.
    // Later connection failures remain uncertain to clients and are resolved by
    // the journal, never replayed merely because the transport disappeared.
    if state
        .suspend_requested
        .load(std::sync::atomic::Ordering::Acquire)
    {
        let listener = listener.into_std()?;
        for _ in 0..16 {
            let (socket, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            };
            socket.set_nonblocking(true)?;
            let socket = UnixStream::from_std(socket)?;
            if socket.peer_cred()?.uid() != unsafe { libc::geteuid() } {
                continue;
            }
            let state = state.clone();
            let directory = directory.clone();
            clients.spawn(async move {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    respond(socket, state, directory),
                )
                .await;
            });
        }
    } else {
        drop(listener);
    }
    while suspensions.join_next().await.is_some() {}
    // Wait for accepted commands to register their owned work before shutdown observation.
    let barrier = state.admission.lock().await;
    if let Some(active) = state.active.lock().await.as_ref() {
        active.cancel.cancel();
    }
    drop(barrier);
    let cleanup = tokio::time::timeout(std::time::Duration::from_secs(75), async {
        loop {
            if state.active.lock().await.is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while clients.join_next().await.is_some() {}
    })
    .await;
    clients.abort_all();
    let handlers = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while clients.join_next().await.is_some() {}
    })
    .await;
    let run_cleanup = state
        .cleanup
        .wait(std::time::Duration::from_secs(60), 1)
        .await;
    let retained = state.controls.shutdown_retained(&state.owner).await;
    let snapshot = state.owner.process_snapshot().await?;
    let session_resources = state.owner.session_resources().await?;
    let observed = run_cleanup
        && retained.is_ok()
        && session_resources.as_array().is_some_and(Vec::is_empty)
        && cleanup.is_ok()
        && handlers.is_ok()
        && snapshot["pending_cleanup_run"].is_null();
    if observed {
        use std::io::Write;
        let candidate = directory.join(format!(".stopped-{}", Uuid::new_v4()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&candidate)?;
        let archive = if snapshot["lifecycle"]["archived"] == true
            && snapshot["lifecycle"]["deleted"] != true
        {
            Some(voyage_protocol::process::ArchivedVoyage {
                name: snapshot["name"].as_str().map(str::to_owned),
                revision: snapshot["revision"]
                    .as_u64()
                    .context("archive revision missing")?,
                receipt: state
                    .archive_receipt
                    .lock()
                    .await
                    .clone()
                    .unwrap_or(serde_json::Value::Null),
            })
        } else {
            None
        };
        let deletion = if snapshot["lifecycle"]["deleted"] == true {
            state.archive_receipt.lock().await.clone()
        } else {
            None
        };
        serde_json::to_writer(
            &mut file,
            &serde_json::json!({"session_id":state.registration.session_id,"incarnation":state.registration.incarnation,"cleanup_observed":true,"suspended":state.suspend_requested.load(std::sync::atomic::Ordering::Acquire),"archive":archive,"deletion":deletion}),
        )?;
        file.flush()?;
        file.sync_all()?;
        std::fs::rename(candidate, directory.join("stopped.json"))?;
        std::fs::File::open(&directory)?.sync_all()?;
    }
    std::fs::remove_file(endpoint)?;
    ensure!(
        observed,
        "runtime shutdown cleanup unconfirmed; obligations retained"
    );
    Ok(())
}
async fn respond(mut socket: UnixStream, state: Arc<State>, directory: PathBuf) -> Result<()> {
    let request: RuntimeRequest = read_frame(&mut socket).await?;
    ensure!(
        request.protocol == PROCESS_PROTOCOL
            && request.session_id == state.registration.session_id
            && request.incarnation == state.registration.incarnation
            && token_matches(&request.token, &state.registration.token),
        "runtime authentication rejected"
    );
    super::authorization::authorize(&state, &request, &directory)?;
    // Long polls are pure observations and must not hold the dispatch barrier.
    // Resolution excludes all dispatch while proving and fencing non-admission.
    let exclusive = if matches!(
        request.command,
        voyage_protocol::process::RuntimeCommand::Resolve { .. }
    ) {
        Some(state.requests.write().await)
    } else {
        None
    };
    let requests = if exclusive.is_none()
        && !matches!(
            request.command,
            voyage_protocol::process::RuntimeCommand::Events { .. }
        ) {
        Some(state.requests.read().await)
    } else {
        None
    };
    // Authority may have changed while waiting behind another dispatch.
    let authorization = super::authorization::authorize(&state, &request, &directory)?;
    let suspending = state.shutdown.is_cancelled()
        && state
            .suspend_requested
            .load(std::sync::atomic::Ordering::Acquire);
    let result = if suspending {
        Ok(serde_json::json!({"status":"suspending","not_dispatched":true}))
    } else {
        commands::dispatch(&state, request.command.clone(), authorization).await
    };
    drop(requests);
    drop(exclusive);
    let rejected = result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<commands::Rejected>())
        .map(|rejected| rejected.0.clone());
    let response = RuntimeResponse {
        protocol: PROCESS_PROTOCOL,
        outcome_unknown: result.is_err() && rejected.is_none(),
        session_id: state.registration.session_id,
        incarnation: state.registration.incarnation,
        resumed_from: None,
        result: result
            .as_ref()
            .cloned()
            .unwrap_or_else(|_| rejected.unwrap_or(serde_json::Value::Null)),
        error: result.err().map(|error| {
            error
                .to_string()
                .chars()
                .filter(|ch| !ch.is_control())
                .take(512)
                .collect()
        }),
    };
    super::authorization::authorize(&state, &request, &directory)?;
    write_frame(&mut socket, &response).await?;
    Ok(())
}
fn token_matches(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .as_bytes()
            .iter()
            .zip(right.as_bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}
