//! Current host-private grants are checked at admission and every execution dispatch.
use super::{LocalActor, State};
use anyhow::{Result, ensure};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use voyage_protocol::process::{
    GrantBinding, ProcessGrant, ProcessRight, RuntimeRequest, required_process_right,
};

#[derive(Clone)]
pub(super) struct Authorization {
    pub authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    pub actor: LocalActor,
    pub grant: Option<GrantBinding>,
}

#[derive(Debug)]
struct GrantAuthority {
    path: PathBuf,
    binding: GrantBinding,
    session: uuid::Uuid,
}
impl crate::policy::ExecutionAuthority for GrantAuthority {
    fn check(&self) -> Result<()> {
        let grant = read_current(&self.path, &self.binding, self.session)?;
        ensure!(
            grant.rights.contains(&ProcessRight::Execute),
            "execution grant withdrawn"
        );
        Ok(())
    }
}

pub(super) fn authorize(
    state: &State,
    request: &RuntimeRequest,
    directory: &Path,
) -> Result<Authorization> {
    let Some(binding) = &request.authorization else {
        return Ok(Authorization {
            authority: None,
            actor: state.actor,
            grant: None,
        });
    };
    ensure!(
        directory.file_name().and_then(|v| v.to_str())
            == Some(state.registration.session_id.to_string().as_str()),
        "grant runtime directory mismatch"
    );
    let sessions = directory
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing supervisor directory"))?;
    ensure!(
        sessions.file_name().and_then(|v| v.to_str()) == Some("sessions"),
        "grant runtime not supervised"
    );
    let root = sessions
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing supervisor directory"))?;
    let path = root
        .join("access/grants")
        .join(format!("{}.json", binding.grant_id));
    let grant = read_current(&path, binding, state.registration.session_id)?;
    ensure!(
        grant.workspace == state.registration.workspace,
        "session grant workspace mismatch"
    );
    let right = required_process_right(&request.command)
        .ok_or_else(|| anyhow::anyhow!("operation unavailable to scoped clients"))?;
    ensure!(grant.rights.contains(&right), "session permission denied");
    if matches!(
        request.command,
        voyage_protocol::process::RuntimeCommand::Terminal { .. }
    ) {
        ensure!(
            grant.rights.contains(&ProcessRight::Execute),
            "private terminal requires execution grant"
        );
    }
    if matches!(
        request.command,
        voyage_protocol::process::RuntimeCommand::AssignmentObserve { cancel: true, .. }
    ) {
        ensure!(
            grant.rights.contains(&ProcessRight::Cancel),
            "participant cancellation requires cancel grant"
        );
    }
    let mut actor = state.actor;
    actor.principal_id = grant.principal_id;
    Ok(Authorization {
        authority: Some(Arc::new(GrantAuthority {
            path,
            binding: binding.clone(),
            session: state.registration.session_id,
        })),
        actor,
        grant: Some(binding.clone()),
    })
}

#[cfg(unix)]
fn read_current(path: &Path, binding: &GrantBinding, session: uuid::Uuid) -> Result<ProcessGrant> {
    use std::os::unix::fs::MetadataExt;
    let grant: ProcessGrant = load_private(path)?;
    let now: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        .try_into()?;
    ensure!(
        grant.grant_id == binding.grant_id
            && grant.principal_id == binding.principal_id
            && grant.revision == binding.revision
            && grant.session_id == session
            && !grant.revoked
            && grant.expires_at_ms > now,
        "session authority revoked, stale or expired"
    );
    if let Some(identity) = &grant.enrollment {
        let metadata = std::fs::symlink_metadata(&identity.database_path)?;
        ensure!(
            identity.database_path.is_absolute()
                && metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "unsafe enrollment authority database"
        );
        let db = rusqlite::Connection::open_with_flags(
            &identity.database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(std::time::Duration::from_millis(100))?;
        let (epoch, revoked): (i64, bool) = db.query_row(
            "SELECT epoch,revoked FROM machines WHERE id=?1",
            [identity.machine_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        ensure!(
            !revoked && u64::try_from(epoch)? == identity.epoch,
            "enrolled machine revoked or replaced"
        );
    }
    if let Some(parent) = &grant.parent_grant {
        let root = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or_else(|| anyhow::anyhow!("missing grant authority root"))?;
        let parent_path = root
            .join("access/grants")
            .join(format!("{}.json", parent.grant_id));
        let metadata: ProcessGrant = load_private(&parent_path)?;
        ensure!(
            metadata.parent_grant.is_none() && metadata.principal_id == grant.principal_id,
            "invalid nested participant grant"
        );
        let original = read_current(&parent_path, parent, metadata.session_id)?;
        ensure!(
            grant
                .rights
                .iter()
                .all(|right| original.rights.contains(right)),
            "participant rights exceed parent grant"
        );
        let accepted = grant
            .participant_binding
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("participant grant missing accepted binding"))?;
        let binding: voyage_protocol::process::ParticipantBinding = load_private(
            &root
                .join("participants/bindings")
                .join(format!("{}.json", accepted.binding_id)),
        )?;
        ensure!(
            binding.binding_id == accepted.binding_id
                && binding.revision >= accepted.revision
                && !binding.cancel_existing
                && binding.expires_at_ms > now
                && binding.parent_session_id == original.session_id
                && binding.principal_id == grant.principal_id,
            "participant binding expired or cancelled"
        );
    }
    Ok(grant)
}

#[cfg(unix)]
fn load_private<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.nlink() == 1
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.len() <= 16384,
        "unsafe runtime grant"
    );
    Ok(serde_json::from_reader(file)?)
}

#[cfg(not(unix))]
fn read_current(_: &Path, _: &GrantBinding, _: uuid::Uuid) -> Result<ProcessGrant> {
    anyhow::bail!("scoped runtime authority unsupported on this platform")
}
