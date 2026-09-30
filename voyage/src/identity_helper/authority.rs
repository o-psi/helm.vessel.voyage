//! Current-authority checks over a root-created private descriptor. The helper
//! cannot read root control state and never trusts its initial scope as a grant.
use anyhow::{Result, ensure};
use std::{
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::net::UnixStream,
    },
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use voyage_protocol::{accounts::EnrollmentActor, identity_helper::*};

pub(super) struct Pipe {
    stream: Mutex<UnixStream>,
    failed: AtomicBool,
}
impl std::fmt::Debug for Pipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IdentityAuthority([private])")
    }
}
impl Pipe {
    pub(super) fn open() -> Result<Self> {
        ensure!(
            std::env::var("VOYAGE_IDENTITY_AUTHORITY_FD")
                .ok()
                .as_deref()
                == Some("3"),
            "private authority pipe unavailable"
        );
        Self::from_descriptor(3)
    }

    fn from_descriptor(fd: i32) -> Result<Self> {
        let raw = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 16) };
        ensure!(raw >= 0, "private authority pipe unavailable");
        let stream = unsafe { UnixStream::from_raw_fd(raw) };
        let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        ensure!(
            unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut peer as *mut libc::ucred).cast(),
                    &mut size,
                )
            } == 0
                && size as usize == std::mem::size_of::<libc::ucred>()
                && peer.uid == 0,
            "private authority root peer refused"
        );
        stream.set_read_timeout(Some(Duration::from_millis(750)))?;
        stream.set_write_timeout(Some(Duration::from_millis(750)))?;
        Ok(Self {
            stream: Mutex::new(stream),
            failed: AtomicBool::new(false),
        })
    }

    pub(super) fn allows(
        &self,
        actor: &EnrollmentActor,
        right: IdentityAuthorityRight,
        connection: uuid::Uuid,
        account: Option<uuid::Uuid>,
    ) -> bool {
        if self.failed.load(Ordering::Acquire) {
            return false;
        }
        let result = (|| -> Result<bool> {
            let mut stream = self
                .stream
                .lock()
                .map_err(|_| anyhow::anyhow!("authority pipe unavailable"))?;
            let request = IdentityAuthorityRequest {
                schema: 1,
                actor: actor.clone(),
                right,
                connection_id: connection,
                account_id: account,
            };
            let bytes = serde_json::to_vec(&request)?;
            ensure!(bytes.len() <= 2048, "authority frame exceeded");
            stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
            stream.write_all(&bytes)?;
            let mut header = [0; 4];
            stream.read_exact(&mut header)?;
            let size = u32::from_be_bytes(header) as usize;
            ensure!((1..=128).contains(&size), "authority reply exceeded");
            let mut bytes = vec![0; size];
            stream.read_exact(&mut bytes)?;
            let reply: IdentityAuthorityReply = serde_json::from_slice(&bytes)?;
            ensure!(reply.schema == 1, "authority reply unavailable");
            Ok(reply.allowed)
        })();
        match result {
            Ok(allowed) => allowed,
            Err(_) => {
                self.failed.store(true, Ordering::Release);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_private_socket_cannot_pose_as_root_authority_or_receive_input() {
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "negative peer case requires ordinary UID"
        );
        let (mut peer, child) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let error = Pipe::from_descriptor(child.as_raw_fd()).err().unwrap();
        assert!(error.to_string().contains("root peer refused"));
        let mut byte = [0; 1];
        let read = peer.read(&mut byte).unwrap_err();
        assert_eq!(read.kind(), std::io::ErrorKind::WouldBlock);
    }
    #[test]
    fn missing_descriptor_cannot_create_positive_authority() {
        assert!(Pipe::from_descriptor(-1).is_err());
        let extra = serde_json::json!({"schema":1,"allowed":true,"control_key":"private"});
        assert!(serde_json::from_value::<IdentityAuthorityReply>(extra).is_err());
    }
}
