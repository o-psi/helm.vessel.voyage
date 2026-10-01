//! Ordinary legacy activation is observation-only until the updater commits.
use anyhow::{Result, ensure};
use std::{
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
use uuid::Uuid;
use voyage_protocol::vessel::VesselCommand;
const FILE: &str = "update-quarantine.json";
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Fence {
    schema_version: u32,
    operation_id: Uuid,
    previous_release: String,
    candidate_release: String,
}
pub(super) fn active(root: &Path) -> Result<bool> {
    let path = root.join(FILE);
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= 4096,
        "Ordinary update quarantine unavailable"
    );
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    let fence: Fence = serde_json::from_slice(&bytes)?;
    ensure!(
        fence.schema_version == 1
            && !fence.operation_id.is_nil()
            && [fence.previous_release, fence.candidate_release]
                .iter()
                .all(|id| id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit())),
        "Invalid ordinary update quarantine"
    );
    Ok(true)
}
pub(super) fn check(root: &Path, command: &VesselCommand) -> Result<()> {
    if !active(root)? {
        return Ok(());
    }
    let mut command = command;
    for _ in 0..4 {
        if let VesselCommand::Granted { command: inner, .. } = command {
            command = inner;
        } else {
            break;
        }
    }
    ensure!(
        matches!(
            command,
            VesselCommand::Capabilities
                | VesselCommand::Identity
                | VesselCommand::Catalogue
                | VesselCommand::CatalogueChanges { .. }
                | VesselCommand::UpdateStatus { .. }
        ),
        "Legacy update activation is quarantined; no voyage, enrollment or mutation was dispatched"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::test_support::Fixture;
    fn fence(f: &Fixture) {
        crate::process::access::store::save(&f.0.join(FILE),&serde_json::json!({"schema_version":1,"operation_id":Uuid::new_v4(),"previous_release":"a".repeat(64),"candidate_release":"b".repeat(64)})).unwrap();
    }
    #[test]
    fn activation_quarantine_is_readiness_only_and_never_admits_account_or_voyage_effects() {
        let f = Fixture::new();
        fence(&f);
        for command in [
            VesselCommand::Capabilities,
            VesselCommand::Identity,
            VesselCommand::Catalogue,
        ] {
            check(&f.0, &command).unwrap();
        }
        for command in [
            VesselCommand::Start {
                command_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                workspace: f.0.clone(),
            },
            VesselCommand::Accounts {
                workspace: f.0.clone(),
                transport: None,
            },
            VesselCommand::Restart {
                command_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                incarnation: Uuid::new_v4(),
            },
        ] {
            assert!(check(&f.0, &command).is_err());
        }
    }
    #[test]
    fn malformed_quarantine_fails_closed_without_admission() {
        let f = Fixture::new();
        crate::process::access::store::save(&f.0.join(FILE),&serde_json::json!({"schema_version":1,"operation_id":Uuid::nil(),"previous_release":"a".repeat(64),"candidate_release":"b".repeat(64)})).unwrap();
        assert!(check(&f.0, &VesselCommand::Capabilities).is_err());
    }
}
