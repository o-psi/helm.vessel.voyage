//! Network preparation is non-mutating except explicit, journaled pairing.
use super::*;
use serde_json::Value;
use voyage_protocol::vessel::{VESSEL_API_VERSION, VesselCommand};

#[derive(Clone, Debug, Deserialize)]
pub struct PairingCapabilities {
    pub protocol: u32,
    pub vessel_id: Uuid,
    pub features: Vec<String>,
}
/// Public metadata does not describe invitation authority. The resulting grant is
/// reviewed after redemption, before saving a live connection.
pub async fn pairing_capabilities(endpoint: &str) -> Result<PairingCapabilities> {
    let response = access::http(false)?
        .get(access::endpoint(endpoint, "/v1/vessel/pair/capabilities")?)
        .send()
        .await
        .map_err(|_| ConnectionFailure::Offline)?;
    let (status, bytes) = access::bounded(response).await?;
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(ConnectionFailure::Version.into());
    }
    ensure!(
        status.is_success(),
        "Vessel pairing discovery unavailable (HTTP {})",
        status.as_u16()
    );
    let capabilities: PairingCapabilities = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid public Vessel pairing metadata"))?;
    if capabilities.protocol != VESSEL_API_VERSION {
        return Err(ConnectionFailure::Version.into());
    }
    ensure!(
        !capabilities.vessel_id.is_nil()
            && capabilities
                .features
                .iter()
                .any(|f| f == "workspace_pairing"),
        "Vessel does not support workspace pairing"
    );
    Ok(capabilities)
}
#[derive(Serialize, Deserialize)]
struct PairRequest {
    protocol: u32,
    command_id: Uuid,
    principal_id: Uuid,
    invitation_id: Uuid,
    code: String,
}
// No Debug and no public secret fields.
#[derive(Serialize, Deserialize)]
struct Redemption {
    summary: PendingPair,
    request: PairRequest,
    credential_ref: Uuid,
    connection_id: Uuid,
}

