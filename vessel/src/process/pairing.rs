//! Account-owner invitations and secret-only redemption, outside command receipts.
//! All writers (including local CLI) serialize on the same private OS file lock.
mod audit;
use super::{access::store, registry};
use anyhow::{Result, ensure};
use audit::Kind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use subtle::ConstantTimeEq;
use uuid::Uuid;
use voyage_protocol::process::{
    ApprovedWorkspace, ConnectionGrant, ProcessRight, WorkspaceCredential,
};

const LIMIT: usize = 4096;
const STATE_BYTES: usize = 16 * 1024 * 1024;
pub const GRANT_TTL_MS: u64 = 30 * 86400 * 1000;
pub const MAX_INVITATION_TTL_SECONDS: u64 = 900;

/// Input-independent refusal; storage failures retain uncertain outcome semantics.
#[derive(Debug)]
pub enum PairRefusal {
    Denied,
    RateLimited,
    IdentityChanged,
    Capacity,
}
impl std::fmt::Display for PairRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Denied => "pairing denied",
            Self::RateLimited => "pairing rate limit reached",
            Self::IdentityChanged => "Vessel identity changed",
            Self::Capacity => "connection capacity exhausted",
        })
    }
}
impl std::error::Error for PairRefusal {}

// Intentionally no Debug: this is a private input, never a VesselCommand.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairRequest {
    pub protocol: u32,
    pub command_id: Uuid,
    pub principal_id: Uuid,
    pub invitation_id: Uuid,
    pub code: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub protocol: u32,
    pub invitation_id: Uuid,
    pub principal_id: Uuid,
    pub vessel_id: Uuid,
    pub endpoint: String,
    pub expires_at_ms: u64,
    pub code: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Redemption {
    command_id: Uuid,
    credential: WorkspaceCredential,
    grant: ConnectionGrant,
    #[serde(default)]
    publication_observed: bool,
    #[serde(default)]
    audit_tracked: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    invitation_id: Uuid,
    principal_id: Uuid,
    vessel_id: Uuid,
    endpoint: String,
    expires_at_ms: u64,
    code_hash: String,
    attempts: u32,
    workspaces: Vec<ApprovedWorkspace>,
    rights: Vec<ProcessRight>,
    #[serde(default)]
    accounts: Vec<Uuid>,
    #[serde(default)]
    enrollment_connections: Vec<Uuid>,
    redemption: Option<Redemption>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Revocation {
    command_id: Uuid,
    grant_id: Uuid,
    expected_revision: u64,
    #[serde(default)]
    publication_observed: bool,
    #[serde(default)]
    audit_tracked: bool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    #[serde(default)]
    audit: Option<audit::Journal>,
    invitations: Vec<Pending>,
    revocations: Vec<Revocation>,
    window_ms: u64,
    attempts: u32,
}
fn directory(root: &Path) -> PathBuf {
    root.join("access").join("pairing")
}
fn state_path(root: &Path) -> PathBuf {
    directory(root).join("state.json")
}
pub fn connection_path(root: &Path, id: Uuid) -> PathBuf {
    root.join("access")
        .join("connections")
        .join(format!("{id}.json"))
}
fn lock(root: &Path) -> Result<File> {
    registry::private_directory(root)?;
    registry::private_directory(&root.join("access"))?;
    registry::private_directory(&directory(root))?;
    registry::private_directory(&root.join("access").join("connections"))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory(root).join("lock"))?;
    let m = file.metadata()?;
    ensure!(
        m.is_file()
            && m.nlink() == 1
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0,
        "unsafe pairing lock"
    );
    file.try_lock()
        .map_err(|_| anyhow::anyhow!("pairing busy; retry the same request"))?;
    Ok(file)
}
fn load(root: &Path) -> Result<State> {
    if !state_path(root).try_exists()? {
        return Ok(State::default());
    }
    let bytes = store::read_bounded(
        &state_path(root),
        (STATE_BYTES + voyage_storage::credentials::OVERHEAD) as u64,
    )?;
    let bytes = voyage_storage::credentials::open(b"vessel-workspace-pairing-state-v1", &bytes)?;
    ensure!(bytes.len() <= STATE_BYTES, "pairing state exceeds limit");
    let state: State = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid private pairing state"))?;
    ensure!(
        state.invitations.len() <= LIMIT && state.revocations.len() <= LIMIT,
        "pairing state capacity exceeded"
    );
    if let Some(audit) = &state.audit {
        audit.validate()?;
    }
    Ok(state)
}
fn save(root: &Path, state: &State) -> Result<()> {
    let bytes = serde_json::to_vec(state)?;
    ensure!(bytes.len() <= STATE_BYTES, "pairing state exceeds limit");
    let sealed = voyage_storage::credentials::seal(b"vessel-workspace-pairing-state-v1", &bytes)?;
    store::save_bytes(
        &state_path(root),
        &sealed,
        STATE_BYTES + voyage_storage::credentials::OVERHEAD,
    )
}
fn start_audit(root: &Path, state: &mut State, now: u64) -> Result<()> {
    if state.audit.is_none() {
        let vessel = identity(root)?;
        let mut journal = audit::Journal::new(vessel, state_path(root).try_exists()?);
        journal.append(audit::event(
            Kind::AuditStarted,
            now,
            vessel,
            None,
            None,
            None,
        ))?;
        state.audit = Some(journal);
    }
    Ok(())
}
fn append(state: &mut State, event: audit::Event) -> Result<()> {
    state
        .audit
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("audit not initialized"))?
        .append(event)
}
fn save_intent(root: &Path, state: &State) -> Result<()> {
    // Reserve space for the bounded publication-observation event before effects.
    ensure!(
        serde_json::to_vec(state)?.len() <= STATE_BYTES - 8192,
        "pairing audit capacity exhausted"
    );
    save(root, state)
}
fn same_grant(a: &ConnectionGrant, b: &ConnectionGrant) -> Result<bool> {
    Ok(serde_json::to_vec(a)? == serde_json::to_vec(b)?)
}
fn observe_redemption(root: &Path, state: &mut State, position: usize, now: u64) -> Result<()> {
    let p = &state.invitations[position];
    let r = p
        .redemption
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("redemption unavailable"))?;
    if r.publication_observed {
        return Ok(());
    }
    let current: ConnectionGrant = store::load(&connection_path(root, r.grant.grant_id))?;
    ensure!(
        same_grant(&current, &r.grant)?,
        "grant publication changed; original intent retained"
    );
    let event = audit::event(
        Kind::GrantPublicationObserved,
        now,
        current.vessel_id,
        Some(r.command_id),
        Some(p.invitation_id),
        Some(&current),
    );
    append(state, event)?;
    state.invitations[position]
        .redemption
        .as_mut()
        .expect("checked redemption")
        .publication_observed = true;
    #[cfg(test)]
    tests::failpoint("audit_finalize")?;
    save(root, state)
}
fn identity(root: &Path) -> Result<Uuid> {
    // Do not create a competing signing identity from an administrative CLI.
    #[derive(Deserialize)]
    struct Identity {
        vessel_id: Uuid,
    }
    let identity: Identity = store::load(&root.join("identity").join("key.json"))?;
    ensure!(!identity.vessel_id.is_nil(), "Vessel identity unavailable");
    Ok(identity.vessel_id)
}
pub fn preflight(root: &Path) -> Result<Value> {
    Ok(json!({"protocol":1,"vessel_id":identity(root)?,
        "features":["workspace_pairing","sse_events"]}))
}
fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Deliberately separate from ConnectionGrant: secret-bearing fields are never
/// part of the inventory output, including if the grant format grows later.
#[derive(Deserialize, Serialize)]
struct ConnectionSummary {
    schema_version: u32,
    grant_id: Uuid,
    principal_id: Uuid,
    vessel_id: Uuid,
    revision: u64,
    rights: Vec<ProcessRight>,
    expires_at_ms: u64,
    revoked: bool,
    workspaces: Vec<ApprovedWorkspace>,
}

