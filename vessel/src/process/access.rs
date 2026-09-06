//! Explicit bearer grants supplement account-owner access; execution policy stays local.
mod routing;
pub(super) mod store;
use super::{registry, service::Supervisor};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use uuid::Uuid;
use voyage_protocol::process::*;

impl Supervisor {
    pub(super) async fn grant(&self, command: VesselCommand) -> Result<Value> {
        let VesselCommand::Grant {
            command_id,
            grant_id,
            principal_id,
            session_id,
            workspace,
            rights,
            expires_at_ms,
            enrollment,
            endpoint,
        } = &command
        else {
            anyhow::bail!("not a grant")
        };
        ensure!(
            !command_id.is_nil()
                && !grant_id.is_nil()
                && !principal_id.is_nil()
                && !session_id.is_nil(),
            "nil access identity"
        );
        let _serial = self.registrations.lock().await;
        store::initialize(&self.directory)?;
        let recorded = registry::command_record(&self.directory, *command_id, &command, false)?;
        if recorded && store::grant_path(&self.directory, *grant_id).exists() {
            let credential: AccessCredential =
                store::load(&store::credential_path(&self.directory, *grant_id))?;
            return Ok(serde_json::to_value(credential)?);
        }
        ensure!(
            !store::grant_path(&self.directory, *grant_id).exists(),
            "grant ID already exists"
        );
        ensure!(
            std::fs::read_dir(store::directory(&self.directory).join("grants"))?
                .take(4096)
                .count()
                < 4096,
            "grant capacity exhausted"
        );
        let now = store::now()?;
        ensure!(
            *expires_at_ms > now && *expires_at_ms - now <= 30 * 24 * 60 * 60 * 1000,
            "grant expiry must be within 30 days"
        );
        ensure!(
            !rights.is_empty()
                && rights.len() <= 8
                && rights
                    .iter()
                    .enumerate()
                    .all(|(i, right)| !rights[..i].contains(right)),
            "invalid grant rights"
        );
        let workspace = std::fs::canonicalize(workspace)?;
        ensure!(workspace.is_dir(), "grant workspace must be a directory");
        if let Some(existing) = _serial.get(session_id) {
            ensure!(existing.workspace == workspace, "grant workspace mismatch");
        }
        if let Some(identity) = enrollment {
            store::enrollment(identity)?;
        }
        let endpoint = crate::enrollment::validate_origin(endpoint, true)
            .map_err(|_| anyhow::anyhow!("grant endpoint requires HTTPS or literal loopback"))?;
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let credential = if recorded && store::credential_path(&self.directory, *grant_id).exists()
        {
            store::load(&store::credential_path(&self.directory, *grant_id))?
        } else {
            AccessCredential {
                endpoint,
                grant_id: *grant_id,
                session_id: *session_id,
                token,
            }
        };
        let grant = ProcessGrant {
            grant_id: *grant_id,
            principal_id: *principal_id,
            session_id: *session_id,
            workspace,
            revision: 1,
            rights: rights.clone(),
            expires_at_ms: *expires_at_ms,
            revoked: false,
            enrollment: enrollment.clone(),
            token_hash: store::hash(&credential.token),
            parent_grant: None,
            participant_binding: None,
        };
        // Immutable intent precedes publication. Retrying only completes deterministic local metadata.
        registry::command_record(&self.directory, *command_id, &command, true)?;
        store::save(
            &store::credential_path(&self.directory, *grant_id),
            &credential,
        )?;
        store::save(&store::grant_path(&self.directory, *grant_id), &grant)
            .map_err(|error| error.context(super::routing::OutcomeUnknown))?;
        Ok(serde_json::to_value(credential)?)
    }
    pub(super) async fn revoke_grant(&self, command: VesselCommand) -> Result<Value> {
        let VesselCommand::RevokeGrant {
            command_id,
            grant_id,
            expected_revision,
        } = &command
        else {
            anyhow::bail!("not a revocation")
        };
        ensure!(!command_id.is_nil(), "nil revocation identity");
        let _serial = self.registrations.lock().await;
        let mut grant: ProcessGrant = store::load(&store::grant_path(&self.directory, *grant_id))?;
        if registry::command_record(&self.directory, *command_id, &command, false)?
            && grant.revoked
            && grant.revision == expected_revision.saturating_add(1)
        {
            return Ok(json!({"grant_id":grant_id,"revision":grant.revision,"revoked":true}));
        }
        ensure!(
            grant.revision == *expected_revision,
            "grant revision conflict"
        );
        registry::command_record(&self.directory, *command_id, &command, true)?;
        grant.revoked = true;
        grant.revision = grant
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("grant revision overflow"))?;
        store::save(&store::grant_path(&self.directory, *grant_id), &grant)
            .map_err(|error| error.context(super::routing::OutcomeUnknown))?;
        Ok(
            json!({"grant_id":grant_id,"revision":grant.revision,"revoked":true,"cleanup":"requested_by_authority_watch"}),
        )
    }
}
