//! Host-operator migration adapters. Source data is read under its original UID;
//! root rebuilds only ordinary authority. No session executor lives here.
use super::{access::store, database, registry};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
            net::UnixStream,
        },
    },
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
use voyage_protocol::{
    execution_identity::{AuthorityClass, ExecutionBinding},
    process::*,
};

const FRAME: usize = 128 * 1024 * 1024;
const TOTAL: u64 = 8 * 1024 * 1024 * 1024;
#[derive(clap::Args)]
pub struct UserArgs {
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub pipe_fd: i32,
}
#[derive(clap::Args)]
pub struct ControlArgs {
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub pipe_fd: i32,
}
pub use voyage_protocol::migration::{ControlRequest, Frame, Import, Snapshot, UserRequest};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Dormant {
    schema_version: u32,
    operation_id: Uuid,
    review_digest: String,
    session_id: Uuid,
    incarnation: Uuid,
    source_boot: Uuid,
    target_boot: Uuid,
    uid: u32,
    incarnation_never_started: bool,
}
fn pipe(fd: i32) -> Result<UnixStream> {
    ensure!(fd > 2, "migration needs a private root pipe");
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    ensure!(
        unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut peer as *mut libc::ucred).cast(),
                &mut length,
            )
        } == 0
            && peer.uid == 0,
        "migration pipe requires root peer"
    );
    stream.set_read_timeout(Some(Duration::from_secs(45)))?;
    stream.set_write_timeout(Some(Duration::from_secs(45)))?;
    Ok(stream)
}
pub fn read_frame<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    ensure!(
        length > 0 && length <= FRAME,
        "migration frame exceeds bound"
    );
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid private migration frame"))
}
pub fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= FRAME, "migration frame exceeds bound");
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}
fn private(path: &Path, limit: u64) -> Result<Vec<u8>> {
    store::read_bounded(path, limit)
}
fn source_file(path: &Path) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.nlink() == 1
            && meta.mode() & 0o077 == 0
            && meta.len() <= 2 * 1024 * 1024 * 1024,
        "unsafe ordinary migration artifact"
    );
    Ok(file)
}
fn files(root: &Path, prefix: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    if !root.try_exists()? {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink()
            && matches!(
                path.file_name().and_then(|s| s.to_str()),
                Some("SingletonLock" | "SingletonSocket" | "SingletonCookie")
            )
        {
            continue;
        }
        ensure!(
            !meta.file_type().is_symlink() && meta.uid() == unsafe { libc::geteuid() },
            "unsafe migration tree"
        );
        if meta.is_dir() {
            ensure!(
                meta.mode() & 0o077 == 0,
                "migration directory is not private"
            );
            files(&path, prefix, output)?;
        } else if meta.is_file() {
            let _ = source_file(&path)?;
            output.push(path.strip_prefix(prefix)?.to_owned());
        } else {
            ensure!(
                meta.file_type().is_socket(),
                "unsupported migration resource"
            );
        }
        ensure!(
            output.len() <= 100000,
            "migration artifact count exceeds bound"
        );
    }
    Ok(())
}
fn digest_tree(root: &Path) -> Result<String> {
    let mut paths = Vec::new();
    files(root, root, &mut paths)?;
    paths.sort();
    let mut hash = Sha256::new();
    let mut total = 0u64;
    for relative in paths {
        hash.update(relative.as_os_str().as_encoded_bytes());
        let mut file = source_file(&root.join(relative))?;
        let mut buf = [0; 65536];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            ensure!(total <= TOTAL, "migration tree exceeds bound");
            hash.update(&buf[..n]);
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn provider_fingerprint(home: &Path) -> Result<String> {
    // Only a private digest crosses the pipe. Provider records and credentials
    // stay in the original executing account; no provider bytes are exported.
    digest_tree(&home.join(".local/share/helm"))
}
fn records<T: serde::de::DeserializeOwned>(directory: &Path) -> Result<Vec<T>> {
    let mut result = Vec::new();
    if !directory.try_exists()? {
        return Ok(result);
    }
    let mut paths = fs::read_dir(directory)?
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        paths.len() <= 4096,
        "ordinary authority export exceeds bound"
    );
    paths.sort();
    for path in paths {
        ensure!(
            path.extension().is_some_and(|s| s == "json"),
            "unknown ordinary authority artifact"
        );
        result.push(serde_json::from_slice(&private(&path, 65536)?)?);
        ensure!(
            result.len() <= 4096,
            "ordinary authority export exceeds bound"
        );
    }
    Ok(result)
}
fn snapshot(root: &Path, held: bool) -> Result<Snapshot> {
    registry::private_directory(root)?;
    let _owner = if held {
        None
    } else {
        Some(registry::lock(root)?)
    };
    let db = database::open(root)?;
    let mut statement = db.prepare("SELECT registration FROM voyages ORDER BY session_id")?;
    let sessions = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|row| Ok(serde_json::from_str::<ProcessRegistration>(&row?)?))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        sessions.len() <= 4096 && sessions.iter().all(|r| r.peer_uids.is_none()),
        "legacy export requires ordinary unbound user registrations"
    );
    let mut statement =
        db.prepare("SELECT DISTINCT command_id FROM lifecycle_commands ORDER BY command_id")?;
    let legacy_commands = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|row| Ok(Uuid::parse_str(&row?)?))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        legacy_commands.len() <= 100000,
        "legacy command export exceeds bound"
    );
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='execution_profiles')",
        [],
        |r| r.get(0),
    )?;
    let profiles = if exists {
        db.query_row("SELECT state FROM execution_profiles WHERE id=1", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .map(|s| serde_json::from_str(&s))
        .transpose()?
    } else {
        None
    };
    let home = PathBuf::from(std::env::var_os("HOME").context("ordinary execution home missing")?);
    Ok(Snapshot {
        schema_version: 1,
        uid: unsafe { libc::geteuid() },
        home: home.clone(),
        identity: serde_json::from_slice(&private(&root.join("identity/key.json"), 16384)?)?,
        sessions,
        connections: records(&root.join("access/connections"))?,
        grants: records(&root.join("access/grants"))?,
        grant_credentials: records(&root.join("access/credentials"))?,
        trusted_vessels: records(&root.join("trusted-vessels"))?,
        profiles,
        legacy_commands,
        pairing: super::pairing::migration_export(root)?,
        provider_fingerprint: provider_fingerprint(&home)?,
    })
}
fn relative(path: &Path) -> Result<()> {
    ensure!(
        path.is_relative()
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && path.components().count() <= 32,
        "migration path is not a bounded relative artifact"
    );
    Ok(())
}
async fn unit_fingerprint(unit: &str) -> Result<String> {
    ensure!(
        unit.ends_with(".service")
            && unit.len() <= 128
            && unit
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_@.".contains(&b)),
        "invalid ordinary unit identity"
    );
    let shown = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new("/usr/bin/systemctl")
            .args([
                "--user",
                "show",
                unit,
                "--property=ExecStart,ExecStop,FragmentPath,DropInPaths,KillMode",
                "--value",
            ])
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await??;
    ensure!(
        shown.status.success() && shown.stdout.len() <= 32768,
        "ordinary unit review unavailable"
    );
    let text = String::from_utf8(shown.stdout)?;
    let stable = text
        .lines()
        .map(|line| line.split(" ; start_time=").next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!("{:x}", Sha256::digest(stable.as_bytes())))
}
pub async fn user(args: UserArgs) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } != 0 && unsafe { libc::getuid() } == unsafe { libc::geteuid() },
        "user migration helper requires its original ordinary identity"
    );
    let mut channel = pipe(args.pipe_fd)?;
    let action: UserRequest = read_frame(&mut channel)?;
    match action {
        UserRequest::HoldOwner => {
            registry::private_directory(&args.directory)?;
            let _owner = registry::lock(&args.directory)?;
            write_frame(&mut channel, &Frame::OwnerHeld)?;
            channel.set_read_timeout(Some(Duration::from_secs(900)))?;
            // EOF or bounded lifetime releases ownership. No effect/command is
            // accepted while root retains this original-UID namespace lease.
            let mut unexpected = [0; 1];
            let _ = channel.read(&mut unexpected);
        }
        UserRequest::Quiesce => {
            let response = super::exchange::exchange(
                &args.directory,
                &VesselRequest {
                    protocol: voyage_protocol::vessel::VESSEL_API_VERSION,
                    command: VesselCommand::Catalogue,
                },
            )
            .await?;
            ensure!(
                response.error.is_none(),
                "legacy catalogue unavailable for stop review"
            );
            let sessions: Vec<ProcessInfo> = serde_json::from_value(response.result)?;
            ensure!(sessions.len() <= 4096, "legacy voyage stop limit");
            let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
            for session in sessions {
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "legacy stop review exceeded deadline; inspect the same migration before replay"
                );
                let _ = super::exchange::exchange(
                    &args.directory,
                    &VesselRequest {
                        protocol: voyage_protocol::vessel::VESSEL_API_VERSION,
                        command: VesselCommand::Stop {
                            session_id: session.session_id,
                            incarnation: session.incarnation,
                        },
                    },
                )
                .await;
            }
            // User manager effects execute under the original account, never root.
            let result = tokio::time::timeout(
                Duration::from_secs(20),
                tokio::process::Command::new("/usr/bin/systemctl")
                    .args([
                        "--user",
                        "--no-pager",
                        "list-units",
                        "--type=service",
                        "--all",
                        "--output=json",
                    ])
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output(),
            )
            .await??;
            ensure!(
                result.status.success() && result.stdout.len() <= 1024 * 1024,
                "ordinary service inventory unavailable"
            );
            let inventory: Vec<serde_json::Value> = serde_json::from_slice(&result.stdout)?;
            let mut units = Vec::new();
            for item in inventory {
                let name = item["unit"].as_str().context("ordinary unit has no name")?;
                ensure!(
                    name.ends_with(".service")
                        && name.len() <= 128
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_@.".contains(&b)),
                    "invalid ordinary unit identity"
                );
                let shown = tokio::time::timeout(
                    Duration::from_secs(20),
                    tokio::process::Command::new("/usr/bin/systemctl")
                        .args(["--user", "show", name, "--property=ExecStart", "--value"])
                        .stderr(std::process::Stdio::null())
                        .output(),
                )
                .await??;
                ensure!(
                    shown.stdout.len() <= 16384,
                    "ordinary unit definition exceeds bound"
                );
                let definition = String::from_utf8(shown.stdout)?;
                let directory = args
                    .directory
                    .to_str()
                    .context("ordinary source path is not UTF8")?;
                if definition.contains(directory)
                    && (definition.contains("serve") || definition.contains("--vessel-directory"))
                    && definition.contains("vessel")
                {
                    units.push(name.to_owned());
                }
            }
            ensure!(
                !units.is_empty() && units.len() <= 16,
                "reviewed ordinary manager has no matching Vessel services"
            );
            let mut definitions = std::collections::BTreeMap::new();
            for unit in &units {
                definitions.insert(unit.clone(), unit_fingerprint(unit).await?);
                let status = tokio::process::Command::new("/usr/bin/systemctl")
                    .args(["--user", "--no-block", "stop", unit])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                let status = tokio::time::timeout(Duration::from_secs(20), status).await??;
                ensure!(status.success(), "ordinary manager stop unconfirmed");
                let status = tokio::time::timeout(
                    Duration::from_secs(20),
                    tokio::process::Command::new("/usr/bin/systemctl")
                        .args(["--user", "disable", unit])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status(),
                )
                .await??;
                ensure!(status.success(), "ordinary manager disable unconfirmed");
            }
            write_frame(
                &mut channel,
                &Frame::Quiesced {
                    stop_requested: true,
                    services: units,
                    definitions,
                },
            )?;
        }
        UserRequest::RestoreServices { definitions } => {
            ensure!(
                !definitions.is_empty() && definitions.len() <= 16,
                "ordinary restore unit limit"
            );
            for (unit, expected) in definitions {
                ensure!(
                    unit_fingerprint(&unit).await? == expected,
                    "original unit configuration changed since review"
                );
                for action in ["enable", "start"] {
                    let status = tokio::time::timeout(
                        Duration::from_secs(20),
                        tokio::process::Command::new("/usr/bin/systemctl")
                            .args(["--user", "--no-block", action, &unit])
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status(),
                    )
                    .await??;
                    ensure!(status.success(), "original user restore unconfirmed");
                }
            }
            write_frame(&mut channel, &Frame::Complete)?;
        }
        UserRequest::ObserveRestoredServices { definitions } => {
            ensure!(
                !definitions.is_empty() && definitions.len() <= 16,
                "ordinary restored-unit observation limit"
            );
            let mut ready = true;
            for (unit, expected) in definitions {
                ensure!(
                    unit_fingerprint(&unit).await? == expected,
                    "restored user unit configuration changed"
                );
                let mut command = tokio::process::Command::new("/usr/bin/systemctl");
                command
                    .args([
                        "--user",
                        "show",
                        "--property=ActiveState,MainPID,Job",
                        &unit,
                    ])
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true);
                let output =
                    tokio::time::timeout(Duration::from_secs(10), command.output()).await??;
                ensure!(
                    output.status.success() && output.stdout.len() <= 16384,
                    "user service observation unavailable"
                );
                let text = String::from_utf8(output.stdout)?;
                let properties = text
                    .lines()
                    .filter_map(|line| line.split_once('='))
                    .collect::<std::collections::BTreeMap<_, _>>();
                ready &= properties.get("ActiveState") == Some(&"active")
                    && properties.get("Job") == Some(&"")
                    && properties
                        .get("MainPID")
                        .and_then(|v| v.parse::<u32>().ok())
                        .is_some_and(|p| p > 1);
            }
            write_frame(&mut channel, &Frame::RestoredServices { ready })?;
        }
        UserRequest::ProviderFingerprint => {
            let home = PathBuf::from(std::env::var_os("HOME").context("original home missing")?);
            write_frame(
                &mut channel,
                &Frame::ProviderFingerprint {
                    sha256: provider_fingerprint(&home)?,
                },
            )?;
        }
        UserRequest::PrepareConfig { session_id } => {
            let directory = registry::directory(&args.directory, session_id);
            registry::private_directory(&directory)?;
            let db = rusqlite::Connection::open_with_flags(
                directory.join("journal/journal.sqlite3"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let configuration: String = db.query_row(
                "SELECT settings FROM process_configuration WHERE session_id=?1",
                [session_id.to_string()],
                |r| r.get(0),
            )?;
            ensure!(
                configuration.len() <= 65536,
                "private frozen configuration exceeds bound"
            );
            let path = directory.join("migration-config.json");
            if path.try_exists()? {
                ensure!(
                    private(&path, 65536)? == configuration.as_bytes(),
                    "private migration configuration conflict"
                );
            } else {
                store::save_bytes(&path, configuration.as_bytes(), 65536)?;
            }
            let mut digest = Sha256::new();
            digest.update(b"voyage/identity-launch-config/v1\0");
            digest.update(configuration.as_bytes());
            write_frame(
                &mut channel,
                &Frame::Config {
                    sha256: format!("{:x}", digest.finalize()),
                },
            )?;
        }
        export @ (UserRequest::Export | UserRequest::ExportHeld) => {
            let held = matches!(export, UserRequest::ExportHeld);
            let snapshot = snapshot(&args.directory, held)?;
            let ids = snapshot
                .sessions
                .iter()
                .map(|s| s.session_id)
                .collect::<Vec<_>>();
            let mut hash = Sha256::new();
            hash.update(serde_json::to_vec(&snapshot)?);
            write_frame(&mut channel, &Frame::Snapshot { snapshot })?;
            let mut total = 0u64;
            for id in ids {
                let directory = registry::directory(&args.directory, id);
                let mut paths = Vec::new();
                files(&directory, &directory, &mut paths)?;
                paths.retain(|p| {
                    !matches!(
                        p.to_str(),
                        Some(
                            "registration.json"
                                | "stopped.json"
                                | "suspended.json"
                                | "startup.lock"
                                | "owner.lock"
                                | "runtime.sock"
                                | "recovered-stop.json"
                        )
                    )
                });
                paths.sort();
                for path in paths {
                    relative(&path)?;
                    let mut file = source_file(&directory.join(&path))?;
                    let mut offset = 0;
                    let mut buffer = [0; 65536];
                    loop {
                        let count = file.read(&mut buffer)?;
                        let last = count == 0;
                        total += count as u64;
                        ensure!(total <= TOTAL, "migration export exceeds bound");
                        hash.update(id.as_bytes());
                        hash.update(path.as_os_str().as_encoded_bytes());
                        hash.update(&buffer[..count]);
                        write_frame(
                            &mut channel,
                            &Frame::File {
                                session_id: id,
                                relative: path.clone(),
                                offset,
                                bytes: STANDARD.encode(&buffer[..count]),
                                last,
                            },
                        )?;
                        offset += count as u64;
                        if last {
                            break;
                        }
                    }
                }
            }
            write_frame(
                &mut channel,
                &Frame::End {
                    sha256: format!("{:x}", hash.finalize()),
                },
            )?;
        }
        UserRequest::Restore { runtime_root } => {
            ensure!(
                runtime_root.is_absolute(),
                "migration runtime root must be absolute"
            );
            let mut open: Option<(Uuid, PathBuf, fs::File, u64)> = None;
            let mut total = 0;
            loop {
                match read_frame::<Frame>(&mut channel)? {
                    Frame::File {
                        session_id,
                        relative: path,
                        offset,
                        bytes,
                        last,
                    } => {
                        relative(&path)?;
                        let directory = runtime_root.join(session_id.to_string());
                        registry::private_directory(&directory)?;
                        if offset == 0 {
                            ensure!(open.is_none(), "overlapping migration artifact");
                            let target = directory.join(&path);
                            if let Some(parent) = target.parent() {
                                registry::private_directory(parent)?;
                            }
                            let file = fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .mode(0o600)
                                .custom_flags(libc::O_NOFOLLOW)
                                .open(target)?;
                            open = Some((session_id, path.clone(), file, 0));
                        }
                        let (id, relative, file, position) = open
                            .as_mut()
                            .context("migration artifact has no first frame")?;
                        ensure!(
                            *id == session_id && *relative == path && *position == offset,
                            "migration artifact offset conflict"
                        );
                        let data = STANDARD.decode(bytes)?;
                        ensure!(data.len() <= 65536, "migration chunk exceeds bound");
                        total += data.len() as u64;
                        ensure!(total <= TOTAL, "migration restore exceeds bound");
                        file.write_all(&data)?;
                        *position += data.len() as u64;
                        if last {
                            file.sync_all()?;
                            open = None;
                        }
                    }
                    Frame::End { .. } => {
                        ensure!(open.is_none(), "migration stream ended mid-artifact");
                        write_frame(&mut channel, &Frame::Complete)?;
                        break;
                    }
                    _ => anyhow::bail!("invalid migration restore frame"),
                }
            }
        }
    }
    Ok(())
}