/// Verify an existing directory without creating it or repairing permissions.
fn inventory_directory(path: &Path) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => anyhow::bail!("connection inventory directory unavailable"),
    };
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe connection inventory directory"
    );
    Ok(true)
}

fn inventory_record(path: &Path) -> Result<ConnectionSummary> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= 16_384,
        "invalid private connection record"
    );
    let mut bytes = Vec::new();
    file.take(16_385).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16_384, "connection record exceeds limit");
    Ok(serde_json::from_slice(&bytes)?)
}

/// Local owner-only observations, serialized with invitation/redemption/revocation
/// writers. Never opens the secret-bearing pairing journal or creates a lock.
pub fn inventory(root: &Path) -> Result<Value> {
    ensure!(
        root.is_absolute() && std::fs::canonicalize(root)? == root,
        "inventory requires a canonical absolute state directory"
    );
    ensure!(
        inventory_directory(root)?,
        "Vessel state directory unavailable"
    );
    let empty = || json!({"schema_version":1,"connections":[]});
    if !inventory_directory(&root.join("access"))? {
        return Ok(empty());
    }
    let connections = root.join("access/connections");
    if !inventory_directory(&connections)? {
        return Ok(empty());
    }
    ensure!(
        inventory_directory(&directory(root))?,
        "connection inventory lock unavailable"
    );
    let lock = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(directory(root).join("lock"))
        .map_err(|_| anyhow::anyhow!("connection inventory lock unavailable"))?;
    let metadata = lock.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe connection inventory lock"
    );
    lock.try_lock_shared()
        .map_err(|_| anyhow::anyhow!("pairing busy; retry connection inventory"))?;
    let mut summaries = Vec::new();
    let entries = std::fs::read_dir(&connections)?;
    // Bound all entries, including interrupted atomic-write files.
    for (index, entry) in entries.enumerate() {
        ensure!(
            index < LIMIT * 2,
            "connection inventory entry limit exceeded"
        );
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("invalid connection inventory filename"))?;
        if name
            .strip_prefix(".pending-")
            .is_some_and(|id| Uuid::parse_str(id).is_ok())
        {
            continue;
        }
        let id = name
            .strip_suffix(".json")
            .and_then(|id| Uuid::parse_str(id).ok())
            .filter(|id| name == format!("{id}.json"))
            .ok_or_else(|| anyhow::anyhow!("invalid connection inventory filename"))?;
        ensure!(
            summaries.len() < LIMIT,
            "connection inventory capacity exceeded"
        );
        // Check type before opening: a FIFO must not block an operator command.
        ensure!(
            entry.file_type()?.is_file(),
            "invalid private connection record"
        );
        let summary = inventory_record(&entry.path())
            .map_err(|_| anyhow::anyhow!("invalid private connection record"))?;
        ensure!(
            summary.schema_version == 1
                && summary.grant_id == id
                && !id.is_nil()
                && !summary.principal_id.is_nil()
                && !summary.vessel_id.is_nil()
                && summary.revision > 0
                && summary.expires_at_ms > 0
                && !summary.rights.is_empty()
                && summary.rights.len() <= 16
                && summary
                    .rights
                    .iter()
                    .enumerate()
                    .all(|(i, right)| !summary.rights[..i].contains(right))
                && !summary.workspaces.is_empty()
                && summary.workspaces.len() <= 32
                && summary.workspaces.iter().enumerate().all(|(i, workspace)| {
                    !workspace.id.is_nil()
                        && workspace.path.is_absolute()
                        && !workspace.name.is_empty()
                        && workspace.name.len() <= 256
                        && !workspace.name.chars().any(char::is_control)
                        && summary.workspaces[..i]
                            .iter()
                            .all(|prior| prior.id != workspace.id && prior.path != workspace.path)
                }),
            "invalid connection inventory metadata"
        );
        summaries.push(summary);
    }
    summaries.sort_by_key(|summary| summary.grant_id);
    Ok(json!({"schema_version":1,"connections":summaries}))
}

