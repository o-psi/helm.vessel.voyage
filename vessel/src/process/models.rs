//! Owner-local model discovery is a short-lived child, never a registered voyage.
use super::service::Supervisor;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{path::PathBuf, process::Stdio, time::Duration};
use voyage_protocol::process::{read_frame, write_frame};
use voyage_protocol::vessel::VesselCommand;

impl Supervisor {
    pub(super) async fn discover_models(
        &self,
        workspace: PathBuf,
        configuration: Value,
    ) -> Result<Value> {
        ensure!(workspace.is_absolute(), "model workspace must be absolute");
        ensure!(
            serde_json::to_vec(&configuration)?.len() <= 1024 * 1024,
            "model configuration exceeds limit"
        );
        let permit = self
            .model_slots
            .clone()
            .try_acquire_owned()
            .context("model discovery capacity reached")?;
        let binary = self.binary.clone();
        // The bounded task owns its permit, quota and child even if the HTTP caller
        // disconnects. It always waits for exit before releasing the host charge.
        tokio::spawn(async move {
            let _permit = permit;
            let reservation = voyage_runtime::host_resources::Reservation::acquire(
                "executors",
                uuid::Uuid::new_v4(),
                1,
            )?;
            let child = tokio::process::Command::new(binary)
                .arg("discover-models")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn();
            let mut child = match child {
                Ok(child) => child,
                Err(_) => {
                    reservation.release_observed()?;
                    anyhow::bail!("model discovery process could not start");
                }
            };
            let result = tokio::time::timeout(Duration::from_secs(13), async {
                let mut input = child
                    .stdin
                    .take()
                    .context("model discovery input unavailable")?;
                write_frame(
                    &mut input,
                    &VesselCommand::DiscoverModels {
                        workspace,
                        configuration,
                    },
                )
                .await?;
                drop(input);
                let models: Value = read_frame(
                    &mut child
                        .stdout
                        .take()
                        .context("model discovery output unavailable")?,
                )
                .await?;
                ensure!(
                    models.is_array() && serde_json::to_vec(&models)?.len() <= 1024 * 1024,
                    "invalid model discovery response"
                );
                ensure!(child.wait().await?.success(), "model discovery failed");
                Ok::<_, anyhow::Error>(models)
            })
            .await;
            // Includes framing failures and malformed or over-limit output, not
            // just timeouts. Killing is requested cleanup; wait proves child exit.
            tokio::time::timeout(Duration::from_secs(2), async {
                if child.try_wait()?.is_none() {
                    child.kill().await?;
                }
                child.wait().await?;
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context("model discovery cleanup unconfirmed")??;
            reservation.release_observed()?;
            match result {
                Ok(Ok(models)) => Ok(models),
                _ => anyhow::bail!("model discovery failed or timed out; no voyage was created"),
            }
        })
        .await
        .context("model discovery supervisor task failed")?
    }
}
