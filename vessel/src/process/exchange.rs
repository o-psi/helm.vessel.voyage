use anyhow::{Result, ensure};
use std::{path::Path, time::Duration};
use tokio::net::UnixStream;
use voyage_protocol::process::*;

/// Trusted local adapter used by the authenticated HTTP gateway.
pub async fn exchange(directory: &Path, request: &VesselRequest) -> Result<VesselResponse> {
    super::registry::private_directory(directory)?;
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut stream = UnixStream::connect(directory.join("vessel.sock")).await?;
        ensure!(
            stream.peer_cred()?.uid() == unsafe { libc::geteuid() },
            "Vessel peer uid mismatch"
        );
        write_frame(&mut stream, request).await?;
        let response: VesselResponse = read_frame(&mut stream).await?;
        ensure!(
            response.protocol == PROCESS_PROTOCOL,
            "unsupported Vessel protocol"
        );
        Ok(response)
    })
    .await
    .map_err(|_| anyhow::anyhow!("Vessel response deadline exceeded; outcome unknown"))?
}
