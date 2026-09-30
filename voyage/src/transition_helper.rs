//! Private retired-journal worker. Supervisor sees only bounded metadata; each
//! phase parses files exclusively in its original/target operating identity.
use anyhow::{Result, ensure};
use voyage_protocol::execution_transition::*;
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{
        fs::File,
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{FileTypeExt, MetadataExt},
        },
        time::Duration,
    };
    use tokio::io::unix::AsyncFd;
    fn pipe(fd: i32) -> Result<File> {
        let raw = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        ensure!(raw >= 0, "private pipe unavailable");
        let file = unsafe { File::from_raw_fd(raw) };
        let meta = file.metadata()?;
        ensure!(
            meta.file_type().is_fifo()
                && meta.uid() == 0
                && meta.mode() & 0o077 == 0
                && std::fs::read_link(format!("/proc/self/fd/{raw}"))?
                    .as_os_str()
                    .as_encoded_bytes()
                    .starts_with(b"pipe:["),
            "root private pipe required"
        );
        let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
        ensure!(
            flags >= 0 && unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } == 0,
            "private pipe cannot be bounded"
        );
        Ok(file)
    }
    fn identity() -> Result<()> {
        ensure!(
            unsafe { libc::getuid() } == unsafe { libc::geteuid() }
                && unsafe { libc::getgid() } == unsafe { libc::getegid() },
            "mixed helper identity"
        );
        for kind in ["uid", "gid"] {
            let value = std::fs::read_to_string(format!("/proc/self/{kind}_map"))?;
            let words = value.split_whitespace().collect::<Vec<_>>();
            ensure!(
                words == ["0", "0", "4294967295"],
                "helper must remain in initial user namespace"
            );
        }
        Ok(())
    }
    async fn read_exact(fd: &AsyncFd<File>, mut bytes: &mut [u8]) -> Result<()> {
        while !bytes.is_empty() {
            let mut ready = fd.readable().await?;
            match ready.try_io(|f| f.get_ref().read(bytes)) {
                Ok(Ok(0)) => anyhow::bail!("private pipe closed"),
                Ok(Ok(n)) => bytes = &mut bytes[n..],
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => (),
                Ok(Err(e)) => return Err(e.into()),
                Err(_) => (),
            }
        }
        Ok(())
    }
    async fn write_all(fd: &AsyncFd<File>, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            let mut ready = fd.writable().await?;
            match ready.try_io(|f| f.get_ref().write(bytes)) {
                Ok(Ok(0)) => anyhow::bail!("private pipe closed"),
                Ok(Ok(n)) => bytes = &bytes[n..],
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => (),
                Ok(Err(e)) => return Err(e.into()),
                Err(_) => (),
            }
        }
        Ok(())
    }
    fn target_bytes(path: &std::path::Path) -> Result<Vec<u8>> {
        ensure!(
            path.is_absolute()
                && !path.components().any(|p| matches!(
                    p,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )),
            "target path must be absolute and normalized"
        );
        let parent = path
            .parent()
            .context("target configuration parent unavailable")?;
        let directory = crate::attachment::local_actor::storage::Directory::open_existing(parent)?;
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .context("target configuration filename unavailable")?;
        directory
            .read_bounded(name, MAX_TARGET_CONFIG)?
            .context("target configuration unavailable")
    }
    use anyhow::Context;
    fn apply(request: TransitionRequest) -> Result<TransitionResponse> {
        ensure!(request.schema == SCHEMA, "unsupported handoff schema");
        let directory = request.operation.directory();
        ensure!(
            directory.is_absolute()
                && directory.is_dir()
                && !directory.components().any(|p| matches!(
                    p,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )),
            "invalid runtime directory"
        );
        // Validate as this operating user, never as the supervisor. Keep the
        // directory capability and both lifetime fences until after SQL commit.
        let _directory =
            crate::attachment::local_actor::storage::Directory::open_existing(directory)?;
        ensure!(
            directory.join("journal/journal.sqlite3").is_file(),
            "runtime journal unavailable"
        );
        let startup =
            crate::attachment::journal::open_private_file(&directory.join("startup.lock"))?;
        startup.try_lock()?;
        let mut journal = crate::attachment::journal::Journal::open(directory.join("journal"))?;
        let guard = journal.acquire_execution(request.session_id)?;
        let mut configuration = None;
        if let TransitionOperation::TargetCommit {
            expected,
            target_config_path,
            ..
        } = &request.operation
        {
            ensure!(
                unsafe { libc::geteuid() } == expected.target_uid
                    && unsafe { libc::getegid() } == expected.target_gid,
                "target operating identity differs"
            );
            let bytes = target_bytes(target_config_path)?;
            ensure!(
                crate::identity_helper::config_digest(&bytes) == expected.target_config_digest,
                "reviewed target configuration changed"
            );
            let workspace = journal.load_session(request.session_id)?.session.workspace;
            let launch: crate::launch_config::LaunchConfig = serde_json::from_slice(&bytes)?;
            let config = launch.resolve(&workspace)?;
            ensure!(
                config.account.is_some(),
                "handoff requires an explicit target account"
            );
            config.validate_account()?;
            crate::runtime_policy::RuntimePolicy::resolve(&config, &workspace)?
                .policy()
                .check_current()?;
            configuration = Some(String::from_utf8(bytes)?);
        }
        _directory.verify()?;
        journal.retired_transition(&guard, &request, configuration.as_deref())
    }
    pub(super) async fn run() -> Result<()> {
        identity()?;
        let input = AsyncFd::new(pipe(0)?)?;
        let output = AsyncFd::new(pipe(1)?)?;
        let response = async {
            let bytes = tokio::time::timeout(Duration::from_secs(5), async {
                let mut header = [0; 4];
                read_exact(&input, &mut header).await?;
                let length = u32::from_be_bytes(header) as usize;
                ensure!(
                    length > 0 && length <= MAX_REQUEST,
                    "handoff input bound exceeded"
                );
                let mut data = vec![0; length];
                read_exact(&input, &mut data).await?;
                Ok::<_, anyhow::Error>(data)
            })
            .await??;
            let request: TransitionRequest = serde_json::from_slice(&bytes)?;
            // This worker has no detached effect. The root caller independently
            // bounds/kills its owned process if filesystem/SQLite work stalls.
            tokio::task::spawn_blocking(move || apply(request)).await?
        }
        .await
        .unwrap_or(TransitionResponse::Unavailable);
        let bytes = serde_json::to_vec(&response)?;
        ensure!(bytes.len() <= MAX_RESPONSE, "handoff output bound exceeded");
        tokio::time::timeout(Duration::from_secs(2), async {
            write_all(&output, &(bytes.len() as u32).to_be_bytes()).await?;
            write_all(&output, &bytes).await
        })
        .await??;
        Ok(())
    }
}
pub async fn run() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::run()
            .await
            .map_err(|_| anyhow::anyhow!("retired journal helper unavailable"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("retired journal helper unavailable");
    }
}