/// Local account-owner only. The returned invitation must be written privately.
pub fn invite(
    root: &Path,
    endpoint: &str,
    principal_id: Uuid,
    workspaces: Vec<ApprovedWorkspace>,
    rights: Vec<ProcessRight>,
    accounts: Vec<Uuid>,
    enrollment_connections: Vec<Uuid>,
    ttl_seconds: u64,
) -> Result<Invitation> {
    let _lock = lock(root)?;
    let mut state = load(root)?;
    let now = store::now()?;
    start_audit(root, &mut state, now)?;
    ensure!(!principal_id.is_nil(), "principal must not be nil");
    ensure!(
        (1..=MAX_INVITATION_TTL_SECONDS).contains(&ttl_seconds),
        "invitation lifetime must be within 15 minutes"
    );
    ensure!(
        !workspaces.is_empty() && workspaces.len() <= 32,
        "approve between 1 and 32 workspaces"
    );
    for (i, workspace) in workspaces.iter().enumerate() {
        ensure!(
            !workspace.id.is_nil()
                && workspace.path.is_dir()
                && std::fs::canonicalize(&workspace.path)? == workspace.path
                && !workspace.name.is_empty()
                && workspace.name.len() <= 256
                && !workspace.name.chars().any(char::is_control)
                && workspaces[..i]
                    .iter()
                    .all(|w| w.id != workspace.id && w.path != workspace.path),
            "invalid approved workspace"
        );
    }
    ensure!(
        serde_json::to_vec(&workspaces)?.len() <= 12_000,
        "approved workspace metadata exceeds limit"
    );
    ensure!(
        !rights.is_empty()
            && rights.len() <= 16
            && rights
                .iter()
                .enumerate()
                .all(|(i, r)| !rights[..i].contains(r)),
        "invalid rights"
    );
    ensure!(
        accounts.len() <= 64 && enrollment_connections.len() <= 32,
        "account scope exceeds executing-host account capacity"
    );
    let endpoint = crate::origin::validate_origin(endpoint, true)
        .map_err(|_| anyhow::anyhow!("endpoint requires HTTPS or literal loopback"))?;
    // Retain redemption material until grant expiry; never evict live retry evidence.
    state.invitations.retain(|p| {
        p.redemption.as_ref().map_or(p.expires_at_ms > now, |r| {
            !r.publication_observed || r.grant.expires_at_ms > now
        })
    });
    ensure!(
        state.invitations.len() < LIMIT,
        "invitation capacity exhausted"
    );
    let invitation = Invitation {
        protocol: 1,
        invitation_id: Uuid::new_v4(),
        principal_id,
        vessel_id: identity(root)?,
        endpoint,
        expires_at_ms: now
            .checked_add(ttl_seconds * 1000)
            .ok_or_else(|| anyhow::anyhow!("expiry overflow"))?,
        code: secret(),
    };
    let mut event = audit::event(
        Kind::InvitationRecorded,
        now,
        invitation.vessel_id,
        None,
        Some(invitation.invitation_id),
        None,
    );
    event.principal_id = Some(principal_id);
    event.expires_at_ms = Some(invitation.expires_at_ms);
    event.workspace_ids = workspaces.iter().map(|w| w.id).collect();
    event.rights = rights.clone();
    event.account_ids = accounts.clone();
    event.enrollment_connection_ids = enrollment_connections.clone();
    append(&mut state, event)?;
    state.invitations.push(Pending {
        invitation_id: invitation.invitation_id,
        principal_id,
        vessel_id: invitation.vessel_id,
        endpoint: invitation.endpoint.clone(),
        expires_at_ms: invitation.expires_at_ms,
        code_hash: store::hash(&invitation.code),
        attempts: 0,
        workspaces,
        rights,
        accounts,
        enrollment_connections,
        redemption: None,
    });
    save(root, &state)?;
    Ok(invitation)
}