impl Registry {
    pub async fn prepare_import(&self, path: PathBuf) -> Result<ConnectionPreview> {
        let legacy = LegacyRoute {
            directory: PathBuf::new(),
            access_file: Some(path.clone()),
        };
        self.prepare_import_route(path, legacy).await
    }
    /// Startup callers supply the original directory/access path verbatim. No
    /// canonicalization, endpoint matching, or SSH reinterpretation is performed.
    pub async fn prepare_import_route(
        &self,
        path: PathBuf,
        legacy: LegacyRoute,
    ) -> Result<ConnectionPreview> {
        ensure!(
            legacy.access_file.as_ref() == Some(&path),
            "legacy import route must name the exact source access file"
        );
        let bytes = tokio::task::spawn_blocking(move || {
            private::read_path(&path, private::CREDENTIAL_LIMIT)
        })
        .await??;
        let credential = Credential::parse(&bytes)?;
        // Read/copy the exact checked bytes, never reopen a source after network IO.
        let (vessel_id, principal_id, scope, metadata) =
            inspect(&credential, credential.vessel_id()).await?;
        let connection = Connection {
            id: Uuid::new_v4(),
            alias: String::new(),
            endpoint: credential.endpoint().into(),
            vessel_id,
            principal_id,
            grant_id: credential.grant_id(),
            scope,
            credential_ref: Uuid::new_v4(),
            autoconnect: true,
            workspace_preference: None,
            revision: 0,
            forgotten: false,
            legacy_route: Some(legacy),
            metadata,
        };
        let registry = self.clone();
        tokio::task::spawn_blocking(move || registry.checkpoint_preview(connection, Some(bytes)))
            .await?
    }
    fn checkpoint_preview(
        &self,
        connection: Connection,
        bytes: Option<Vec<u8>>,
    ) -> Result<ConnectionPreview> {
        let dir = self.directory()?;
        let _lock = dir.lock()?;
        if let Some(bytes) = bytes {
            let name = format!("{}.credential", connection.credential_ref);
            if let Some(existing) = dir.read(&name, private::CREDENTIAL_LIMIT)? {
                ensure!(
                    existing == bytes,
                    "immutable credential checkpoint conflict; original retained"
                );
            } else {
                dir.write(&name, &bytes, private::CREDENTIAL_LIMIT)?;
            }
        }
        let preview_id = connection.id;
        dir.write(
            &format!("{preview_id}.preview"),
            &serde_json::to_vec(&connection)?,
            private::CREDENTIAL_LIMIT * 4,
        )?;
        Ok(ConnectionPreview {
            connection,
            preview_id,
        })
    }
    pub async fn prepare_pair(
        &self,
        endpoint: String,
        invitation: String,
    ) -> Result<ConnectionPreview> {
        let registry = self.clone();
        let principal_id = tokio::task::spawn_blocking(move || registry.principal_id()).await??;
        let capabilities = pairing_capabilities(&endpoint).await?;
        let (invitation_id, code) = invitation
            .split_once('.')
            .context("pairing input must be invitationUUID.secret")?;
        let invitation_id = Uuid::parse_str(invitation_id)
            .map_err(|_| anyhow::anyhow!("invalid invitation identity"))?;
        ensure!(
            !invitation_id.is_nil()
                && !code.is_empty()
                && code.len() <= 4096
                && code.bytes().all(|b| b.is_ascii_graphic()),
            "invalid private invitation code"
        );
        let request = PairRequest {
            protocol: VESSEL_API_VERSION,
            command_id: Uuid::new_v4(),
            principal_id,
            invitation_id,
            code: code.into(),
        };
        // Normalize only the endpoint, never a legacy route used by drafts.
        let endpoint = access::endpoint(&endpoint, "/")?.to_string();
        let registry = self.clone();
        let pending_id = tokio::task::spawn_blocking(move || {
            let dir = registry.directory()?;
            let _lock = dir.lock()?;
            let mut snapshot = Self::read_snapshot(&dir)?;
            // A repeated Add after timeout MUST reconcile the original operation.
            if let Some(old) = snapshot
                .pending_pairs
                .iter()
                .find(|p| p.invitation_id == invitation_id && p.endpoint == endpoint)
            {
                ensure!(
                    old.vessel_id == capabilities.vessel_id,
                    "pairing Vessel identity changed; original redemption retained"
                );
                let old_redemption = read_redemption(&dir, old.id)?;
                ensure!(
                    old_redemption.request.code == request.code,
                    "invitation differs from retained redemption; original preserved"
                );
                return Ok(old.id);
            }
            ensure!(
                snapshot.pending_pairs.len() < 1024,
                "pending pairing capacity reached; recovery records retained"
            );
            let summary = PendingPair {
                id: Uuid::new_v4(),
                endpoint,
                vessel_id: capabilities.vessel_id,
                principal_id: request.principal_id,
                invitation_id,
                command_id: request.command_id,
            };
            let pending_id = summary.id;
            let redemption = Redemption {
                summary: summary.clone(),
                request,
                credential_ref: Uuid::new_v4(),
                connection_id: Uuid::new_v4(),
            };
            // Durable private intent and discoverable index both precede network.
            dir.write(
                &format!("{pending_id}.redemption"),
                &serde_json::to_vec(&redemption)?,
                private::CREDENTIAL_LIMIT,
            )?;
            snapshot.pending_pairs.push(summary);
            Self::commit(&dir, &mut snapshot)?;
            Ok::<_, anyhow::Error>(pending_id)
        })
        .await??;
        self.resume_pair(pending_id).await
    }
    pub async fn resume_pair(&self, pending_id: Uuid) -> Result<ConnectionPreview> {
        let registry = self.clone();
        let (redemption, saved) = tokio::task::spawn_blocking(move || {
            let dir = registry.directory()?;
            let redemption = read_redemption(&dir, pending_id)?;
            let saved = dir.read(
                &format!("{}.credential", redemption.credential_ref),
                private::CREDENTIAL_LIMIT,
            )?;
            Ok::<_, anyhow::Error>((redemption, saved))
        })
        .await??;
        let bytes = if let Some(saved) = saved {
            saved
        } else {
            let response = access::http(false)?
                .post(access::endpoint(
                    &redemption.summary.endpoint,
                    "/v1/vessel/pair",
                )?)
                .header("x-voyage-vessel", redemption.summary.vessel_id.to_string())
                .json(&redemption.request)
                .send()
                .await
                .map_err(|_| ConnectionFailure::Offline)?;
            let value = access::envelope(response).await?;
            // Accept the documented direct credential result, never a changed origin.
            let bytes = serde_json::to_vec(&value)?;
            ensure!(
                bytes.len() <= private::CREDENTIAL_LIMIT,
                "pairing credential exceeds limit"
            );
            let credential = Credential::parse(&bytes)?;
            if credential.vessel_id() != Some(redemption.summary.vessel_id)
                || credential.principal_id() != Some(redemption.summary.principal_id)
                || access::endpoint(credential.endpoint(), "/")?
                    != access::endpoint(&redemption.summary.endpoint, "/")?
            {
                return Err(ConnectionFailure::Identity.into());
            }
            // Checkpoint credential BEFORE metadata refresh. A timeout here is
            // resolved from the same request identity, never a second grant.
            let registry = self.clone();
            let id = redemption.credential_ref;
            let saved = bytes.clone();
            tokio::task::spawn_blocking(move || {
                let dir = registry.directory()?;
                let _lock = dir.lock()?;
                let name = format!("{id}.credential");
                if let Some(existing) = dir.read(&name, private::CREDENTIAL_LIMIT)? {
                    ensure!(
                        existing == saved,
                        "pairing returned a different credential; original retained"
                    );
                } else {
                    dir.write(&name, &saved, private::CREDENTIAL_LIMIT)?;
                }
                Ok::<_, anyhow::Error>(())
            })
            .await??;
            bytes
        };
        let credential = Credential::parse(&bytes)?;
        if credential.vessel_id() != Some(redemption.summary.vessel_id)
            || credential.principal_id() != Some(redemption.summary.principal_id)
        {
            return Err(ConnectionFailure::Identity.into());
        }
        let (vessel_id, principal_id, scope, metadata) =
            inspect(&credential, Some(redemption.summary.vessel_id)).await?;
        let connection = Connection {
            id: redemption.connection_id,
            alias: String::new(),
            endpoint: credential.endpoint().into(),
            vessel_id,
            principal_id,
            grant_id: credential.grant_id(),
            scope,
            credential_ref: redemption.credential_ref,
            autoconnect: true,
            workspace_preference: None,
            revision: 0,
            forgotten: false,
            legacy_route: None,
            metadata,
        };
        let registry = self.clone();
        tokio::task::spawn_blocking(move || registry.checkpoint_preview(connection, None)).await?
    }
}
fn read_redemption(dir: &private::Directory, id: Uuid) -> Result<Redemption> {
    let bytes = dir
        .read(&format!("{id}.redemption"), private::CREDENTIAL_LIMIT)?
        .context("private pairing recovery record unavailable")?;
    let value: Redemption = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid private pairing recovery record"))?;
    ensure!(
        value.summary.id == id
            && value.summary.command_id == value.request.command_id
            && value.summary.principal_id == value.request.principal_id
            && value.summary.invitation_id == value.request.invitation_id
            && value.request.protocol == VESSEL_API_VERSION,
        "pairing recovery identity mismatch; original record retained"
    );
    Ok(value)
}
fn uuid(value: &Value, field: &str) -> Result<Uuid> {
    value
        .get(field)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|id| !id.is_nil())
        .ok_or_else(|| anyhow::anyhow!("Vessel metadata missing valid {field}"))
}
pub(super) async fn inspect(
    credential: &Credential,
    pin: Option<Uuid>,
) -> Result<(Uuid, Option<Uuid>, Scope, Metadata)> {
    let value = access::exchange_credential(credential, pin, VesselCommand::Capabilities).await?;
    if value.get("protocol").and_then(Value::as_u64) != Some(VESSEL_API_VERSION as u64) {
        return Err(ConnectionFailure::Version.into());
    }
    let vessel_id = uuid(&value, "vessel_id")?;
    if pin.is_some_and(|pin| pin != vessel_id) {
        return Err(ConnectionFailure::Identity.into());
    }
    let principal_id = Some(uuid(&value, "principal_id")?);
    if credential
        .principal_id()
        .is_some_and(|p| Some(p) != principal_id)
    {
        return Err(ConnectionFailure::Identity.into());
    }
    let mut metadata: Metadata = serde_json::from_value(value.clone())
        .map_err(|_| anyhow::anyhow!("invalid Vessel access metadata"))?;
    ensure!(
        metadata.workspaces.len() <= 256
            && metadata.rights.len() <= 64
            && metadata.features.len() <= 128,
        "Vessel access metadata exceeds bounds"
    );
    for workspace in &metadata.workspaces {
        ensure!(
            !workspace.id.is_nil() && workspace.path.is_absolute() && workspace.name.len() <= 256,
            "invalid approved workspace metadata"
        );
    }
    let scope = match credential {
        Credential::Session(c) => {
            // Older servers omit scope. Only the explicit legacy credential's
            // session is allowed, regardless of workspace fields in a response.
            if uuid(&value, "session_id")? != c.session_id
                || value.get("scope").is_some_and(|s| s != "session")
            {
                return Err(ConnectionFailure::Identity.into());
            }
            metadata.workspaces.clear();
            Scope::Session {
                session_id: c.session_id,
            }
        }
        Credential::Workspace(_) => {
            if value.get("scope").and_then(Value::as_str) != Some("workspaces") {
                return Err(ConnectionFailure::Identity.into());
            }
            ensure!(
                metadata.version.is_some()
                    && metadata.expires_at_ms.is_some()
                    && metadata.grant_revision.is_some()
                    && !metadata.rights.is_empty(),
                "workspace grant is missing review metadata"
            );
            let mut workspace_ids: Vec<_> = metadata.workspaces.iter().map(|w| w.id).collect();
            workspace_ids.sort();
            let n = workspace_ids.len();
            workspace_ids.dedup();
            ensure!(
                n == workspace_ids.len() && n > 0,
                "invalid approved workspace set"
            );
            Scope::Workspaces { workspace_ids }
        }
    };
    if metadata.expires_at_ms.is_some_and(|expiry| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .is_ok_and(|now| now.as_millis() >= expiry as u128)
    }) {
        return Err(ConnectionFailure::Expired.into());
    }
    Ok((vessel_id, principal_id, scope, metadata))
}
