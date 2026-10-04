//! Native private catalogue discovery: saved registrations are not liveness.
use anyhow::{Result, ensure};
use std::{os::unix::fs::MetadataExt, path::Path};
use voyage_protocol::process::ProcessRegistration;

pub fn registrations(root: &Path) -> Result<Vec<ProcessRegistration>> {
    super::unix_registry::directory(root)?;
    let sessions = root.join("sessions");
    if !sessions.try_exists()? {
        return Ok(Vec::new());
    }
    super::unix_registry::directory(&sessions)?;
    let mut result = Vec::new();
    for entry in std::fs::read_dir(&sessions)? {
        ensure!(result.len() < 4096, "Native catalogue exceeds bound");
        let entry = entry?;
        let name = entry.file_name();
        let id: uuid::Uuid = name
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Non-Unicode native session entry"))?
            .parse()?;
        let meta = std::fs::symlink_metadata(entry.path())?;
        ensure!(
            meta.is_dir()
                && !meta.file_type().is_symlink()
                && meta.uid() == unsafe { libc::geteuid() },
            "Unsafe native catalogue entry"
        );
        let registration = super::unix_registry::load(&entry.path())?;
        ensure!(
            registration.session_id == id,
            "Native catalogue session identity mismatch"
        );
        result.push(registration);
    }
    result.sort_by_key(|entry| entry.session_id);
    Ok(result)
}

/// Recovery observation only. Never spawn or infer a dead runtime survived.
pub async fn observe(
    root: &Path,
    session: uuid::Uuid,
    command: voyage_protocol::process::RuntimeCommand,
) -> Result<voyage_protocol::process::RuntimeResponse> {
    let directory = root.join("sessions").join(session.to_string());
    let registration = super::unix_registry::load(&directory)?;
    ensure!(
        registration.session_id == session,
        "Recovery session identity mismatch"
    );
    ensure!(
        command.observes_saved(),
        "Recovery route accepts observations only; uncertain mutations require exact receipt recovery"
    );
    super::unix_transport::request(&directory, &registration, command).await
}