/// HTTP-only secret input. Errors are deliberately generic and never contain input.
/// Exact retries return the original credential, even after invitation expiry, but
/// never after grant expiry/revocation. No command receipt contains these secrets.
pub fn redeem(
    root: &Path,
    origin: &str,
    expected_vessel_id: Option<Uuid>,
    request: PairRequest,
) -> Result<WorkspaceCredential> {
    ensure!(
        request.protocol == 1
            && !request.command_id.is_nil()
            && !request.principal_id.is_nil()
            && !request.invitation_id.is_nil()
            && request.code.len() == 64,
        PairRefusal::Denied
    );
    let origin = crate::origin::validate_origin(origin, true)
        .map_err(|_| anyhow::anyhow!(PairRefusal::Denied))?;
    let _lock = lock(root)?;
    let mut state = load(root)?;
    let now = store::now()?;
    start_audit(root, &mut state, now)?;
    let position = state
        .invitations
        .iter()
        .position(|p| p.invitation_id == request.invitation_id);
    let code_hash = store::hash(&request.code);
    let matching = position.is_some_and(|position| {
        let p = &state.invitations[position];
        bool::from(code_hash.as_bytes().ct_eq(p.code_hash.as_bytes()))
            && p.principal_id == request.principal_id
            && p.endpoint == origin
    });
    if !matching {
        if now.saturating_sub(state.window_ms) >= 60_000 {
            state.window_ms = now;
            state.attempts = 0;
        }
        ensure!(state.attempts < 120, PairRefusal::RateLimited);
        state.attempts += 1;
        if let Some(position) = position {
            let p = &mut state.invitations[position];
            p.attempts = p.attempts.saturating_add(1);
        }
        // Anonymous failures share a durable abuse budget. Possession of the
        // invitation secret must still permit redemption and exact retries when
        // strangers have exhausted that budget. Per-invitation lockout remains.
        save(root, &state)?;
        anyhow::bail!(PairRefusal::Denied);
    }
    let position = position.expect("matching invitation");
    let p = &state.invitations[position];
    let vessel_id = identity(root)?;
    if let Some(expected) = expected_vessel_id {
        ensure!(expected == vessel_id, PairRefusal::IdentityChanged);
    }
    ensure!(p.vessel_id == vessel_id, PairRefusal::Denied);
    if p.redemption.is_none() {
        ensure!(p.attempts < 8 && p.expires_at_ms > now, PairRefusal::Denied);
        ensure!(
            !state.invitations.iter().any(|p| p
                .redemption
                .as_ref()
                .is_some_and(|r| r.command_id == request.command_id)),
            PairRefusal::Denied
        );
        ensure!(
            std::fs::read_dir(root.join("access/connections"))?
                .take(LIMIT)
                .count()
                < LIMIT,
            PairRefusal::Capacity
        );
        let p = &mut state.invitations[position];
        let credential = WorkspaceCredential {
            schema_version: 1,
            kind: "workspace".into(),
            endpoint: p.endpoint.clone(),
            grant_id: Uuid::new_v4(),
            principal_id: p.principal_id,
            vessel_id: p.vessel_id,
            token: secret(),
        };
        let grant = ConnectionGrant {
            schema_version: 1,
            grant_id: credential.grant_id,
            principal_id: p.principal_id,
            vessel_id: p.vessel_id,
            revision: 1,
            rights: p.rights.clone(),
            accounts: p.accounts.clone(),
            enrollment_connections: p.enrollment_connections.clone(),
            expires_at_ms: now
                .checked_add(GRANT_TTL_MS)
                .ok_or_else(|| anyhow::anyhow!("expiry overflow"))?,
            revoked: false,
            token_hash: store::hash(&credential.token),
            workspaces: p.workspaces.clone(),
        };
        ensure!(
            serde_json::to_vec(&grant)?.len() <= 16_384,
            "connection record exceeds limit"
        );
        p.redemption = Some(Redemption {
            command_id: request.command_id,
            credential,
            grant,
            publication_observed: false,
            audit_tracked: true,
        });
        let r = p.redemption.as_ref().expect("new redemption");
        let event = audit::event(
            Kind::GrantPublicationIntended,
            now,
            r.grant.vessel_id,
            Some(r.command_id),
            Some(p.invitation_id),
            Some(&r.grant),
        );
        append(&mut state, event)?;
        // Intent and event are one durable encrypted replacement before authority.
        save_intent(root, &state)?;
    }
    let redemption = state.invitations[position]
        .redemption
        .as_ref()
        .expect("retained redemption");
    ensure!(
        redemption.command_id == request.command_id && redemption.grant.expires_at_ms > now,
        PairRefusal::Denied
    );
    ensure!(
        !state
            .revocations
            .iter()
            .any(|r| r.grant_id == redemption.grant.grant_id),
        PairRefusal::Denied
    );
    let path = connection_path(root, redemption.grant.grant_id);
    if path.try_exists()? {
        let current: ConnectionGrant = store::load(&path)?;
        ensure!(
            same_grant(&current, &redemption.grant)?,
            PairRefusal::Denied
        );
    } else {
        ensure!(
            !redemption.publication_observed,
            "observed grant disappeared; authority will not be recreated"
        );
        #[cfg(test)]
        tests::failpoint("grant_publish")?;
        store::save(&path, &redemption.grant)?;
    }
    observe_redemption(root, &mut state, position, now)?;
    let redemption = state.invitations[position]
        .redemption
        .as_ref()
        .expect("retained redemption");
    // Avoid requiring a Debug implementation or leaking through a receipt type.
    Ok(serde_json::from_value(serde_json::to_value(
        &redemption.credential,
    )?)?)
}

