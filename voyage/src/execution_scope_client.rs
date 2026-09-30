//! No credential leaves this process before the kernel authenticates UID 0.
use anyhow::{Result, ensure};
use voyage_protocol::{
    execution_scope::*,
    process::{GrantBinding, ProcessGrant, ProcessRegistration},
};
pub(crate) fn current(
    registration: &ProcessRegistration,
    handle: &ExecutionScopeHandle,
    binding: &GrantBinding,
) -> Result<ProcessGrant> {
    query(registration, handle, binding).map_err(|_| anyhow::anyhow!("execution scope unavailable"))
}
fn query(
    registration: &ProcessRegistration,
    handle: &ExecutionScopeHandle,
    binding: &GrantBinding,
) -> Result<ProcessGrant> {
    #[cfg(target_os = "linux")]
    let scope = linux::query(registration, handle)?;
    #[cfg(not(target_os = "linux"))]
    let scope: RuntimeScope = {
        let _ = (registration, handle, binding);
        anyhow::bail!("scope unavailable")
    };
    let now: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        .try_into()?;
    ensure!(
        !binding.grant_id.is_nil()
            && !binding.principal_id.is_nil()
            && binding.revision > 0
            && scope.schema == SCHEMA
            && scope.session_id == registration.session_id
            && scope.incarnation == registration.incarnation
            && scope.binding == *binding
            && scope.expires_at_ms > now
            && scope.workspace == registration.workspace
            && (!scope.full_access || scope.connection_binding.is_some())
            && scope.rights.len() <= 64
            && scope.accounts.len() <= 256
            && scope.enrollment_connections.len() <= 256,
        "invalid scope metadata"
    );
    Ok(ProcessGrant {
        full_access: scope.full_access,
        grant_id: scope.binding.grant_id,
        principal_id: scope.binding.principal_id,
        session_id: scope.session_id,
        workspace: scope.workspace,
        revision: scope.binding.revision,
        rights: scope.rights,
        accounts: scope.accounts,
        enrollment_connections: scope.enrollment_connections,
        expires_at_ms: scope.expires_at_ms,
        revoked: false,
        token_hash: String::new(),
        parent_grant: None,
        connection_binding: scope.connection_binding,
        participant_binding: None,
    })
}
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{
        io::{Read, Write},
        os::fd::{AsRawFd, FromRawFd},
        os::unix::net::UnixStream,
        time::{Duration, Instant},
    };
    fn wait(stream: &UnixStream, event: i16, deadline: Instant) -> Result<()> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "scope deadline elapsed");
        let mut descriptor = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: event,
            revents: 0,
        };
        let result = unsafe {
            libc::poll(
                &mut descriptor,
                1,
                remaining.as_millis().min(i32::MAX as u128).max(1) as i32,
            )
        };
        ensure!(
            result > 0 && descriptor.revents & event != 0,
            "scope transport unavailable"
        );
        Ok(())
    }
    fn write(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> Result<()> {
        while !bytes.is_empty() {
            ensure!(Instant::now() < deadline, "scope timeout");
            match stream.write(bytes) {
                Ok(0) => anyhow::bail!("scope closed"),
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    wait(stream, libc::POLLOUT, deadline)?
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    fn read(stream: &mut UnixStream, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
        while !bytes.is_empty() {
            ensure!(Instant::now() < deadline, "scope timeout");
            match stream.read(bytes) {
                Ok(0) => anyhow::bail!("scope closed"),
                Ok(n) => bytes = &mut bytes[n..],
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    wait(stream, libc::POLLIN, deadline)?
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    pub(super) fn query(
        registration: &ProcessRegistration,
        handle: &ExecutionScopeHandle,
    ) -> Result<RuntimeScope> {
        ensure!(
            (1..=80).contains(&handle.socket_name.len())
                && handle
                    .socket_name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !handle.lease_id.is_nil()
                && (32..=256).contains(&handle.secret.len()),
            "invalid scope handle"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let fd = unsafe {
            libc::socket(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                0,
            )
        };
        ensure!(fd >= 0, "scope socket unavailable");
        let mut stream = unsafe { UnixStream::from_raw_fd(fd) };
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (i, b) in handle.socket_name.bytes().enumerate() {
            address.sun_path[i + 1] = b as libc::c_char;
        }
        let length = (std::mem::offset_of!(libc::sockaddr_un, sun_path)
            + 1
            + handle.socket_name.len()) as libc::socklen_t;
        let connected =
            unsafe { libc::connect(fd, (&address as *const libc::sockaddr_un).cast(), length) };
        if connected != 0 {
            let error = std::io::Error::last_os_error();
            ensure!(
                error.raw_os_error() == Some(libc::EINPROGRESS),
                "scope connect refused"
            );
            wait(&stream, libc::POLLOUT, deadline)?;
            let mut error = 0;
            let mut size = std::mem::size_of::<i32>() as libc::socklen_t;
            ensure!(
                unsafe {
                    libc::getsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        libc::SO_ERROR,
                        (&mut error as *mut i32).cast(),
                        &mut size,
                    )
                } == 0
                    && error == 0,
                "scope connect failed"
            );
        }
        let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        ensure!(
            unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut peer as *mut libc::ucred).cast(),
                    &mut size,
                )
            } == 0
                && size as usize == std::mem::size_of::<libc::ucred>()
                && peer.uid == 0,
            "scope peer is not root"
        );
        let body = serde_json::to_vec(&ScopeCheck {
            schema: SCHEMA,
            session_id: registration.session_id,
            incarnation: registration.incarnation,
            token: registration.token.clone(),
            handle: handle.clone(),
        })?;
        ensure!(body.len() <= MAX_FRAME, "scope request exceeds bound");
        write(&mut stream, &(body.len() as u32).to_be_bytes(), deadline)?;
        write(&mut stream, &body, deadline)?;
        let mut header = [0; 4];
        read(&mut stream, &mut header, deadline)?;
        let size = u32::from_be_bytes(header) as usize;
        ensure!(
            size > 0 && size <= MAX_FRAME,
            "scope response exceeds bound"
        );
        let mut bytes = vec![0; size];
        read(&mut stream, &mut bytes, deadline)?;
        serde_json::from_slice::<ScopeReply>(&bytes)?
            .scope
            .context("scope denied")
    }
    use anyhow::Context;
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedScope {
    schema: u32,
    binding: GrantBinding,
    handle: ExecutionScopeHandle,
}
fn cache_name(binding: &GrantBinding) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{}.json",
        hex::encode(Sha256::digest(serde_json::to_vec(binding)?))
    ))
}
fn cache_records(
    path: &std::path::Path,
    cache: &crate::attachment::local_actor::storage::Directory,
) -> Result<usize> {
    let mut count = 0usize;
    let mut entries = 0usize;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        entries += 1;
        ensure!(entries <= 128, "scope cache entry bound exceeded");
        if entry.file_name().as_encoded_bytes().ends_with(b".json") {
            count += 1;
            ensure!(count <= 64, "scope cache capacity exceeded");
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid credential name"))?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            ensure!(
                metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() <= 64 * 1024,
                "unsafe scope credential"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                ensure!(
                    metadata.uid() == unsafe { libc::geteuid() }
                        && metadata.mode() & 0o7777 == 0o600
                        && metadata.nlink() == 1,
                    "unsafe scope credential ownership"
                );
            }
            cache
                .read_bounded(name, 64 * 1024)?
                .ok_or_else(|| anyhow::anyhow!("credential disappeared"))?;
        }
    }
    Ok(count)
}
pub(crate) fn cache_validated(
    directory: &std::path::Path,
    binding: &GrantBinding,
    handle: &ExecutionScopeHandle,
) -> Result<()> {
    let result = (|| -> Result<()> {
        let _runtime =
            crate::attachment::local_actor::storage::Directory::open_existing(directory)?;
        let path = directory.join("scope-credentials");
        let cache = crate::attachment::local_actor::storage::Directory::open(&path)?;
        let _lock = cache.lock()?;
        let name = cache_name(binding)?;
        let count = cache_records(&path, &cache)?;
        let existing = cache.read_bounded(&name, 64 * 1024)?;
        ensure!(
            existing.is_some() || count < 64,
            "scope cache capacity reached"
        );
        let bytes = serde_json::to_vec(&CachedScope {
            schema: SCHEMA,
            binding: binding.clone(),
            handle: handle.clone(),
        })?;
        ensure!(
            bytes.len() <= 64 * 1024,
            "scope credential record bound exceeded"
        );
        cache.publish(&name, &bytes)?;
        cache.verify()?;
        Ok(())
    })();
    result.map_err(|_| anyhow::anyhow!("execution scope credential cache unavailable"))
}
pub(crate) fn cached_handle(
    directory: &std::path::Path,
    binding: &GrantBinding,
) -> Result<ExecutionScopeHandle> {
    let result = (|| -> Result<ExecutionScopeHandle> {
        let _runtime =
            crate::attachment::local_actor::storage::Directory::open_existing(directory)?;
        let cache = crate::attachment::local_actor::storage::Directory::open_existing(
            &directory.join("scope-credentials"),
        )?;
        let _lock = cache.read_lock()?;
        cache_records(&directory.join("scope-credentials"), &cache)?;
        let bytes = cache
            .read_bounded(&cache_name(binding)?, 64 * 1024)?
            .ok_or_else(|| anyhow::anyhow!("scope credential absent"))?;
        let record: CachedScope = serde_json::from_slice(&bytes)?;
        ensure!(
            record.schema == SCHEMA && record.binding == *binding,
            "scope credential binding changed"
        );
        cache.verify()?;
        Ok(record.handle)
    })();
    result.map_err(|_| anyhow::anyhow!("execution scope credential unavailable"))
}
