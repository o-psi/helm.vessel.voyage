//! Authenticated startup input without uncancellable blocking stdin readers.
use anyhow::{Result, ensure};
use std::{
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    time::Duration,
};
use tokio::io::unix::AsyncFd;
use voyage_protocol::process::ProcessRegistration;

pub(super) async fn registration() -> Result<ProcessRegistration> {
    let raw = unsafe { libc::fcntl(0, libc::F_DUPFD_CLOEXEC, 3) };
    ensure!(raw >= 0, "bound startup pipe unavailable");
    let input = unsafe { File::from_raw_fd(raw) };
    let metadata = input.metadata()?;
    ensure!(
        metadata.file_type().is_fifo()
            && metadata.uid() == 0
            && metadata.mode() & 0o077 == 0
            && std::fs::read_link(format!("/proc/self/fd/{raw}"))?
                .as_os_str()
                .as_encoded_bytes()
                .starts_with(b"pipe:["),
        "bound startup requires a root-owned private pipe"
    );
    let bytes = read_frame(input, Duration::from_secs(10)).await?;
    let registration: ProcessRegistration = serde_json::from_slice(&bytes)?;
    ensure!(
        registration.protocol == voyage_protocol::process::PROCESS_PROTOCOL
            && registration.token.len() >= 32
            && registration.peer_uids.as_ref().is_some_and(
                |peers| peers.supervisor == 0 && peers.runtime == unsafe { libc::geteuid() }
            ),
        "invalid bound launch identity"
    );
    Ok(registration)
}

async fn read_frame(input: File, timeout: Duration) -> Result<Vec<u8>> {
    let flags = unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) };
    ensure!(
        flags >= 0
            && unsafe { libc::fcntl(input.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                == 0,
        "bound startup pipe cannot be bounded"
    );
    let input = AsyncFd::new(input)?;
    tokio::time::timeout(timeout, async {
        let mut header = [0; 4];
        read_exact(&input, &mut header).await?;
        let length = u32::from_be_bytes(header) as usize;
        ensure!(length <= 16384, "bound registration exceeds limit");
        let mut bytes = vec![0; length];
        read_exact(&input, &mut bytes).await?;
        Ok(bytes)
    })
    .await?
}

async fn read_exact(input: &AsyncFd<File>, mut bytes: &mut [u8]) -> Result<()> {
    while !bytes.is_empty() {
        let mut ready = input.readable().await?;
        match ready.try_io(|fd| fd.get_ref().read(bytes)) {
            Ok(Ok(0)) => anyhow::bail!("bound startup pipe closed before complete registration"),
            Ok(Ok(count)) => bytes = &mut bytes[count..],
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn pipe() -> (File, File) {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) }
    }
    #[tokio::test]
    async fn frame_is_exact_bounded_and_cancellable() {
        let (read, mut write) = pipe();
        write.write_all(b"\0\0\0\x03abcignored").unwrap();
        assert_eq!(
            read_frame(read, Duration::from_secs(1)).await.unwrap(),
            b"abc"
        );
        let (read, mut write) = pipe();
        write.write_all(&16385u32.to_be_bytes()).unwrap();
        assert!(
            read_frame(read, Duration::from_secs(1))
                .await
                .unwrap_err()
                .to_string()
                .contains("exceeds limit")
        );
        let (read, mut write) = pipe();
        write.write_all(b"\0\0\0\x03a").unwrap();
        drop(write);
        assert!(
            read_frame(read, Duration::from_secs(1))
                .await
                .unwrap_err()
                .to_string()
                .contains("closed")
        );
        // A writer which retains its end without completing the frame cannot
        // strand a Tokio blocking thread or prevent runtime shutdown.
        let (read, mut write) = pipe();
        write.write_all(b"\0\0\0\x03a").unwrap();
        assert!(
            read_frame(read, Duration::from_millis(20))
                .await
                .unwrap_err()
                .is::<tokio::time::error::Elapsed>()
        );
        assert!(write.write_all(b"b").is_err());
    }
}
