//! Authenticated bounded native Unix runtime route; never starts on observation.
use anyhow::{Result, ensure};
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::Path,
    time::Duration,
};
use tokio::net::UnixStream;
use voyage_protocol::process::{
    PROCESS_PROTOCOL, ProcessRegistration, RuntimeCommand, RuntimeRequest, RuntimeResponse,
    read_frame, write_frame,
};

pub async fn request(
    directory: &Path,
    registration: &ProcessRegistration,
    command: RuntimeCommand,
) -> Result<RuntimeResponse> {
    super::unix_registry::directory(directory)?;
    ensure!(
        registration.peer_uids.is_none(),
        "Native ordinary route refuses privileged binding"
    );
    let endpoint = directory.join("runtime.sock");
    let metadata = std::fs::symlink_metadata(&endpoint)?;
    ensure!(
        metadata.file_type().is_socket()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "Unsafe native runtime endpoint"
    );
    let effect = async {
        let mut socket = UnixStream::connect(&endpoint).await?;
        ensure!(
            socket.peer_cred()?.uid() == unsafe { libc::geteuid() },
            "Native runtime peer owner changed"
        );
        write_frame(
            &mut socket,
            &RuntimeRequest {
                protocol: PROCESS_PROTOCOL,
                session_id: registration.session_id,
                incarnation: registration.incarnation,
                token: registration.token.clone(),
                authorization: None,
                scope_authority: None,
                command,
            },
        )
        .await?;
        let response: RuntimeResponse = read_frame(&mut socket).await?;
        ensure!(
            response.protocol == PROCESS_PROTOCOL
                && response.session_id == registration.session_id
                && response.incarnation == registration.incarnation,
            "Native runtime response identity mismatch"
        );
        Ok(response)
    };
    match tokio::time::timeout(Duration::from_secs(30), effect).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!(
            "Native runtime request timed out; mutation outcome unknown, inspect exact receipt before retry"
        ),
    }
}