/// Local owner revocation with durable exact command identity. Returns no secret.
pub fn revoke(
    root: &Path,
    grant_id: Uuid,
    expected_revision: u64,
    command_id: Uuid,
) -> Result<Value> {
    ensure!(
        !grant_id.is_nil() && !command_id.is_nil(),
        "nil revocation identity"
    );
    let _lock = lock(root)?;
    let mut state = load(root)?;
    let now = store::now()?;
    start_audit(root, &mut state, now)?;
    // Completed exact retries return the retained historical outcome, even if a
    // later explicit revocation advanced the current grant revision.
    if let Some(prior) = state
        .revocations
        .iter()
        .find(|r| r.command_id == command_id)
    {
        ensure!(
            prior.grant_id == grant_id && prior.expected_revision == expected_revision,
            "revocation command conflict"
        );
        if prior.publication_observed {
            let revision = expected_revision
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("revision overflow"))?;
            return Ok(
                json!({"grant_id":grant_id,"revision":revision,"revoked":true,"cleanup":"not_observed"}),
            );
        }
    }
    // Before changing a published grant, preserve any pending publication observation.
    if let Some(position) = state.invitations.iter().position(|p| {
        p.redemption.as_ref().is_some_and(|r| {
            r.grant.grant_id == grant_id && r.audit_tracked && !r.publication_observed
        })
    }) {
        observe_redemption(root, &mut state, position, now)?;
    }
    let path = connection_path(root, grant_id);
    let mut grant: ConnectionGrant = store::load(&path)?;
    // Finalize an earlier exact observed revocation before another changes revision.
    let pending: Vec<_> = state
        .revocations
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.audit_tracked
                && !r.publication_observed
                && r.grant_id == grant_id
                && r.expected_revision.checked_add(1) == Some(grant.revision)
                && grant.revoked
        })
        .map(|(i, _)| i)
        .collect();
    for position in pending {
        observe_revocation(root, &mut state, position, &grant, now)?;
    }
    let revision = expected_revision
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("revision overflow"))?;
    let prior_position = state
        .revocations
        .iter()
        .position(|r| r.command_id == command_id);
    if let Some(position) = prior_position {
        let prior = &state.revocations[position];
        ensure!(
            prior.grant_id == grant_id && prior.expected_revision == expected_revision,
            "revocation command conflict"
        );
        if grant.revoked && grant.revision == revision {
            observe_revocation(root, &mut state, position, &grant, now)?;
            return Ok(
                json!({"grant_id":grant_id,"revision":revision,"revoked":true,"cleanup":"not_observed"}),
            );
        }
        ensure!(
            !prior.publication_observed,
            "observed revocation changed; original outcome retained"
        );
    } else {
        ensure!(
            state.revocations.len() < LIMIT,
            "revocation receipt capacity exhausted"
        );
        ensure!(
            grant.revision == expected_revision,
            "grant revision conflict"
        );
        state.revocations.push(Revocation {
            command_id,
            grant_id,
            expected_revision,
            publication_observed: false,
            audit_tracked: true,
        });
        let event = audit::event(
            Kind::RevocationIntended,
            now,
            grant.vessel_id,
            Some(command_id),
            None,
            Some(&grant),
        );
        append(&mut state, event)?;
        save_intent(root, &state)?;
    }
    ensure!(
        grant.revision == expected_revision,
        "grant revision conflict"
    );
    grant.revoked = true;
    grant.revision = revision;
    store::save(&path, &grant)?;
    let position = state
        .revocations
        .iter()
        .position(|r| r.command_id == command_id)
        .expect("retained revocation");
    observe_revocation(root, &mut state, position, &grant, now)?;
    Ok(json!({"grant_id":grant_id,"revision":revision,"revoked":true,"cleanup":"not_observed"}))
}

