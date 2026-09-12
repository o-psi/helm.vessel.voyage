//! Restart requires positive runtime cleanup evidence; unavailability never grants it.
use super::{registry, routing, service::Supervisor};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{
    fs::OpenOptions,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
use uuid::Uuid;
use voyage_protocol::process::*;

#[derive(Deserialize)]
struct Stopped {
    session_id: Uuid,
    incarnation: Uuid,
    cleanup_observed: bool,
    #[serde(default)]
    suspended: bool,
    #[serde(default)]
    archive: Option<ArchivedVoyage>,
    #[serde(default)]
    deletion: Option<serde_json::Value>,
}

pub fn clean_stop(directory: &Path, registration: &ProcessRegistration) -> bool {
    stopped(directory, registration).is_ok()
}
pub(super) fn startup_failed(directory: &Path, registration: &ProcessRegistration) -> bool {
    clean_stop(directory, registration)
        && super::access::store::load::<serde_json::Value>(&directory.join("stopped.json"))
            .is_ok_and(|value| value["startup_failed"] == true)
}
pub(super) fn suspended(directory: &Path, registration: &ProcessRegistration) -> bool {
    matches!(std::fs::symlink_metadata(directory.join("runtime.sock")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        && !matches!(
            registration.state,
            ProcessState::Relinquished | ProcessState::Stopped
        )
        && stopped(directory, registration)
            .is_ok_and(|s| s.suspended && s.archive.is_none() && s.deletion.is_none())
}

pub(super) fn archived(
    directory: &Path,
    registration: &ProcessRegistration,
) -> Option<ArchivedVoyage> {
    stopped(directory, registration)
        .ok()
        .and_then(|s| s.archive)
}

pub(super) fn deletion(
    directory: &Path,
    registration: &ProcessRegistration,
) -> Option<serde_json::Value> {
    stopped(directory, registration)
        .ok()
        .and_then(|s| s.deletion)
}

fn stopped(directory: &Path, registration: &ProcessRegistration) -> Result<Stopped> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join("stopped.json"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() < 4096,
        "invalid stop evidence"
    );
    let stopped: Stopped = serde_json::from_reader(file)?;
    ensure!(
        stopped.session_id == registration.session_id
            && stopped.incarnation == registration.incarnation
            && stopped.cleanup_observed,
        "stop evidence identity or cleanup mismatch"
    );
    Ok(stopped)
}

// Observers hold this file only while reading a suspended session. Acquire it
// before the global registrations mutex so other sessions keep making progress.
async fn startup_gate(directory: &Path) -> Result<std::fs::File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("startup.lock"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe runtime startup lock"
    );
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(error) => return Err(anyhow::anyhow!("runtime startup owned: {error}")),
        }
    }
}

impl Supervisor {
    pub(super) async fn restart(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
    ) -> Result<serde_json::Value> {
        ensure!(!command_id.is_nil(), "command ID must be nonnil");
        let directory = registry::directory(&self.directory, session_id);
        let startup = startup_gate(&directory).await?;
        let mut registrations = self.registrations.lock().await?;
        let registration = registrations
            .get(&session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown session"))?;
        let command = VesselCommand::Restart {
            command_id,
            session_id,
            incarnation,
        };
        if registry::command_record(&self.directory, command_id, &command, false).await? {
            return Ok(serde_json::to_value(
                routing::inspect(
                    &registry::directory(&self.directory, session_id),
                    registration,
                )
                .await,
            )?);
        }
        if registration.command_id == command_id {
            ensure!(
                registration.restart_from == Some(incarnation),
                "restart command payload conflict"
            );
            return Ok(serde_json::to_value(
                routing::inspect(
                    &registry::directory(&self.directory, session_id),
                    registration,
                )
                .await,
            )?);
        }
        ensure!(
            !registrations
                .values()
                .any(|entry| entry.command_id == command_id),
            "command ID conflict"
        );
        ensure!(
            registration.incarnation == incarnation,
            "stale runtime incarnation"
        );
        ensure!(
            matches!(
                routing::inspect(&directory, registration).await.state,
                ProcessState::Stopped | ProcessState::Suspended
            ) || super::recover_command::restart_permitted(&directory, registration),
            "restart requires positively observed clean runtime stop; unavailable is not stopped"
        );
        let mut next = registration.clone();
        next.executable = Some(self.binary.clone());
        next.incarnation = Uuid::new_v4();
        next.command_id = command_id;
        next.restart_from = Some(incarnation);
        next.token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        next.state = ProcessState::Starting;
        registry::command_record(&self.directory, command_id, &command, true)
            .await
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registry::save(&directory, &next)
            .await
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registrations.insert(session_id, next.clone());
        drop(registrations);
        // A fresh owner takes the same gate itself; never carry this lock into launch.
        drop(startup);
        super::launch::launch(&self.binary, &directory, &next)
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        let observed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let info = routing::inspect(&directory, &next).await;
                if info.state == ProcessState::Live {
                    return info;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!("restart outcome unconfirmed; inspect retained incarnation")
                .context(routing::OutcomeUnknown)
        })?;
        Ok(serde_json::to_value(observed)?)
    }
}
