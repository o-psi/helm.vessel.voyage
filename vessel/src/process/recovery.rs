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
    archive: Option<ArchivedVoyage>,
}

pub fn clean_stop(directory: &Path, registration: &ProcessRegistration) -> bool {
    stopped(directory, registration).is_ok()
}

pub(super) fn archived(
    directory: &Path,
    registration: &ProcessRegistration,
) -> Option<ArchivedVoyage> {
    stopped(directory, registration)
        .ok()
        .and_then(|s| s.archive)
}

fn stopped(directory: &Path, registration: &ProcessRegistration) -> Result<Stopped> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join("stopped.json"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
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

impl Supervisor {
    pub(super) async fn restart(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
    ) -> Result<serde_json::Value> {
        ensure!(!command_id.is_nil(), "command ID must be nonnil");
        let mut registrations = self.registrations.lock().await;
        let registration = registrations
            .get(&session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown session"))?;
        let command = VesselCommand::Restart {
            command_id,
            session_id,
            incarnation,
        };
        if registry::command_record(&self.directory, command_id, &command, false)? {
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
        let directory = registry::directory(&self.directory, session_id);
        ensure!(
            routing::inspect(&directory, registration).await.state == ProcessState::Stopped
                || super::recover_command::restart_permitted(&directory, registration),
            "restart requires positively observed clean runtime stop; unavailable is not stopped"
        );
        let occupied = registrations
            .values()
            .filter(|entry| {
                let path = registry::directory(&self.directory, entry.session_id);
                entry.session_id != session_id
                    && entry.state != ProcessState::Relinquished
                    && (path.join("runtime.sock").exists()
                        || (!clean_stop(&path, entry)
                            && !super::recover_command::restart_permitted(&path, entry)))
            })
            .count();
        ensure!(
            occupied < self.capacity,
            "Vessel process capacity exhausted"
        );
        let mut next = registration.clone();
        next.incarnation = Uuid::new_v4();
        next.command_id = command_id;
        next.restart_from = Some(incarnation);
        next.token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        next.state = ProcessState::Starting;
        registry::command_record(&self.directory, command_id, &command, true)
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registry::save(&directory, &next)
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registrations.insert(session_id, next.clone());
        drop(registrations);
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