pub async fn control(args: ControlArgs) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0 && unsafe { libc::getuid() } == 0,
        "migration control requires real root"
    );
    for kind in ["uid", "gid"] {
        let map = fs::read_to_string(format!("/proc/self/{kind}_map"))?;
        let fields = map.split_whitespace().collect::<Vec<_>>();
        ensure!(
            fields == ["0", "0", "4294967295"],
            "migration control requires full host identity maps"
        );
    }
    let mut channel = pipe(args.pipe_fd)?;
    let request: ControlRequest = read_frame(&mut channel)?;
    let import = match request {
        ControlRequest::Import { import } => import,
        ControlRequest::Quiescence { drain } => {
            let _root = voyage_storage::protected_linux::RootDirectory::open(&args.directory)?;
            let path = args.directory.join("catalogue.sqlite3");
            if path.try_exists()? {
                let file = source_file(&path)?;
                drop(file);
                let db = rusqlite::Connection::open_with_flags(
                    path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                let mut query = db.prepare("SELECT registration FROM voyages")?;
                for row in query.query_map([], |r| r.get::<_, String>(0))? {
                    let record: ProcessRegistration = serde_json::from_str(&row?)?;
                    ensure!(
                        record.peer_uids.as_ref().is_some_and(|u| u.supervisor == 0),
                        "unbound system owner requires explicit legacy recovery, not fabricated retirement"
                    );
                    // Resolve the protected binding before trusting retirement. This also
                    // admits explicit administrator UID0 identities without guessing
                    // authority from the peer UID alone.
                    let _identity =
                        database::bound_observer_identity(&args.directory, &record).await?;
                    let retired = || {
                        super::guardian::cleanup_observed(
                            &args.directory,
                            record.session_id,
                            record.incarnation,
                        )
                        .unwrap_or(false)
                            || dormant(&args.directory, record.session_id, record.incarnation)
                                .unwrap_or(false)
                    };
                    if !retired() && drain {
                        super::guardian::request_stop(
                            &args.directory,
                            record.session_id,
                            record.incarnation,
                        )?;
                        let deadline = std::time::Instant::now() + Duration::from_secs(35);
                        while !retired() && std::time::Instant::now() < deadline {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                    ensure!(retired(), "system voyage retirement is not observed");
                    if drain {
                        freeze_system_journal(&args.directory, &record).await?;
                    }
                }
            }
            write_frame(
                &mut channel,
                &Frame::Quiescent {
                    incarnations_retired: true,
                },
            )?;
            return Ok(());
        }
    };
    ensure!(
        import.source_boot != import.target_boot
            && import.target_boot
                == Uuid::parse_str(fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim())?,
        "legacy migration needs observed host boot retirement; a user stopped marker is not evidence"
    );
    ensure!(
        !import.operation_id.is_nil()
            && import.review_digest.len() == 64
            && import.identity.uid == import.snapshot.uid
            && import.identity.home == import.snapshot.home
            && import.identity.authority == AuthorityClass::Ordinary,
        "migration ordinary identity mismatch"
    );
    super::launch::validate_identity(&import.identity)?;
    super::launch::protected_binary(&import.executable)?;
    let control = voyage_storage::protected_linux::RootDirectory::open(&args.directory)?;
    let mut db = database::open(&args.directory)?;
    let count: i64 = db.query_row("SELECT count(*) FROM voyages", [], |r| r.get(0))?;
    ensure!(
        count == 0,
        "migration target already has canonical registrations"
    );
    registry::private_directory(&args.directory.join("sessions"))?;
    let identity_bytes = serde_json::to_vec(&import.snapshot.identity)?;
    ensure!(
        identity_bytes.len() <= 16384,
        "legacy signing identity exceeds bound"
    );
    registry::private_directory(&args.directory.join("identity"))?;
    store::save_bytes(
        &args.directory.join("identity/key.json"),
        &identity_bytes,
        16384,
    )?;
    let vessel = super::identity::public(&args.directory)?.vessel_id;
    let taint=import.snapshot.connections.iter().map(|g|serde_json::json!({"grant_id":g.grant_id,"principal_id":g.principal_id,"revision":g.revision})).collect::<Vec<_>>();
    let taint_bytes = serde_json::to_vec(&taint)?;
    ensure!(
        taint_bytes.len() <= 512 * 1024,
        "legacy connection taint exceeds bound"
    );
    control.publish_new(
        "legacy-user-connections.json".as_ref(),
        &taint_bytes,
        512 * 1024,
    )?;
    registry::private_directory(&args.directory.join("access/connections"))?;
    registry::private_directory(&args.directory.join("access/grants"))?;
    for grant in &import.snapshot.connections {
        ensure!(
            grant.vessel_id == vessel
                && grant.schema_version == 1
                && grant.valid_owner_scope()
                && !grant.grant_id.is_nil()
                && !grant.principal_id.is_nil(),
            "invalid ordinary legacy connection"
        );
        store::save(
            &store::connection_path(&args.directory, grant.grant_id),
            grant,
        )?;
    }
    registry::private_directory(&args.directory.join("access/credentials"))?;
    for credential in &import.snapshot.grant_credentials {
        ensure!(import.snapshot.grants.iter().any(|g|g.grant_id==credential.grant_id && g.session_id==credential.session_id),"legacy credential has no matching ordinary grant");
        store::save(
            &store::credential_path(&args.directory, credential.grant_id),
            credential,
        )?;
    }
    for peer in &import.snapshot.trusted_vessels {
        super::identity::pin(&args.directory, peer)?;
    }
    super::pairing::migration_import(&args.directory, &import.snapshot.pairing)?;
    database::store_identity(&args.directory, &import.identity).await?;
    let markers = args.directory.join("migration-dormant");
    registry::private_directory(&markers)?;
    for old in &import.snapshot.sessions {
        ensure!(
            old.peer_uids.is_none() && !old.session_id.is_nil(),
            "migration source is not an ordinary user voyage"
        );
        let mut registration = old.clone();
        registration.executable = Some(import.executable.clone());
        registration.incarnation = *import
            .target_incarnations
            .get(&old.session_id)
            .context("migration target incarnation missing")?;
        registration.command_id = Uuid::new_v4();
        registration.restart_from = None;
        if !matches!(
            registration.initialize,
            Some(RuntimeInitialization::Participant { .. })
        ) {
            registration.initialize = None;
        }
        registration.token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        registration.peer_uids = Some(ProcessPeerUids {
            supervisor: 0,
            runtime: import.identity.uid,
        });
        registration.state = ProcessState::Suspended;
        let binding = ExecutionBinding {
            session_id: registration.session_id,
            incarnation: registration.incarnation,
            identity: import.identity.identity.clone(),
            account_context: import.identity.account_context.clone(),
            peer_uids: registration.peer_uids.clone().unwrap(),
            administrator_grant_id: None,
            host_identity_digest: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&import.identity)?)
            ),
            policy_digest: import.review_digest.clone(),
        };
        database::admit_with_binding(
            &args.directory,
            &registration,
            serde_json::to_vec(&VesselCommand::Inspect {
                session_id: registration.session_id,
            })?,
            Some(&binding),
        )
        .await?;
        store::save(
            &markers.join(format!("{}.json", registration.session_id)),
            &Dormant {
                schema_version: 1,
                operation_id: import.operation_id,
                review_digest: import.review_digest.clone(),
                session_id: registration.session_id,
                incarnation: registration.incarnation,
                source_boot: import.source_boot,
                target_boot: import.target_boot,
                uid: import.identity.uid,
                incarnation_never_started: true,
            },
        )?;
    }
    for grant in &import.snapshot.grants {
        ensure!(
            !grant.grant_id.is_nil()
                && !grant.session_id.is_nil()
                && import
                    .snapshot
                    .sessions
                    .iter()
                    .any(|s| s.session_id == grant.session_id),
            "invalid ordinary legacy session grant"
        );
        store::save(&store::grant_path(&args.directory, grant.grant_id), grant)?;
        // Root has just rebuilt an ordinary binding for this original UID/home.
        // No administrator binding or lazy authority is imported from user data.
        super::access::execution_epoch::pin(&args.directory, grant)?;
    }
    let tx = db.transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS legacy_migration_commands(command_id TEXT PRIMARY KEY, operation_id TEXT NOT NULL) STRICT;")?;
    for id in &import.snapshot.legacy_commands {
        tx.execute(
            "INSERT INTO legacy_migration_commands VALUES(?1,?2)",
            params![id.to_string(), import.operation_id.to_string()],
        )?;
    }
    if let Some(profiles) = &import.snapshot.profiles {
        let _: voyage_protocol::execution_profiles::ProfileCatalogue =
            serde_json::from_value(profiles.clone())?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS execution_profiles(id INTEGER PRIMARY KEY CHECK(id=1),state TEXT NOT NULL CHECK(json_valid(state))) STRICT;")?;
        tx.execute(
            "INSERT INTO execution_profiles VALUES(1,?1)",
            [serde_json::to_string(profiles)?],
        )?;
    }
    tx.commit()?;
    write_frame(&mut channel, &Frame::Complete)?;
    Ok(())
}

