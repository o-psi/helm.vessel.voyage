use crate::process_client::transport::Client;
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde_json::Value;
use uuid::Uuid;
use voyage_protocol::vessel::{
    ProcessInfo, VesselCommand, VesselEvent, VesselEventSubscription, VoyageCommand,
};

pub(super) struct Connection<'a> {
    pub client: &'a Client,
    pub process: ProcessInfo,
    observed_incarnation: std::cell::Cell<Uuid>,
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
        let process: ProcessInfo = serde_json::from_value(
            client
                .request(VesselCommand::Inspect { session_id })
                .await?,
        )?;
        Ok(Self {
            client,
            observed_incarnation: std::cell::Cell::new(process.incarnation),
            process,
        })
    }
    pub async fn voyage(&self, command: VoyageCommand) -> Result<Value> {
        let (value, incarnation) = self
            .client
            .voyage_observed(
                self.process.session_id,
                self.observed_incarnation.get(),
                command,
            )
            .await?;
        self.observed_incarnation.set(incarnation);
        Ok(value)
    }
    pub async fn snapshot(&self) -> Result<Value> {
        self.voyage(VoyageCommand::Snapshot).await
    }
    pub async fn event_stream(
        &self,
        after: u64,
    ) -> Result<Option<futures_util::stream::BoxStream<'static, Result<VesselEvent>>>> {
        if !self.client.supports_events().await {
            return Ok(None);
        }
        Ok(Some(
            self.client
                .events(vec![VesselEventSubscription {
                    session_id: self.process.session_id,
                    incarnation: self.observed_incarnation.get(),
                    after,
                }])
                .await?,
        ))
    }
    async fn wait_event(
        &self,
        stream: &mut futures_util::stream::BoxStream<'static, Result<VesselEvent>>,
    ) -> Result<()> {
        let event = stream.next().await.context("Vessel event stream ended")??;
        ensure!(
            event.session_id == self.process.session_id,
            "Vessel event stream identity mismatch"
        );
        if let Some(error) = event.error {
            if event.outcome_unknown {
                anyhow::bail!(
                    "Vessel event stream outcome unknown: {}",
                    crate::process_client::safe(&error)
                );
            }
            anyhow::bail!(
                "Vessel event stream refused: {}",
                crate::process_client::safe(&error)
            );
        }
        self.observed_incarnation.set(event.incarnation);
        Ok(())
    }
    pub async fn wait_update(
        &self,
        stream: &mut Option<futures_util::stream::BoxStream<'static, Result<VesselEvent>>>,
    ) -> Result<()> {
        let Some(events) = stream else {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            return Ok(());
        };
        if self.wait_event(events).await.is_ok() {
            return Ok(());
        }
        // A completed turn may suspend immediately after committing its final
        // invalidation. Refreshing is observational and updates the incarnation;
        // reconnect from that snapshot cursor without repeating any command.
        let snapshot = self.snapshot().await?;
        let cursor = snapshot["observation_cursor"]
            .as_u64()
            .context("snapshot observation cursor missing")?;
        *stream = self.event_stream(cursor).await?;
        Ok(())
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
            VoyageCommand::Steer {
                coordination: None,
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
            VoyageCommand::Submit {
                coordination: None,
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
        let receipt = self.voyage(command).await?;
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
