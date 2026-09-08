//! Bounded Vessel requests. An I/O error never causes a command retry.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use voyage_protocol::vessel::{
    VESSEL_API_VERSION, VesselCommand, VesselEvent, VesselEventRequest, VesselEventSubscription,
    VoyageCommand, VoyageReply, VoyageRequest,
};

#[derive(Clone, Debug)]
pub struct Client {
    pub directory: PathBuf,
    pub access_file: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Refusal(pub String);

impl Client {
    pub fn is_local(&self) -> bool {
        self.access_file.is_none()
    }
    pub fn label(&self) -> String {
        self.access_file
            .as_ref()
            .map(|p| {
                format!(
                    "grant:{}",
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
            })
            .unwrap_or_else(|| "local".into())
    }
    pub async fn request(&self, command: VesselCommand) -> Result<Value> {
        tokio::time::timeout(Duration::from_secs(15), self.exchange(command))
            .await
            .context("Vessel response deadline elapsed; command delivery may be unknown")?
    }

    pub async fn events(
        &self,
        subscriptions: Vec<VesselEventSubscription>,
    ) -> Result<futures_util::stream::BoxStream<'static, Result<VesselEvent>>> {
        let request = VesselEventRequest {
            protocol: VESSEL_API_VERSION,
            subscriptions,
        };
        if let Some(path) = &self.access_file {
            return super::access::events(path, request).await;
        }
        super::local::events(&self.directory, request).await
    }

    pub async fn supports_events(&self) -> bool {
        self.request(VesselCommand::Capabilities)
            .await
            .ok()
            .and_then(|value| value.get("features").and_then(Value::as_array).cloned())
            .is_some_and(|features| features.iter().any(|feature| feature == "sse_events"))
    }

    #[cfg(unix)]
    async fn exchange(&self, command: VesselCommand) -> Result<Value> {
        if let Some(path) = &self.access_file {
            return super::access::exchange(path, command).await;
        }
        super::local::exchange(&self.directory, command).await
    }

    #[cfg(not(unix))]
    async fn exchange(&self, _command: VesselCommand) -> Result<Value> {
        anyhow::bail!("private Vessel client transport is not implemented on this platform")
    }

    pub async fn voyage(
        &self,
        session_id: uuid::Uuid,
        incarnation: uuid::Uuid,
        command: VoyageCommand,
    ) -> Result<Value> {
        Ok(self
            .voyage_observed(session_id, incarnation, command)
            .await?
            .0)
    }

    pub(super) async fn voyage_observed(
        &self,
        session_id: uuid::Uuid,
        incarnation: uuid::Uuid,
        command: VoyageCommand,
    ) -> Result<(Value, uuid::Uuid)> {
        let exact_owner = command.requires_incarnation();
        let reply: VoyageReply = serde_json::from_value(
            self.request(VesselCommand::Voyage(VoyageRequest {
                session_id,
                incarnation: exact_owner.then_some(incarnation),
                command,
            }))
            .await?,
        )?;
        ensure!(
            reply.session_id == session_id && (!exact_owner || reply.incarnation == incarnation),
            "Vessel response identity mismatch"
        );
        Ok((reply.result, reply.incarnation))
    }
}