/// Explicit offline conversion; retains original transaction/credential identities.
pub fn protect_credentials(root: &Path) -> Result<()> {
    ensure!(
        root.is_absolute() && root.is_dir() && std::fs::canonicalize(root)? == root,
        "protection requires an existing canonical Vessel directory"
    );
    voyage_storage::credentials::check_key()?;
    let _lock = lock(root)?;
    if state_path(root).try_exists()? {
        load(root)?; // Validate without changing the canonical plaintext representation.
        let bytes = store::read_bounded(
            &state_path(root),
            (STATE_BYTES + voyage_storage::credentials::OVERHEAD) as u64,
        )?;
        let plaintext =
            voyage_storage::credentials::open(b"vessel-workspace-pairing-state-v1", &bytes)?;
        let sealed =
            voyage_storage::credentials::seal(b"vessel-workspace-pairing-state-v1", &plaintext)?;
        store::save_bytes(
            &state_path(root),
            &sealed,
            STATE_BYTES + voyage_storage::credentials::OVERHEAD,
        )?;
    }
    // Legacy interrupted atomic-write files may also contain recovery secrets.
    // Encrypt their opaque bytes without interpreting them as committed state.
    for (i, entry) in std::fs::read_dir(directory(root))?.enumerate() {
        ensure!(i < LIMIT * 2, "pairing migration entry limit exceeded");
        let entry = entry?;
        let name = entry.file_name();
        if name
            .to_str()
            .and_then(|n| n.strip_prefix(".pending-"))
            .is_some_and(|id| Uuid::parse_str(id).is_ok())
        {
            let bytes = store::read_bounded(
                &entry.path(),
                (STATE_BYTES + voyage_storage::credentials::OVERHEAD) as u64,
            )?;
            if !voyage_storage::credentials::encrypted(&bytes) {
                let sealed = voyage_storage::credentials::seal(
                    b"vessel-workspace-pairing-state-v1",
                    &bytes,
                )?;
                store::save_bytes(
                    &entry.path(),
                    &sealed,
                    STATE_BYTES + voyage_storage::credentials::OVERHEAD,
                )?;
            }
        }
    }
    Ok(())
}

