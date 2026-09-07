//! Explicit SSH account access; independent of Vessel enrollment/session grants.
use super::transport::Refusal;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{path::Path, process::Stdio};
use voyage_protocol::vessel::{
    VESSEL_API_VERSION, VesselCommand, VesselRequest, VesselResponse, read_frame, write_frame,
};

pub async fn exchange(
    destination: &str,
    directory: &Path,
    command: VesselCommand,
) -> Result<Value> {
    ensure!(
        !destination.is_empty()
            && !destination.starts_with('-')
            && destination
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "@._-:[]".contains(ch)),
        "invalid SSH destination"
    );
    ensure!(
        directory.is_absolute(),
        "remote Vessel directory must be absolute"
    );
    let path = directory
        .to_str()
        .context("remote directory must be UTF-8")?;
    ensure!(
        !path.chars().any(char::is_control),
        "invalid remote directory"
    );
    let quoted = format!("'{}'", path.replace('\'', "'\\''"));
    let mut child = tokio::process::Command::new("ssh")
        .args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ForwardAgent=no",
            "-o",
            "ClearAllForwardings=yes",
            "-o",
            "ConnectTimeout=8",
            "--",
        ])
        .arg(destination)
        .arg(format!(
            "exec vessel local-request --api-version {VESSEL_API_VERSION} --directory {quoted}"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("launch SSH Vessel connection")?;
    let mut input = child.stdin.take().context("SSH input unavailable")?;
    let mut output = child.stdout.take().context("SSH output unavailable")?;
    write_frame(
        &mut input,
        &VesselRequest {
            protocol: VESSEL_API_VERSION,
            command,
        },
    )
    .await?;
    drop(input);
    let reply: VesselResponse = read_frame(&mut output).await?;
    ensure!(
        child.wait().await?.success(),
        "SSH connection failed; delivery may be unknown"
    );
    ensure!(
        reply.protocol == VESSEL_API_VERSION,
        "unsupported remote Vessel protocol"
    );
    if let Some(error) = reply.error {
        if reply.outcome_unknown {
            anyhow::bail!("Vessel command outcome unknown: {}", super::safe(&error));
        }
        return Err(Refusal(format!("Vessel refused: {}", super::safe(&error))).into());
    }
    Ok(reply.result)
}
