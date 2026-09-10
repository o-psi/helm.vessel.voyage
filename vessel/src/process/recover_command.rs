//! Explicit operator recovery runs in a separate, exclusively fenced runtime.
use super::{access::store, registry, routing, service::Supervisor};
use anyhow::{Result, ensure};
use std::{path::Path, process::Stdio, time::Duration};
use voyage_protocol::process::*;

pub(super) fn restart_permitted(directory: &Path, registration: &ProcessRegistration) -> bool {
    if registration.state == ProcessState::Relinquished {
        return false;
    }
    let Ok(marker) = store::load::<serde_json::Value>(&directory.join("recovered.json")) else {
        return false;
    };
    marker["session_id"] == registration.session_id.to_string()
        && marker["incarnation"] == registration.incarnation.to_string()
        && marker["restart_permitted"] == true
        && matches!(
            marker["cleanup_disposition"].as_str(),
            Some("observed" | "operator_attested")
        )
}

impl Supervisor {
    /// Recover an abandoned owner without supplying operator attestations. This
    /// can record interrupted work, but cannot clear uncertain cleanup, reconcile
    /// tools or claim that retained resources stopped.
    pub(super) async fn recover_abandoned(
        &self,
        session_id: uuid::Uuid,
        incarnation: uuid::Uuid,
    ) -> Result<()> {
        let directory = registry::directory(&self.directory, session_id);
        let registration = self.registration(session_id).await?;
        ensure!(
            registration.incarnation == incarnation,
            "stale runtime incarnation"
        );
        if restart_permitted(&directory, &registration) {
            return Ok(());
        }
        // One stable recovery request per incarnation. Re-observation can consume
        // later guardian evidence without growing the durable command catalogue.
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(format!(
            "voyage-automatic-recovery-v1:{session_id}:{incarnation}"
        ));
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        let marker = self
            .recover(VesselCommand::Recover {
                command_id: uuid::Uuid::from_bytes(bytes),
                session_id,
                incarnation,
                acknowledge_cleanup: None,
                reconcile_tools: None,
                expected_revision: None,
                acknowledge_resources: Vec::new(),
            })
            .await?;
        if marker["restart_permitted"] == true {
            return Ok(());
        }

        // Keep uncertainty visible, but never disable later automatic observation.
        let mut registrations = self.registrations.lock().await;
        let current = registrations
            .get_mut(&session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown session"))?;
        ensure!(
            current.incarnation == incarnation,
            "runtime changed during recovery"
        );
        current.state = ProcessState::CleanupUnconfirmed;
        registry::save(&directory, current)?;
        anyhow::bail!(
            "Saved conversation is available. Previous program cleanup cannot yet be verified; automatic recovery will check again. Older voyages may lack the process evidence needed for automatic cleanup."
        )
    }

    pub(super) async fn recover(&self, command: VesselCommand) -> Result<serde_json::Value> {
        let VesselCommand::Recover {
            command_id,
            session_id,
            incarnation,
            acknowledge_cleanup,
            reconcile_tools,
            expected_revision,
            acknowledge_resources,
        } = &command
        else {
            anyhow::bail!("not recovery");
        };
        ensure!(
            acknowledge_resources.len() <= 256,
            "too many resource attestations"
        );
        ensure!(!command_id.is_nil(), "recovery command ID must be nonnil");
        ensure!(
            reconcile_tools.is_none() || expected_revision.is_some(),
            "tool reconciliation requires exact revision"
        );
        let registrations = self.registrations.lock().await;
        let registration = registrations
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown session"))?;
        ensure!(
            registration.incarnation == *incarnation
                && registration.state != ProcessState::Relinquished,
            "stale or relinquished owner cannot recover"
        );
        let directory = registry::directory(&self.directory, *session_id);
        ensure!(
            routing::inspect(&directory, registration).await.state != ProcessState::Live,
            "live owner cannot be recovered"
        );
        registry::command_record(&self.directory, *command_id, &command, false)?;
        registry::command_record(&self.directory, *command_id, &command, true)?;
        let mut child = tokio::process::Command::new(&self.binary);
        child
            .arg("recover")
            .arg("--directory")
            .arg(&directory)
            .arg("--session")
            .arg(session_id.to_string())
            .arg("--incarnation")
            .arg(incarnation.to_string())
            .arg("--command-id")
            .arg(command_id.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for id in acknowledge_resources {
            child.arg("--acknowledge-resource").arg(id.to_string());
        }
        if let Some(id) = acknowledge_cleanup {
            child.arg("--acknowledge-cleanup").arg(id.to_string());
        }
        if let Some(id) = reconcile_tools {
            child.arg("--reconcile-tools").arg(id.to_string());
        }
        if let Some(revision) = expected_revision {
            child.arg("--expected-revision").arg(revision.to_string());
        }
        let status = tokio::time::timeout(Duration::from_secs(15), child.status())
            .await
            .map_err(|_| {
                anyhow::anyhow!("recovery outcome unconfirmed").context(routing::OutcomeUnknown)
            })?
            .map_err(|error| anyhow::Error::from(error).context(routing::OutcomeUnknown))?;
        if !status.success() {
            return Err(anyhow::anyhow!(
                "runtime recovery refused or incomplete; inspect durable recovery disposition"
            )
            .context(routing::OutcomeUnknown));
        }
        let marker: serde_json::Value = store::load(
            &directory
                .join("recoveries")
                .join(format!("{command_id}.json")),
        )
        .map_err(|error| error.context(routing::OutcomeUnknown))?;
        ensure!(
            marker["command_id"] == command_id.to_string()
                && marker["session_id"] == session_id.to_string()
                && marker["incarnation"] == incarnation.to_string(),
            "recovery disposition incomplete"
        );
        Ok(marker)
    }
}
