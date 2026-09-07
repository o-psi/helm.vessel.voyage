use anyhow::{Result, ensure};
use std::{path::Path, time::Duration};
use tokio::net::UnixStream;
use voyage_protocol::process::*;

#[derive(Debug)]
pub(super) struct OutcomeUnknown;
impl std::fmt::Display for OutcomeUnknown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "command outcome unknown; inspect durable receipt")
    }
}
impl std::error::Error for OutcomeUnknown {}

pub async fn forward(
    directory: &Path,
    registration: &ProcessRegistration,
    command: RuntimeCommand,
) -> Result<RuntimeResponse> {
    forward_authorized(directory, registration, command, None).await
}
pub(super) async fn forward_authorized(
    directory: &Path,
    registration: &ProcessRegistration,
    command: RuntimeCommand,
    authorization: Option<GrantBinding>,
) -> Result<RuntimeResponse> {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let mut stream = UnixStream::connect(directory.join("runtime.sock")).await?;
        ensure!(
            stream.peer_cred()?.uid() == unsafe { libc::geteuid() },
            "runtime peer uid mismatch"
        );
        write_frame(
            &mut stream,
            &RuntimeRequest {
                protocol: PROCESS_PROTOCOL,
                session_id: registration.session_id,
                incarnation: registration.incarnation,
                token: registration.token.clone(),
                authorization,
                command,
            },
        )
        .await?;
        let response: RuntimeResponse = read_frame(&mut stream).await?;
        ensure!(
            response.protocol == PROCESS_PROTOCOL
                && response.session_id == registration.session_id
                && response.incarnation == registration.incarnation,
            "runtime identity mismatch"
        );
        Ok(response)
    })
    .await
    .map_err(|_| anyhow::anyhow!("runtime response deadline exceeded; command outcome unknown"))
    .and_then(|response| response);
    result.map_err(|error| error.context(OutcomeUnknown))
}

pub async fn inspect(directory: &Path, registration: &ProcessRegistration) -> ProcessInfo {
    let mut info = ProcessInfo::from(registration);
    info.state = match forward(directory, registration, RuntimeCommand::Health).await {
        Ok(response) if response.error.is_none() => ProcessState::Live,
        _ if registration.state == ProcessState::Relinquished => ProcessState::Relinquished,
        _ if !directory.join("runtime.sock").exists()
            && super::recovery::clean_stop(directory, registration) =>
        {
            ProcessState::Stopped
        }
        _ => match registration.state {
            ProcessState::Stopped => ProcessState::Stopped,
            _ => ProcessState::Unavailable,
        },
    };
    if info.state == ProcessState::Stopped {
        info.archive = super::recovery::archived(directory, registration);
    }
    info
}
