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
    struct HeldJournal {
        directory: crate::attachment::local_actor::storage::Directory,
        _startup: File,
        journal: crate::attachment::journal::Journal,
        guard: crate::attachment::journal::ExecutionGuard,
    }
    fn inherited_lock(variable: &str, path: &std::path::Path) -> Result<Option<File>> {
        let Some(value) = std::env::var_os(variable) else {
            return Ok(None);
        };
        let fd = value
            .to_str()
            .context("private guard descriptor unavailable")?
            .parse::<i32>()?;
        ensure!(fd > 2, "private guard descriptor cannot be stdio");
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        ensure!(
            flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == 0,
            "private guard close-on-exec unavailable"
        );
        let file = unsafe { File::from_raw_fd(fd) };
        use std::os::unix::fs::OpenOptionsExt;
        let expected = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        let actual = file.metadata()?;
        let expected = expected.metadata()?;
        ensure!(
            actual.is_file()
                && actual.uid() == unsafe { libc::geteuid() }
                && actual.nlink() == 1
                && actual.mode() & 0o077 == 0
                && actual.dev() == expected.dev()
                && actual.ino() == expected.ino(),
            "private retained lock descriptor differs"
        );
        file.try_lock()
            .context("retained lock description unavailable")?;
        Ok(Some(file))
    }
    fn hold(request: &TransitionRequest) -> Result<HeldJournal> {
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
        let startup = inherited_lock(
            "VOYAGE_TRANSITION_STARTUP_FD",
            &directory.join("startup.lock"),
        )?;
        let execution = inherited_lock(
            "VOYAGE_TRANSITION_EXECUTION_FD",
            &directory
                .join("journal")
                .join(format!("{}.execution.lock", request.session_id)),
        )?;
        ensure!(
            startup.is_some() == execution.is_some(),
            "retained rollback needs both lifetime guards"
        );
        if startup.is_some() {
            ensure!(
                matches!(&request.operation,TransitionOperation::SourceLease{freeze,..} if matches!(&freeze.operation,TransitionOperation::Lookup{..})),
                "inherited guards are restricted to original read-only rollback lease"
            );
        }
        let startup = if let Some(file) = startup {
            file
        } else {
            let file =
                crate::attachment::journal::open_private_file(&directory.join("startup.lock"))?;
            file.try_lock()?;
            file
        };
        let journal = crate::attachment::journal::Journal::open(directory.join("journal"))?;
        let guard = if let Some(file) = execution {
            journal.acquire_execution_pinned(request.session_id, file)?
        } else {
            journal.acquire_execution(request.session_id)?
        };
        Ok(HeldJournal {
            directory: _directory,
            _startup: startup,
            journal,
            guard,
        })
    }
    fn apply_held(
        held: &mut HeldJournal,
        request: TransitionRequest,
    ) -> Result<TransitionResponse> {
        let _directory = &held.directory;
        let directory = request.operation.directory();
        let journal = &mut held.journal;
        let guard = &held.guard;
        if matches!(
            &request.operation,
            TransitionOperation::RetainConfiguration { .. }
        ) {
            let value = journal.retain_transition_configuration(guard)?;
            let bytes = value.as_bytes();
            if let Some(existing) =
                _directory.read_bounded("migration-config.json", MAX_TARGET_CONFIG)?
            {
                ensure!(existing == bytes, "retained source configuration changed");
            } else {
                _directory.publish_new("migration-config.json", bytes)?;
            }
            _directory.verify()?;
            return Ok(TransitionResponse::Configuration {
                path: directory.join("migration-config.json"),
                digest: crate::identity_helper::config_digest(bytes),
            });
        }
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
                config.account.is_some()
                    || (expected.source_uid != 0
                        && expected.source_uid == expected.target_uid
                        && expected.source_gid == expected.target_gid
                        && expected.previous_config_digest == expected.target_config_digest),
                "changed identity/configuration and administrator handoff require an explicit target account"
            );
            config.validate_account()?;
            crate::runtime_policy::RuntimePolicy::resolve(&config, &workspace)?
                .policy()
                .check_current()?;
            configuration = Some(String::from_utf8(bytes)?);
        }
        _directory.verify()?;
        journal.retired_transition(guard, &request, configuration.as_deref())
    }
    fn apply(request: TransitionRequest) -> Result<TransitionResponse> {
        let mut held = hold(&request)?;
        apply_held(&mut held, request)
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
            if let TransitionOperation::SourceLease { directory, freeze } = &request.operation {
                ensure!(
                    request.session_id == freeze.session_id
                        && request.source_incarnation == freeze.source_incarnation
                        && freeze.schema == SCHEMA
                        && freeze.operation.directory() == directory
                        && matches!(
                            &freeze.operation,
                            TransitionOperation::SourceFreeze { .. }
                                | TransitionOperation::Lookup { .. }
                        ),
                    "source lease identity differs from exact freeze"
                );
                let mut held = hold(&request)?;
                let response = apply_held(&mut held, *freeze.clone())?;
                ensure!(
                    matches!(
                        &response,
                        TransitionResponse::Prepared { .. }
                            | TransitionResponse::Aborted { .. }
                            | TransitionResponse::Absent { .. }
                    ),
                    "source lease freeze unavailable"
                );
                let bytes = serde_json::to_vec(&response)?;
                ensure!(
                    bytes.len() <= MAX_RESPONSE,
                    "source lease response exceeds bound"
                );
                tokio::time::timeout(Duration::from_secs(2), async {
                    write_all(&output, &(bytes.len() as u32).to_be_bytes()).await?;
                    write_all(&output, &bytes).await
                })
                .await??;
                // The same child keeps both old guards throughout rollback.
                // Read-only lookup may be followed by exact AbortSource; a second
                // helper must not reacquire or replace these lifetime fences.
                let deadline = tokio::time::Instant::now() + Duration::from_secs(900);
                for _ in 0..64 {
                    let next = tokio::time::timeout_at(deadline, async {
                        let mut header = [0; 4];
                        read_exact(&input, &mut header).await?;
                        let length = u32::from_be_bytes(header) as usize;
                        ensure!(
                            length > 0 && length <= MAX_REQUEST,
                            "source lease command bound"
                        );
                        let mut bytes = vec![0; length];
                        read_exact(&input, &mut bytes).await?;
                        Ok::<_, anyhow::Error>(serde_json::from_slice::<TransitionRequest>(&bytes)?)
                    })
                    .await;
                    let next = match next {
                        Ok(Ok(next)) => next,
                        _ => break,
                    };
                    ensure!(
                        next.schema == SCHEMA
                            && next.session_id == request.session_id
                            && next.source_incarnation == request.source_incarnation
                            && next.operation.directory() == directory
                            && matches!(&next.operation, TransitionOperation::AbortSource { .. }),
                        "retained lease accepts only exact original abort"
                    );
                    let reply = apply_held(&mut held, next)?;
                    ensure!(
                        matches!(&reply, TransitionResponse::Aborted { .. }),
                        "retained source abort unavailable"
                    );
                    let bytes = serde_json::to_vec(&reply)?;
                    ensure!(bytes.len() <= MAX_RESPONSE, "source abort response bound");
                    tokio::time::timeout(Duration::from_secs(2), async {
                        write_all(&output, &(bytes.len() as u32).to_be_bytes()).await?;
                        write_all(&output, &bytes).await
                    })
                    .await??;
                }
                drop(held);
                return Ok(TransitionResponse::Unavailable);
            }
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
