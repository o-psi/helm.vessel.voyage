//! Dedicated current-authority channel. Only a typed permission check can be
//! requested; the root side sends a boolean and no control/provider secrets.
use anyhow::{Result, ensure};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};
use voyage_protocol::identity_helper::*;

pub(super) fn attach(command: &mut tokio::process::Command) -> Result<(UnixStream, OwnedFd)> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "root authority channel required"
    );
    let (parent, child) = std::os::unix::net::UnixStream::pair()?;
    let raw = unsafe { libc::fcntl(child.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 16) };
    ensure!(raw >= 0, "authority descriptor unavailable");
    let descriptor = unsafe { OwnedFd::from_raw_fd(raw) };
    command.env("VOYAGE_IDENTITY_AUTHORITY_FD", "3");
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(raw, 3) != 3 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    parent.set_nonblocking(true)?;
    Ok((UnixStream::from_std(parent)?, descriptor))
}

pub(super) async fn read(stream: &mut UnixStream) -> Result<IdentityAuthorityRequest> {
    let size = stream.read_u32().await? as usize;
    ensure!(
        (1..=2048).contains(&size),
        "authority request exceeded bound"
    );
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await?;
    let request: IdentityAuthorityRequest = serde_json::from_slice(&bytes)?;
    ensure!(
        request.schema == 1 && !request.connection_id.is_nil(),
        "authority request refused"
    );
    Ok(request)
}

pub(super) async fn reply(stream: &mut UnixStream, allowed: bool) -> Result<()> {
    let bytes = serde_json::to_vec(&IdentityAuthorityReply { schema: 1, allowed })?;
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}
