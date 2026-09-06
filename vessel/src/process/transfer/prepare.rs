use super::*;
use crate::process::identity;
impl Supervisor {
    pub(super) async fn prepare_transfer(
        &self,
        command: VesselCommand,
    ) -> Result<serde_json::Value> {
        let VesselCommand::PrepareTransfer {
            command_id,
            transfer_id,
            source_vessel_id,
            session_id,
            workspace,
            config_path,
            expires_at_ms,
        } = &command
        else {
            anyhow::bail!("not preparation")
        };
        ensure!(
            !command_id.is_nil() && !transfer_id.is_nil() && !session_id.is_nil(),
            "nil transfer identity"
        );
        let registrations = self.registrations.lock().await;
        registry::private_directory(&self.directory.join("transfers"))?;
        let path = directory(&self.directory, *transfer_id);
        registry::private_directory(&path)?;
        if registry::command_record(&self.directory, *command_id, &command, false)?
            && path.join("prepared.json").exists()
        {
            return Ok(serde_json::to_value(
                load(&self.directory, *transfer_id)?.preparation,
            )?);
        }
        ensure!(
            !path.join("prepared.json").exists(),
            "transfer ID already prepared"
        );
        ensure!(
            !registrations.contains_key(session_id),
            "destination already knows this session identity"
        );
        let public = identity::public(&self.directory)?;
        ensure!(
            public.vessel_id != *source_vessel_id,
            "source and destination Vessel must differ"
        );
        let pinned: VesselIdentity = store::load(
            &self
                .directory
                .join("trusted-vessels")
                .join(format!("{source_vessel_id}.json")),
        )?;
        ensure!(
            pinned.vessel_id == *source_vessel_id,
            "source identity is not locally pinned"
        );
        let now = store::now()?;
        ensure!(
            *expires_at_ms > now && *expires_at_ms - now <= 1800000,
            "preparation deadline must be within thirty minutes"
        );
        ensure!(
            registrations
                .values()
                .filter(|item| !matches!(
                    item.state,
                    ProcessState::Stopped | ProcessState::Relinquished
                ))
                .count()
                + self.reserved_transfers(None)?
                < self.capacity,
            "destination process capacity exhausted"
        );
        let workspace = std::fs::canonicalize(workspace)?;
        ensure!(workspace.is_dir(), "destination workspace missing");
        if let Some(config) = config_path {
            ensure!(
                config.is_absolute() && config.is_file(),
                "destination config must exist locally"
            );
        }
        let mut check = tokio::process::Command::new(&self.binary);
        check
            .arg("validate-start")
            .arg("--workspace")
            .arg(&workspace)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        if let Some(config) = config_path {
            check.arg("--config").arg(config);
        }
        let status =
            tokio::time::timeout(std::time::Duration::from_secs(10), check.status()).await??;
        ensure!(
            status.success(),
            "destination configuration or execution policy unavailable"
        );
        let preparation = identity::sign(
            &self.directory,
            TransferPreparation {
                transfer_id: *transfer_id,
                source_vessel_id: *source_vessel_id,
                destination_vessel_id: public.vessel_id,
                session_id: *session_id,
                nonce: *command_id,
                workspace,
                expires_at_ms: *expires_at_ms,
            },
        )?;
        registry::command_record(&self.directory, *command_id, &command, true)?;
        save(
            &self.directory,
            *transfer_id,
            &Prepared {
                preparation: preparation.clone(),
                config_path: config_path.clone(),
                manifest: None,
                activated: false,
            },
        )?;
        Ok(serde_json::to_value(preparation)?)
    }
}
