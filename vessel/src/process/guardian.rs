//! Independent privileged launch/cleanup owner. Runtime files are never proof.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use uuid::Uuid;
use voyage_protocol::{
    execution_identity::{ConfiguredExecutionIdentity, ObservedExecution},
    process::{ProcessRegistration, ProcessState},
};
use voyage_storage::protected_linux::RootDirectory;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub session: Uuid,
    #[arg(long)]
    pub incarnation: Uuid,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    session_id: Uuid,
    incarnation: Uuid,
    boot_id: Uuid,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    session_id: Uuid,
    incarnation: Uuid,
    boot_id: Uuid,
    cleanup_observed: bool,
    handoff_completed: bool,
    child_exit_code: Option<i32>,
    child_exited_successfully: bool,
    #[serde(default)]
    stop_reason: Option<StopReason>,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StopReason {
    Requested,
    AuthorityChanged,
    HandoffFailed,
}

/// A durable request is not proof of completion. Only the independent guardian
/// may publish completion after observing the entire owned child tree retire.
pub(super) fn request_stop(root: &Path, session: Uuid, incarnation: Uuid) -> Result<()> {
    let record = RootDirectory::open(root)?
        .child("guardians".as_ref())?
        .child(session.to_string().as_ref())?
        .child(incarnation.to_string().as_ref())?;
    let admission: Admission =
        serde_json::from_slice(&record.read("admission.json".as_ref(), 4096)?)?;
    ensure!(
        admission.session_id == session && admission.incarnation == incarnation,
        "guardian admission identity mismatch"
    );
    let bytes = serde_json::to_vec(&admission)?;
    match record.publish_new("stop.json".as_ref(), &bytes, 4096) {
        Ok(()) => Ok(()),
        Err(error) => {
            if record
                .read("stop.json".as_ref(), 4096)
                .is_ok_and(|saved| saved == bytes)
            {
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}

fn stop_requested(
    record: &RootDirectory,
    session: Uuid,
    incarnation: Uuid,
    boot_id: Uuid,
) -> Result<bool> {
    let bytes = match record.read("stop.json".as_ref(), 4096) {
        Ok(bytes) => bytes,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let stop: Admission = serde_json::from_slice(&bytes)?;
    ensure!(
        stop.session_id == session && stop.incarnation == incarnation && stop.boot_id == boot_id,
        "guardian stop request identity mismatch"
    );
    Ok(true)
}

fn continuing_authority(
    root: &Path,
    registration: &ProcessRegistration,
    identity: &ConfiguredExecutionIdentity,
) -> Result<()> {
    let latest = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(super::database::bound_observer_identity(root, registration))?;
    ensure!(&latest == identity, "execution authority changed");
    super::launch::validate_identity(&latest)
}

fn wait_owned(
    child: &mut std::process::Child,
    root: &Path,
    record: &RootDirectory,
    registration: &ProcessRegistration,
    identity: &ConfiguredExecutionIdentity,
    boot_id: Uuid,
    stop_reason: &mut Option<StopReason>,
) -> Result<std::process::ExitStatus> {
    let mut authority_at = Instant::now();
    let mut kill_at = None;
    let mut stop_deadline = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if stop_reason.is_none() {
            // Corrupt or unreadable protected stop state also fails closed.
            if stop_requested(
                record,
                registration.session_id,
                registration.incarnation,
                boot_id,
            )
            .unwrap_or(true)
            {
                *stop_reason = Some(StopReason::Requested);
            }
        }
        if !matches!(
            stop_reason,
            Some(StopReason::HandoffFailed | StopReason::AuthorityChanged)
        ) && Instant::now() >= authority_at
        {
            if continuing_authority(root, registration, identity).is_err() {
                *stop_reason = Some(StopReason::AuthorityChanged);
                kill_at = Some(Instant::now());
            }
            authority_at = Instant::now() + Duration::from_secs(1);
        }
        if stop_reason.is_some() && kill_at.is_none() {
            kill_at = Some(
                Instant::now()
                    + if matches!(stop_reason, Some(StopReason::Requested)) {
                        Duration::from_secs(10)
                    } else {
                        Duration::ZERO
                    },
            );
        }
        if kill_at.is_some_and(|at| Instant::now() >= at) && stop_deadline.is_none() {
            // This Child is unreaped and cannot be replaced by PID reuse.
            let _ = child.kill();
            stop_deadline = Some(Instant::now() + Duration::from_secs(10));
        }
        ensure!(
            stop_deadline.is_none_or(|deadline| Instant::now() < deadline),
            "owned runtime stop remains unconfirmed"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn boot() -> Result<Uuid> {
    Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .parse()?)
}
fn session_control(root: &Path, session: Uuid) -> Result<RootDirectory> {
    RootDirectory::open(root)?
        .create_child("guardians".as_ref())?
        .create_child(session.to_string().as_ref())
}

/// Local child retirement only. A missing/unfinished same-boot record cannot be
/// converted into a restart permission by any runtime-written stopped marker.
/// Bounded polling must not wait on the live guardian's lifetime ownership lock.
/// Only immutable protected retirement records may pass this preliminary gate;
/// the normal fenced proof still establishes the final result.
pub(super) fn cleanup_available(root: &Path, session: Uuid, incarnation: Uuid) -> Result<bool> {
    let record = RootDirectory::open(root)?
        .child("guardians".as_ref())?
        .child(session.to_string().as_ref())?
        .child(incarnation.to_string().as_ref())?;
    let admission: Admission =
        serde_json::from_slice(&record.read("admission.json".as_ref(), 4096)?)?;
    ensure!(
        admission.session_id == session && admission.incarnation == incarnation,
        "guardian admission identity mismatch"
    );
    if admission.boot_id == boot()? {
        match record.read("completion.json".as_ref(), 4096) {
            Ok(_) => (),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        }
    }
    cleanup_observed(root, session, incarnation)
}

pub(super) fn cleanup_observed(root: &Path, session: Uuid, incarnation: Uuid) -> Result<bool> {
    let control = RootDirectory::open(root)?
        .child("guardians".as_ref())?
        .child(session.to_string().as_ref())?;
    let _fence = control.lock("owner.lock".as_ref())?;
    let record = control.child(incarnation.to_string().as_ref())?;
    let admission: Admission =
        serde_json::from_slice(&record.read("admission.json".as_ref(), 4096)?)?;
    ensure!(
        admission.session_id == session && admission.incarnation == incarnation,
        "guardian admission identity mismatch"
    );
    if admission.boot_id != boot()? {
        return Ok(true);
    }
    let complete: Completion =
        serde_json::from_slice(&record.read("completion.json".as_ref(), 4096)?)?;
    ensure!(
        complete.session_id == session
            && complete.incarnation == incarnation
            && complete.boot_id == admission.boot_id,
        "guardian completion identity mismatch"
    );
    Ok(complete.cleanup_observed)
}

/// Necessary protected retirement proof for automatic ordinary wake. This is
/// not sufficient alone: an original-UID fenced observer must also positively
/// attest the runtime's suspended disposition. Old boots, Stop, failed handoff
/// and authority loss never become automatic execution admission.
pub(super) fn suspension_candidate(root: &Path, session: Uuid, incarnation: Uuid) -> Result<bool> {
    let control = RootDirectory::open(root)?
        .child("guardians".as_ref())?
        .child(session.to_string().as_ref())?;
    let _fence = control.lock("owner.lock".as_ref())?;
    let record = control.child(incarnation.to_string().as_ref())?;
    let admission: Admission =
        serde_json::from_slice(&record.read("admission.json".as_ref(), 4096)?)?;
    let complete: Completion =
        serde_json::from_slice(&record.read("completion.json".as_ref(), 4096)?)?;
    let stop_absent = matches!(record.read("stop.json".as_ref(), 4096),
        Err(error) if error.downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound));
    Ok(suspension_facts(
        &admission,
        &complete,
        session,
        incarnation,
        boot()?,
        stop_absent,
    ))
}

fn suspension_facts(
    admission: &Admission,
    complete: &Completion,
    session: Uuid,
    incarnation: Uuid,
    current_boot: Uuid,
    stop_absent: bool,
) -> bool {
    admission.session_id == session
        && admission.incarnation == incarnation
        && admission.boot_id == current_boot
        && complete.session_id == session
        && complete.incarnation == incarnation
        && complete.boot_id == current_boot
        && complete.cleanup_observed
        && complete.handoff_completed
        && complete.child_exited_successfully
        && complete.child_exit_code == Some(0)
        && complete.stop_reason.is_none()
        && stop_absent
}

fn current(
    root: &Path,
    session: Uuid,
    incarnation: Uuid,
) -> Result<(ProcessRegistration, ConfiguredExecutionIdentity)> {
    // Drop the temporary database runtime before launch. The guardian owns
    // its synchronous kernel child tree; a separately retained metadata-only
    // scope listener never creates children or runs an agent executor.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let registration = super::database::registration(root, session).await?;
            ensure!(
                registration.incarnation == incarnation
                    && registration.state == ProcessState::Starting
                    && registration.token.len() >= 32
                    && registration.workspace.is_absolute()
                    && registration
                        .config_path
                        .as_ref()
                        .is_none_or(|path| path.is_absolute()),
                "invalid protected launch admission"
            );
            let identity = super::database::bound_observer_identity(root, &registration).await?;
            super::launch::validate_identity(&identity)?;
            super::admin_execution::verify_running(root, &registration, &identity).await?;
            Ok((registration, identity))
        })
}

/// Internal root-only entry point. No client identity-selection capability or
/// system installation is enabled by the presence of this guardian.
pub fn run(args: Args) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "privileged guardian requires root"
    );
    ensure!(
        !args.session.is_nil() && !args.incarnation.is_nil(),
        "invalid guardian identity"
    );
    RootDirectory::open(&args.directory)?;
    let (registration, identity) = current(&args.directory, args.session, args.incarnation)?;
    let control = session_control(&args.directory, args.session)?;
    if let Some(previous) = registration.restart_from {
        ensure!(
            previous != args.incarnation
                && cleanup_observed(&args.directory, args.session, previous)?,
            "previous local cleanup is unconfirmed"
        );
    }
    let _fence = control.lock("owner.lock".as_ref())?;
    // Recheck after acquiring the session-wide exclusive launch/cleanup fence.
    let (current_registration, current_identity) =
        current(&args.directory, args.session, args.incarnation)?;
    ensure!(
        serde_json::to_vec(&registration)? == serde_json::to_vec(&current_registration)?
            && identity == current_identity,
        "launch admission changed"
    );
    let binary = registration
        .executable
        .as_ref()
        .context("protected runtime executable missing")?;
    super::launch::protected_binary(binary)?;
    let directory = super::runtime_storage::provision_bound_directory(
        &args.directory,
        args.session,
        identity.uid,
        identity.gid,
    )?;
    ensure!(
        directory.join("runtime.sock").as_os_str().len() < 108,
        "runtime socket path exceeds limit"
    );
    let record = control.create_child(args.incarnation.to_string().as_ref())?;
    let boot_id = boot()?;
    let mut command = Command::new(binary);
    command
        .arg("serve-bound")
        .arg("--directory")
        .arg(&directory)
        .arg("--session")
        .arg(args.session.to_string())
        .arg("--incarnation")
        .arg(args.incarnation.to_string())
        .arg("--workspace")
        .arg(&registration.workspace)
        .current_dir(&identity.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(config) = &registration.config_path {
        command.arg("--config").arg(config);
    }
    super::launch::configure_identity(&mut command, &identity)?;
    if let Some((data, config)) =
        super::admin_execution::runtime_namespace(&args.directory, &registration, &identity)?
    {
        command
            .env("XDG_DATA_HOME", data)
            .env("XDG_CONFIG_HOME", config);
    }
    let launch_digest = super::identity_start::launch_digest(&args.directory, &registration)?;
    let retained_digest = if identity.authority
        == voyage_protocol::execution_identity::AuthorityClass::Administrator
    {
        launch_digest.clone()
    } else {
        super::execution_transition::retained_digest(&args.directory, &registration)?
    };
    if let Some(digest) = retained_digest {
        command.env("VOYAGE_BOUND_RETAINED_CONFIG_DIGEST", digest);
    }
    if let Some(digest) = launch_digest {
        command.env("VOYAGE_BOUND_CONFIG_DIGEST", digest);
    }
    ensure!(
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } == 0,
        "protected child guardian unavailable"
    );
    // Keep current scope reauthorization with the independent execution
    // guardian, so replacing the Root service cannot cancel a healthy voyage.
    // This listener exposes protected current metadata only: no cached rights,
    // credential material or agent loop. It is ready before any child effect.
    let scope_service = super::scope_authority::start_guardian(&args.directory, &registration)?;
    record.publish_new(
        "admission.json".as_ref(),
        &serde_json::to_vec(&Admission {
            session_id: args.session,
            incarnation: args.incarnation,
            boot_id,
        })?,
        4096,
    )?;
    let mut child_exit_code = None;
    let mut child_exited_successfully = false;
    let mut stop_reason = None;
    let spawned = command.spawn();
    let handoff = if let Ok(mut child) = spawned {
        let result = (|| -> Result<()> {
            let observed = super::guardian_observation::observe(
                child.id(),
                args.incarnation,
                &identity,
                binary,
            )?;
            record.publish_new(
                "observed.json".as_ref(),
                &serde_json::to_vec::<ObservedExecution>(&observed)?,
                8192,
            )?;
            let (latest, latest_identity) =
                current(&args.directory, args.session, args.incarnation)?;
            ensure!(
                serde_json::to_vec(&latest)? == serde_json::to_vec(&registration)?
                    && latest_identity == identity,
                "launch authority changed before handoff"
            );
            send_registration(
                child.stdin.take().context("launch pipe unavailable")?,
                &registration,
            )
        })();
        if result.is_err() {
            stop_reason = Some(StopReason::HandoffFailed);
        }
        // Even failed handoffs retain ownership until all descendants are reaped.
        let waited = wait_owned(
            &mut child,
            &args.directory,
            &record,
            &registration,
            &identity,
            boot_id,
            &mut stop_reason,
        );
        if let Ok(status) = &waited {
            child_exit_code = status.code();
            child_exited_successfully = status.success();
        }
        result.and_then(|()| {
            waited?;
            Ok(())
        })
    } else {
        Err(spawned.err().unwrap().into())
    };
    voyage_storage::descendant_cleanup::drain(Duration::from_secs(10))?;
    record.publish_new(
        "completion.json".as_ref(),
        &serde_json::to_vec(&Completion {
            session_id: args.session,
            incarnation: args.incarnation,
            boot_id,
            cleanup_observed: true,
            handoff_completed: handoff.is_ok(),
            child_exit_code,
            child_exited_successfully,
            stop_reason,
        })?,
        4096,
    )?;
    // The scoped listener retires only after child exit, descendant drain and
    // the protected completion publication; readiness never means cleanup.
    drop(scope_service);
    handoff
}

fn send_registration(
    mut pipe: std::process::ChildStdin,
    registration: &ProcessRegistration,
) -> Result<()> {
    let bytes = serde_json::to_vec(registration)?;
    ensure!(bytes.len() <= 16384, "launch registration exceeds limit");
    let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
    frame.extend(bytes);
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    ensure!(
        flags >= 0
            && unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                == 0,
        "launch pipe cannot be bounded"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut remaining = frame.as_slice();
    while !remaining.is_empty() {
        ensure!(Instant::now() < deadline, "launch handoff deadline elapsed");
        match pipe.write(remaining) {
            Ok(0) => anyhow::bail!("launch pipe closed"),
            Ok(count) => remaining = &remaining[count..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "guardian_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "guardian_suspension_tests.rs"]
mod suspension_tests;
