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

fn current(
    root: &Path,
    session: Uuid,
    incarnation: Uuid,
) -> Result<(ProcessRegistration, ConfiguredExecutionIdentity)> {
    // Drop this temporary runtime and its worker threads before becoming a
    // subreaper. The guardian itself owns one synchronous kernel child tree.
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
    ensure!(
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } == 0,
        "protected child guardian unavailable"
    );
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
            let _ = child.kill();
        }
        // Even failed handoffs retain ownership until all descendants are reaped.
        let waited = child.wait();
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
        })?,
        4096,
    )?;
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