fn observe_revocation(
    root: &Path,
    state: &mut State,
    position: usize,
    grant: &ConnectionGrant,
    now: u64,
) -> Result<()> {
    if state.revocations[position].publication_observed {
        return Ok(());
    }
    let observed: ConnectionGrant = store::load(&connection_path(root, grant.grant_id))?;
    ensure!(
        same_grant(&observed, grant)?,
        "revocation publication changed; original intent retained"
    );
    let event = audit::event(
        Kind::RevocationObserved,
        now,
        grant.vessel_id,
        Some(state.revocations[position].command_id),
        None,
        Some(grant),
    );
    append(state, event)?;
    state.revocations[position].publication_observed = true;
    #[cfg(test)]
    tests::failpoint("audit_finalize")?;
    save(root, state)
}

/// Read-only local-owner journal view. Unlocks private state, emits allowlisted metadata only.
pub fn connection_audit(root: &Path, limit: usize, cursor: Option<&str>) -> Result<Value> {
    ensure!(
        root.is_absolute() && std::fs::canonicalize(root)? == root,
        "audit requires a canonical absolute directory"
    );
    ensure!(
        inventory_directory(root)?,
        "Vessel state directory unavailable"
    );
    if !inventory_directory(&root.join("access"))? || !inventory_directory(&directory(root))? {
        return audit::page(None, limit, cursor);
    }
    let lock = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(directory(root).join("lock"))
        .map_err(|_| anyhow::anyhow!("connection audit lock unavailable"))?;
    let m = lock.metadata()?;
    ensure!(
        m.is_file()
            && m.nlink() == 1
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0,
        "unsafe connection audit lock"
    );
    lock.try_lock_shared()
        .map_err(|_| anyhow::anyhow!("pairing busy; retry audit read"))?;
    let state = load(root)?;
    audit::page(state.audit.as_ref(), limit, cursor)
}

#[cfg(test)]
mod tests;
