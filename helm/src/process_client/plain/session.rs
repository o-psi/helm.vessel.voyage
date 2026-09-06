use crate::process_client::transport::Client;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use uuid::Uuid;
use voyage_protocol::process::{ProcessInfo, RuntimeCommand, VesselCommand};

pub(super) struct Connection<'a> {
    pub client: &'a Client,
    pub process: ProcessInfo,
}

pub(super) fn deadline() -> Result<u64> {
    Ok(u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?
    .saturating_add(60_000))
}

impl<'a> Connection<'a> {
    pub async fn open(client: &'a Client, session_id: Uuid) -> Result<Self> {
        let process = serde_json::from_value(
            client
                .request(VesselCommand::Inspect { session_id })
                .await?,
        )?;
        Ok(Self { client, process })
    }
    pub async fn forward(&self, command: RuntimeCommand) -> Result<Value> {
        self.client
            .forward(self.process.session_id, self.process.incarnation, command)
            .await
    }
    pub async fn snapshot(&self) -> Result<Value> {
        self.forward(RuntimeCommand::Snapshot).await
    }
    pub async fn submit(&self, command_id: Option<Uuid>, prompt: String) -> Result<Uuid> {
        let snapshot = self.snapshot().await?;
        self.admit(
            &snapshot,
            command_id.unwrap_or_else(Uuid::new_v4),
            prompt,
            false,
        )
        .await
    }
    pub async fn send(&self, snapshot: &Value, prompt: String) -> Result<Uuid> {
        self.admit(snapshot, Uuid::new_v4(), prompt, true).await
    }
    async fn admit(
        &self,
        snapshot: &Value,
        command_id: Uuid,
        prompt: String,
        steer: bool,
    ) -> Result<Uuid> {
        ensure!(
            !prompt.trim().is_empty() && prompt.len() <= 65536,
            "prompt must contain 1..65536 UTF-8 bytes"
        );
        let expected_revision = snapshot["revision"]
            .as_u64()
            .context("snapshot revision missing")?;
        let expires_at_ms = deadline()?;
        let active = matches!(
            snapshot["run"]["state"].as_str(),
            Some("accepted" | "running" | "awaiting_decision" | "cancel_requested")
        );
        let command = if active && steer {
            RuntimeCommand::Steer {
                command_id,
                expected_revision,
                expires_at_ms,
                run_id: serde_json::from_value(snapshot["run"]["run_id"].clone())?,
                prompt,
            }
        } else {
            ensure!(
                !active,
                "voyage already has an active run; use chat to steer it"
            );
            RuntimeCommand::Submit {
                command_id,
                expected_revision,
                expires_at_ms,
                prompt,
            }
        };
        eprintln!(
            "Voyage {} · command {command_id}; retrieve this exact receipt if delivery is lost",
            self.process.session_id
        );
        let receipt = self.forward(command).await?;
        ensure!(
            receipt["status"] != "rejected",
            "command rejected: {}",
            crate::process_client::safe(&receipt.to_string())
        );
        let id = receipt
            .get("run_id")
            .or_else(|| {
                receipt
                    .get("record")
                    .and_then(|record| record.get("run_id"))
            })
            .context("accepted receipt omitted run identity")?;
        Ok(serde_json::from_value(id.clone())?)
    }
}
