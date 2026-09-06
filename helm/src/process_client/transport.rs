//! Bounded Vessel requests. An I/O error never causes a command retry.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use voyage_protocol::process::{
    PROCESS_PROTOCOL, RuntimeCommand, RuntimeResponse, VesselCommand, VesselRequest,
    VesselResponse, read_frame, write_frame,
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

    #[cfg(unix)]
    async fn exchange(&self, command: VesselCommand) -> Result<Value> {
        if let Some(path) = &self.access_file {
            return super::access::exchange(path, command).await;
        }
        if let Some(destination) = &self.ssh {
            return super::ssh::exchange(destination, &self.directory, command).await;
        }
        super::local::check_private_directory(&self.directory)?;
        let mut socket = tokio::net::UnixStream::connect(self.directory.join("vessel.sock"))
            .await
            .context("Vessel unavailable; accepted voyages may still be running")?;
        // Filesystem permissions alone do not authenticate a substituted listener.
        ensure!(
            socket.peer_cred()?.uid() == unsafe { libc::geteuid() },
            "Vessel listener belongs to another user"
        );
        write_frame(
            &mut socket,
            &VesselRequest {
                protocol: PROCESS_PROTOCOL,
                command,
            },
        )
        .await?;
        let reply: VesselResponse = read_frame(&mut socket).await?;
        ensure!(
            reply.protocol == PROCESS_PROTOCOL,
            "unsupported Vessel protocol"
        );
        if let Some(error) = reply.error {
            if reply.outcome_unknown {
                anyhow::bail!("Vessel command outcome unknown: {}", super::safe(&error));
            }
            return Err(Refusal(format!("Vessel refused: {}", super::safe(&error))).into());
        }
        Ok(reply.result)
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
                && reply.incarnation == incarnation,
            "runtime response identity mismatch"
        );
        if let Some(error) = reply.error {
            if reply.outcome_unknown {
                anyhow::bail!("Voyage command outcome unknown: {}", super::safe(&error));
            }
            return Err(Refusal(format!("Voyage refused: {}", super::safe(&error))).into());
        }
        Ok(reply.result)
    }
}
