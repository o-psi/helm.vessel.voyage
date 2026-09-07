use anyhow::{Result, ensure};
use std::{path::PathBuf, time::Duration};
use voyage_protocol::process::*;
use voyage_protocol::vessel::{MAX_VESSEL_BODY, VESSEL_API_VERSION};

/// Framed stdio bridge, suitable for an explicitly authenticated SSH account.
pub async fn request(directory: PathBuf) -> Result<()> {
    super::registry::private_directory(&directory)?;
    let bytes = tokio::task::spawn_blocking(|| {
        use std::os::fd::FromRawFd;
        let fd = unsafe { libc::dup(libc::STDIN_FILENO) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut stdin = unsafe { std::fs::File::from_raw_fd(fd) };
        let mut prefix = [0u8; 4];
        read_bounded(&mut stdin, &mut prefix)?;
        let length = u32::from_be_bytes(prefix) as usize;
        if length == 0 || length > MAX_VESSEL_BODY {
            return Err(std::io::Error::other("invalid process frame length"));
        }
        let mut bytes = vec![0; length];
        read_bounded(&mut stdin, &mut bytes)?;
        Ok::<_, std::io::Error>(bytes)
    })
    .await??;
    let request: VesselRequest = serde_json::from_slice(&bytes)?;
    ensure!(
        request.protocol == VESSEL_API_VERSION,
        "unsupported process protocol"
    );
    let response = super::exchange::exchange(&directory, &request).await?;
    ensure!(
        response.protocol == VESSEL_API_VERSION,
        "unsupported Vessel response protocol"
    );
    let bytes = serde_json::to_vec(&response)?;
    ensure!(bytes.len() <= MAX_VESSEL_BODY, "response frame too large");
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&(bytes.len() as u32).to_be_bytes())?;
        stdout.write_all(&bytes)?;
        stdout.flush()
    })
    .await??;
    Ok(())
}

// A remote caller that sends only part of a frame cannot pin a helper indefinitely.
fn read_bounded(reader: &mut impl std::io::Read, mut bytes: &mut [u8]) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !bytes.is_empty() {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "stdio frame deadline exceeded",
            ));
        }
        let mut fd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let polled = unsafe {
            libc::poll(
                &mut fd,
                1,
                remaining.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        if polled < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if polled == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "stdio frame deadline exceeded",
            ));
        }
        let read = reader.read(bytes)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "incomplete frame",
            ));
        }
        bytes = &mut bytes[read..];
    }
    Ok(())
}