pub(super) fn dormant(root: &Path, session: Uuid, incarnation: Uuid) -> Result<bool> {
    let path = root
        .join("migration-dormant")
        .join(format!("{session}.json"));
    if !path.try_exists()? {
        return Ok(false);
    }
    let _control = voyage_storage::protected_linux::RootDirectory::open(root)?;
    let marker: Dormant = serde_json::from_slice(&private(&path, 16384)?)?;
    ensure!(
        marker.schema_version == 1
            && marker.session_id == session
            && marker.incarnation == incarnation
            && marker.source_boot != marker.target_boot
            && marker.uid != 0
            && marker.incarnation_never_started,
        "invalid dormant migration evidence"
    );
    // A real protected guardian admission always wins. This marker only proves
    // the new incarnation has never launched, not cleanup of a live process.
    Ok(!root
        .join("guardians")
        .join(session.to_string())
        .join(incarnation.to_string())
        .join("admission.json")
        .try_exists()?)
}

async fn retired_request(
    identity: &voyage_protocol::execution_identity::ConfiguredExecutionIdentity,
    request: &voyage_protocol::execution_transition::TransitionRequest,
) -> Result<voyage_protocol::execution_transition::TransitionResponse> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let executable = std::env::current_exe()?.with_file_name("voyage");
    super::launch::protected_binary(&executable)?;
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("transition-helper")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    super::launch::configure_identity(command.as_std_mut(), identity)?;
    let mut child = command.spawn()?;
    let mut input = child.stdin.take().context("root handoff input missing")?;
    let bytes = serde_json::to_vec(request)?;
    ensure!(
        bytes.len() <= voyage_protocol::execution_transition::MAX_REQUEST,
        "root handoff frame exceeds bound"
    );
    input.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    input.write_all(&bytes).await?;
    drop(input);
    let response = tokio::time::timeout(Duration::from_secs(10), async {
        let mut output = child.stdout.take().context("root handoff output missing")?;
        let mut header = [0; 4];
        output.read_exact(&mut header).await?;
        let length = u32::from_be_bytes(header) as usize;
        ensure!(
            length > 0 && length <= voyage_protocol::execution_transition::MAX_RESPONSE,
            "root handoff response exceeds bound"
        );
        let mut bytes = vec![0; length];
        output.read_exact(&mut bytes).await?;
        ensure!(child.wait().await?.success(), "root handoff helper failed");
        Ok::<_, anyhow::Error>(serde_json::from_slice(&bytes)?)
    })
    .await;
    match response {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!("root handoff unconfirmed; lookup exact retained command identity")
        }
    }
}
async fn freeze_system_journal(root: &Path, registration: &ProcessRegistration) -> Result<()> {
    use voyage_protocol::execution_transition::*;
    let identity = database::bound_observer_identity(root, registration).await?;
    let directory = super::runtime_storage::directory(root, registration).await?;
    let facts = match retired_request(
        &identity,
        &TransitionRequest {
            schema: SCHEMA,
            session_id: registration.session_id,
            source_incarnation: registration.incarnation,
            operation: TransitionOperation::Observe {
                directory: directory.clone(),
            },
        },
    )
    .await?
    {
        TransitionResponse::Facts { facts } => facts,
        _ => anyhow::bail!("retired root-system journal unavailable"),
    };
    let (config_path, config_digest) = match retired_request(
        &identity,
        &TransitionRequest {
            schema: SCHEMA,
            session_id: registration.session_id,
            source_incarnation: registration.incarnation,
            operation: TransitionOperation::RetainConfiguration {
                directory: directory.clone(),
            },
        },
    )
    .await?
    {
        TransitionResponse::Configuration { path, digest } => (path, digest),
        _ => anyhow::bail!("retired root-system configuration unavailable"),
    };
    ensure!(
        config_digest == facts.frozen_config_digest,
        "root-system retained configuration differs"
    );
    let operation = Uuid::new_v4();
    let freeze = Uuid::new_v4();
    let commit = Uuid::new_v4();
    let target_incarnation = Uuid::new_v4();
    let review_digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            registration.session_id,
            registration.incarnation,
            &identity,
            &facts
        ))?)
    );
    let receipts = root.join("migration-system-freezes");
    registry::private_directory(&receipts)?;
    let record = receipts.join(format!(
        "{}-{}.json",
        registration.session_id, registration.incarnation
    ));
    // Persist exact phase command identities before invoking any journal effect.
    if record.try_exists()? {
        let saved: serde_json::Value = store::load(&record)?;
        ensure!(
            saved["session_id"] == serde_json::to_value(registration.session_id)?
                && saved["source_incarnation"] == serde_json::to_value(registration.incarnation)?,
            "root-system journal freeze identity changed"
        );
        let commit = Uuid::parse_str(
            saved["commit_command"]
                .as_str()
                .context("retained commit identity missing")?,
        )?;
        ensure!(
            matches!(
                retired_request(
                    &identity,
                    &TransitionRequest {
                        schema: SCHEMA,
                        session_id: registration.session_id,
                        source_incarnation: registration.incarnation,
                        operation: TransitionOperation::Lookup {
                            directory: directory.clone(),
                            command_id: commit
                        }
                    }
                )
                .await?,
                TransitionResponse::Committed { .. }
            ),
            "root-system journal handoff remains uncertain; no source or target command replayed"
        );
        return Ok(());
    }
    store::save(
        &record,
        &serde_json::json!({"operation_id":operation,"freeze_command":freeze,"commit_command":commit,"session_id":registration.session_id,"source_incarnation":registration.incarnation,"review_digest":review_digest,"phase":"source-freeze-admitted"}),
    )?;
    let prepared = match retired_request(
        &identity,
        &TransitionRequest {
            schema: SCHEMA,
            session_id: registration.session_id,
            source_incarnation: registration.incarnation,
            operation: TransitionOperation::SourceFreeze {
                directory: directory.clone(),
                command_id: freeze,
                transition_id: operation,
                target_incarnation,
                expected: facts,
                target_uid: identity.uid,
                target_gid: identity.gid,
                target_config_digest: config_digest,
                review_digest: review_digest.clone(),
            },
        },
    )
    .await?
    {
        TransitionResponse::Prepared { receipt } => receipt,
        _ => anyhow::bail!("root-system source freeze unconfirmed"),
    };
    ensure!(
        matches!(
            retired_request(
                &identity,
                &TransitionRequest {
                    schema: SCHEMA,
                    session_id: registration.session_id,
                    source_incarnation: registration.incarnation,
                    operation: TransitionOperation::TargetCommit {
                        directory,
                        command_id: commit,
                        expected: prepared,
                        target_config_path: config_path
                    }
                }
            )
            .await?,
            TransitionResponse::Committed { .. }
        ),
        "root-system target freeze commit unconfirmed"
    );
    store::save(
        &record,
        &serde_json::json!({"operation_id":operation,"freeze_command":freeze,"commit_command":commit,"session_id":registration.session_id,"source_incarnation":registration.incarnation,"review_digest":review_digest,"phase":"complete"}),
    )?;
    Ok(())
}
