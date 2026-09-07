//! Bounded Vessel requests. An I/O error never causes a command retry.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use voyage_protocol::process::{
    PROCESS_PROTOCOL, ProcessInfo, RuntimeCommand, RuntimeResponse, VesselCommand, VesselEvent,
    VesselEventRequest, VesselEventSubscription,
};

#[derive(Clone, Debug)]
pub struct Client {
    pub directory: PathBuf,
    pub ssh: Option<String>,
    pub access_file: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Refusal(pub String);

impl Client {
    pub fn is_local(&self) -> bool {
        self.ssh.is_none() && self.access_file.is_none()
    }
    pub fn label(&self) -> String {
        self.ssh.clone().unwrap_or_else(|| {
            self.access_file
                .as_ref()
                .map(|p| {
                    format!(
                        "grant:{}",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    )
                })
                .unwrap_or_else(|| "local".into())
        })
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
            protocol: PROCESS_PROTOCOL,
            subscriptions,
        };
        if let Some(path) = &self.access_file {
            return super::access::events(path, request).await;
        }
        ensure!(
            self.ssh.is_none(),
            "SSE is unavailable through the SSH compatibility route"
        );
        super::local::events(&self.directory, request).await
    }

    pub async fn supports_events(&self) -> bool {
        if self.ssh.is_some() {
            return false;
        }
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
        if let Some(destination) = &self.ssh {
            return super::ssh::exchange(destination, &self.directory, command).await;
        }
        super::local::exchange(&self.directory, command).await
    }

    #[cfg(not(unix))]
    async fn exchange(&self, _command: VesselCommand) -> Result<Value> {
        anyhow::bail!("private Vessel client transport is not implemented on this platform")
    }

    pub async fn forward(
        &self,
        session_id: uuid::Uuid,
        incarnation: uuid::Uuid,
        command: RuntimeCommand,
    ) -> Result<Value> {
        Ok(self
            .forward_observed(session_id, incarnation, command)
            .await?
            .0)
    }

    pub(super) async fn forward_observed(
        &self,
        session_id: uuid::Uuid,
        incarnation: uuid::Uuid,
        command: RuntimeCommand,
    ) -> Result<(Value, uuid::Uuid)> {
        // Session reads and new work follow clean resumes. Commands addressing a
        // live resource or decision must retain the incarnation the caller saw.
        let process: ProcessInfo =
            serde_json::from_value(self.request(VesselCommand::Inspect { session_id }).await?)?;
        ensure!(
            process.session_id == session_id,
            "process identity mismatch"
        );
        let exact_owner = matches!(
            &command,
            RuntimeCommand::ExecuteTool { .. }
                | RuntimeCommand::Terminal { .. }
                | RuntimeCommand::Cancel { .. }
                | RuntimeCommand::Steer { .. }
                | RuntimeCommand::Respond { .. }
                | RuntimeCommand::Stop
        );
        if exact_owner && process.incarnation != incarnation {
            return Err(
                Refusal("Voyage changed; refresh before acting on its resources".into()).into(),
            );
        }
        let incarnation = process.incarnation;
        let reply: RuntimeResponse = serde_json::from_value(
            self.request(VesselCommand::Forward {
                session_id,
                incarnation,
                command,
            })
            .await?,
        )?;
        ensure!(
            reply.protocol == PROCESS_PROTOCOL
                && reply.session_id == session_id
                && (reply.incarnation == incarnation
                    || (!exact_owner && reply.resumed_from == Some(incarnation))),
            "runtime response identity mismatch"
        );
        if let Some(error) = reply.error {
            if reply.outcome_unknown {
                anyhow::bail!("Voyage command outcome unknown: {}", super::safe(&error));
            }
            return Err(Refusal(format!("Voyage refused: {}", super::safe(&error))).into());
        }
        Ok((reply.result, reply.incarnation))
    }
}
