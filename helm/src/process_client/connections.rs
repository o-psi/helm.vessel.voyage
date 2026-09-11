//! Owner-private human connection address book. Never model-driven remotes.
//!
//! Credentials are immutable, separately stored, and deliberately retained when a
//! connection is forgotten. Replacement is a NEW identity; pending operations may
//! still require the exact old grant. Updates lock, reread, compare the individual
//! record revision, and merge into the latest snapshot (other windows' edits live).
mod prepare;
pub(super) mod private;

use super::{
    access::{self, ConnectionFailure, Credential},
    transport::Client,
};
use anyhow::{Context, Result, ensure};
pub use prepare::{PairingCapabilities, pairing_capabilities};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
pub type ConnectionId = Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyRoute {
    pub directory: PathBuf,
    pub access_file: Option<PathBuf>,
}
impl LegacyRoute {
    /// Exactly matches the historical new-voyage route. SSH is never represented.
    pub fn key(&self) -> Result<String> {
        Ok(serde_json::to_string(&(
            &self.directory,
            Option::<&str>::None,
            &self.access_file,
        ))?)
    }
    pub fn id(&self) -> ConnectionId {
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        digest.update(b"helm-legacy-route-v1");
        let directory = self.directory.as_os_str().as_encoded_bytes();
        digest.update((directory.len() as u64).to_le_bytes());
        digest.update(directory);
        if let Some(path) = &self.access_file {
            digest.update([1]);
            digest.update(path.as_os_str().as_encoded_bytes());
        } else {
            digest.update([0]);
        }
        let hash = digest.finalize();
        let mut id = [0; 16];
        id.copy_from_slice(&hash[..16]);
        id[6] = (id[6] & 0x0f) | 0x80;
        id[8] = (id[8] & 0x3f) | 0x80;
        Uuid::from_bytes(id)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    Session { session_id: Uuid },
    Workspaces { workspace_ids: Vec<Uuid> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub provider_ready: Option<bool>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub version: Option<String>,
    #[serde(default)]
    pub rights: Vec<String>,
    pub expires_at_ms: Option<u64>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub features: Vec<String>,
    pub grant_revision: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub id: ConnectionId,
    pub alias: String,
    pub endpoint: String,
    pub vessel_id: Uuid,
    pub principal_id: Option<Uuid>,
    pub grant_id: Uuid,
    pub scope: Scope,
    pub credential_ref: Uuid,
    pub autoconnect: bool,
    pub workspace_preference: Option<Uuid>,
    pub revision: u64,
    pub forgotten: bool,
    pub legacy_route: Option<LegacyRoute>,
    pub metadata: Metadata,
}
impl Connection {
    pub fn client(&self, registry: &Registry) -> Client {
        Client::from_connection(self.clone(), registry.credential_path(self.credential_ref))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingPair {
    pub id: Uuid,
    #[serde(default)]
    pub connection_id: Option<Uuid>,
    #[serde(default)]
    pub completed: bool,
    pub endpoint: String,
    pub vessel_id: Uuid,
    pub principal_id: Uuid,
    pub invitation_id: Uuid,
    pub command_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub connections: Vec<Connection>,
    pub pending_pairs: Vec<PendingPair>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            connections: Vec::new(),
            pending_pairs: Vec::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Preferences {
    pub alias: String,
    pub autoconnect: bool,
    pub workspace_preference: Option<Uuid>,
}
#[derive(Clone, Debug)]
pub struct ConnectionPreview {
    pub connection: Connection,
    // Only prepare APIs can construct a saveable review; save compares the private checkpoint.
    preview_id: Uuid,
}
#[derive(Clone, Debug)]
pub struct Registry {
    root: PathBuf,
}
impl Registry {
    /// Parent directory must already exist. Root is created 0700, never chmod-ed.
    pub fn open(root: PathBuf) -> Result<Self> {
        ensure!(
            root.is_absolute(),
            "connection registry path must be absolute"
        );
        private::Directory::open(&root)?;
        Ok(Self { root })
    }
    /// Public, non-secret identity owners can bind an invitation to before redemption.
    pub fn principal_id(&self) -> Result<Uuid> {
        let dir = self.directory()?;
        let _lock = dir.lock()?;
        if let Some(bytes) = dir.read("principal.json", 1024)? {
            let id: Uuid = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("invalid private Helm principal"))?;
            ensure!(!id.is_nil(), "invalid private Helm principal");
            return Ok(id);
        }
        let id = Uuid::new_v4();
        dir.write("principal.json", &serde_json::to_vec(&id)?, 1024)?;
        Ok(id)
    }
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }
    fn directory(&self) -> Result<private::Directory> {
        private::Directory::open(&self.root)
    }
    fn credential_path(&self, id: Uuid) -> PathBuf {
        self.root.join(format!("{id}.credential"))
    }
    fn read_snapshot(dir: &private::Directory) -> Result<Snapshot> {
        let snapshot = match dir.read("registry.json", private::REGISTRY_LIMIT)? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("invalid connection registry; original preserved"))?,
            None => Snapshot::default(),
        };
        Self::validate(&snapshot)?;
        Ok(snapshot)
    }
    fn validate(snapshot: &Snapshot) -> Result<()> {
        ensure!(
            snapshot.schema_version == 1,
            "unsupported connection registry version"
        );
        ensure!(
            snapshot.connections.len() <= 4096 && snapshot.pending_pairs.len() <= 1024,
            "connection registry capacity exceeded"
        );
        let mut ids = std::collections::HashSet::new();
        for c in &snapshot.connections {
            ensure!(
                !c.id.is_nil() && ids.insert(c.id),
                "duplicate or invalid connection identity"
            );
            ensure!(
                !c.vessel_id.is_nil() && !c.credential_ref.is_nil(),
                "invalid pinned connection"
            );
            access::endpoint(&c.endpoint, "/")?;
            preferences(
                c,
                &Preferences {
                    alias: c.alias.clone(),
                    autoconnect: c.autoconnect,
                    workspace_preference: c.workspace_preference,
                },
            )?;
        }
        Ok(())
    }
    fn commit(dir: &private::Directory, snapshot: &mut Snapshot) -> Result<()> {
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .context("registry revision exhausted")?;
        Self::validate(snapshot)?;
        dir.write(
            "registry.json",
            &serde_json::to_vec(snapshot)?,
            private::REGISTRY_LIMIT,
        )
    }
    pub fn load(&self) -> Result<Snapshot> {
        Self::read_snapshot(&self.directory()?)
    }
    pub fn save_preview(
        &self,
        preview: &ConnectionPreview,
        alias: String,
        autoconnect: bool,
    ) -> Result<Connection> {
        let dir = self.directory()?;
        let _lock = dir.lock()?;
        let saved: Connection = serde_json::from_slice(
            &dir.read(
                &format!("{}.preview", preview.preview_id),
                private::CREDENTIAL_LIMIT * 4,
            )?
            .context("connection preview is unavailable")?,
        )
        .map_err(|_| anyhow::anyhow!("invalid private connection preview"))?;
        ensure!(
            saved == preview.connection,
            "connection preview changed; review again"
        );
        let bytes = dir
            .read(
                &format!("{}.credential", saved.credential_ref),
                private::CREDENTIAL_LIMIT,
            )?
            .context("review credential unavailable")?;
        validate_credential(&saved, &Credential::parse(&bytes)?)?;
        let mut snapshot = Self::read_snapshot(&dir)?;
        if let Some(existing) = snapshot
            .connections
            .iter()
            .find(|c| c.id == saved.id)
            .cloned()
        {
            ensure!(
                existing.credential_ref == saved.credential_ref && !existing.forgotten,
                "connection already saved or forgotten; reload registry"
            );
            if Self::complete_pair(&dir, &mut snapshot, saved.id)? {
                Self::commit(&dir, &mut snapshot)?;
            }
            return Ok(existing);
        }
        // Never merge privileges just because endpoint/Vessel matches. Exact grants
        // also cannot be silently revived after Forget; a new review is explicit.
        ensure!(
            !snapshot.connections.iter().any(|c| !c.forgotten
                && c.vessel_id == saved.vessel_id
                && c.grant_id == saved.grant_id),
            "this exact access grant is already saved; use its existing connection"
        );
        let mut connection = saved;
        // An original route can be claimed once, including by a tombstone. A
        // replacement grant at the same pathname cannot steal existing drafts.
        if connection.legacy_route.as_ref().is_some_and(|route| {
            snapshot
                .connections
                .iter()
                .any(|old| old.legacy_route.as_ref() == Some(route))
        }) {
            connection.legacy_route = None;
        }
        let p = Preferences {
            alias,
            autoconnect,
            workspace_preference: None,
        };
        preferences(&connection, &p)?;
        connection.alias = p.alias;
        connection.autoconnect = p.autoconnect;
        connection.revision = 1;
        snapshot.connections.push(connection.clone());
        Self::complete_pair(&dir, &mut snapshot, connection.id)?;
        Self::commit(&dir, &mut snapshot)?;
        Ok(connection)
    }
    fn complete_pair(dir: &private::Directory, snapshot: &mut Snapshot, id: Uuid) -> Result<bool> {
        let mut changed = false;
        for pending in &mut snapshot.pending_pairs {
            if pending.completed {
                continue;
            }
            let connection_id = match pending.connection_id {
                Some(id) => id,
                None => prepare::redemption_connection(dir, pending.id)?,
            };
            if connection_id == id {
                pending.connection_id = Some(id);
                pending.completed = true;
                changed = true;
            }
        }
        Ok(changed)
    }
    pub fn update(
        &self,
        id: Uuid,
        expected_revision: u64,
        value: Preferences,
    ) -> Result<Connection> {
        self.mutate(id, expected_revision, |c| {
            ensure!(
                !c.forgotten,
                "forgotten connection retains recovery material but cannot be edited"
            );
            preferences(c, &value)?;
            c.alias = value.alias;
            c.autoconnect = value.autoconnect;
            c.workspace_preference = value.workspace_preference;
            Ok(())
        })
    }
    /// No private credential, preview or pending redemption is deleted. The UI must
    /// explain this retention rather than promising erasure or remote revocation.
    pub fn forget(&self, id: Uuid, expected_revision: u64) -> Result<Connection> {
        self.mutate(id, expected_revision, |c| {
            c.forgotten = true;
            c.autoconnect = false;
            Ok(())
        })
    }
    /// Restore only the retained route and exact original credential. This does
    /// not renew authority, replay commands, or connect to a replacement grant.
    pub fn restore(&self, id: Uuid, expected_revision: u64) -> Result<Connection> {
        self.mutate(id, expected_revision, |c| {
            ensure!(c.forgotten, "connection is not forgotten; reload registry");
            self.read_credential(c)?;
            c.forgotten = false;
            c.autoconnect = false;
            Ok(())
        })
    }
    fn mutate(
        &self,
        id: Uuid,
        expected_revision: u64,
        edit: impl FnOnce(&mut Connection) -> Result<()>,
    ) -> Result<Connection> {
        let dir = self.directory()?;
        let _lock = dir.lock()?;
        let mut snapshot = Self::read_snapshot(&dir)?;
        let connection = snapshot
            .connections
            .iter_mut()
            .find(|c| c.id == id)
            .context("connection not found")?;
        ensure!(
            connection.revision == expected_revision,
            "connection changed in another Helm window; reload before editing"
        );
        edit(connection)?;
        connection.revision = connection
            .revision
            .checked_add(1)
            .context("connection revision exhausted")?;
        let result = connection.clone();
        Self::commit(&dir, &mut snapshot)?;
        Ok(result)
    }
    pub async fn refresh(&self, connection: &Connection) -> Result<Metadata> {
        let registry = self.clone();
        let connection = connection.clone();
        let c = connection.clone();
        let credential =
            tokio::task::spawn_blocking(move || registry.read_credential(&c)).await??;
        let (vessel, principal, scope, metadata) =
            prepare::inspect(&credential, Some(connection.vessel_id)).await?;
        if vessel != connection.vessel_id
            || principal != connection.principal_id
            || scope != connection.scope
        {
            return Err(ConnectionFailure::Identity.into());
        }
        Ok(metadata)
    }
    fn read_credential(&self, connection: &Connection) -> Result<Credential> {
        let bytes = self
            .directory()?
            .read(
                &format!("{}.credential", connection.credential_ref),
                private::CREDENTIAL_LIMIT,
            )?
            .context("managed credential missing; recovery record retained")?;
        let credential = Credential::parse(&bytes)?;
        validate_credential(connection, &credential)?;
        Ok(credential)
    }
}
fn preferences(connection: &Connection, preferences: &Preferences) -> Result<()> {
    ensure!(
        preferences.alias.len() <= 256 && !preferences.alias.chars().any(char::is_control),
        "connection alias must be at most 256 bytes without control characters"
    );
    ensure!(
        preferences.workspace_preference.is_none_or(|id| connection
            .metadata
            .workspaces
            .iter()
            .any(|w| w.id == id)),
        "workspace is not authorized for this connection"
    );
    Ok(())
}
pub(super) fn validate_credential(connection: &Connection, credential: &Credential) -> Result<()> {
    let scope_matches = match (&connection.scope, credential) {
        (Scope::Session { session_id }, Credential::Session(c)) => *session_id == c.session_id,
        (Scope::Workspaces { .. }, Credential::Workspace(_)) => true,
        _ => false,
    };
    if credential.endpoint() != connection.endpoint
        || credential.grant_id() != connection.grant_id
        || credential
            .principal_id()
            .is_some_and(|id| Some(id) != connection.principal_id)
        || credential
            .vessel_id()
            .is_some_and(|id| id != connection.vessel_id)
        || !scope_matches
    {
        return Err(ConnectionFailure::Identity.into());
    }
    Ok(())
}

/// Explicit offline migration of retained connection secrets; never edits identities.
pub fn protect_credentials(root: &std::path::Path) -> Result<usize> {
    private::protect(root)
}
