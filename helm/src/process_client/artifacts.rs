//! Download immutable tool bytes explicitly; never open or execute downloaded files.
use super::transport::Client;
use anyhow::{Context, Result, ensure};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};
use uuid::Uuid;
use voyage_protocol::{
    tool_result::ArtifactReference,
    vessel::{ProcessInfo, VesselCommand, VoyageCommand},
};

pub async fn download(
    client: &Client,
    session: Uuid,
    artifact: Uuid,
    destination: &Path,
) -> Result<Value> {
    ensure!(
        !session.is_nil() && !artifact.is_nil(),
        "invalid artifact identity"
    );
    let process: ProcessInfo = serde_json::from_value(
        client
            .request(VesselCommand::Inspect {
                session_id: session,
            })
            .await?,
    )?;
    let absolute = if destination.is_absolute() {
        destination.to_owned()
    } else {
        std::env::current_dir()?.join(destination)
    };
    let parent = absolute
        .parent()
        .context("artifact destination directory missing")?;
    // NamedTempFile creates an owner-only file on Unix. Publication never clobbers
    // an existing file or follows a destination symlink.
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    let mut metadata: Option<ArtifactReference> = None;
    let mut offset = 0u64;
    let mut digest = Sha256::new();
    loop {
        let chunk = client
            .voyage(
                session,
                process.incarnation,
                VoyageCommand::ReadArtifact {
                    artifact_id: artifact,
                    offset,
                    limit: 65536,
                },
            )
            .await?;
        let current: ArtifactReference = serde_json::from_value(chunk["metadata"].clone())
            .map_err(|_| anyhow::anyhow!("invalid artifact metadata"))?;
        ensure!(
            current.id == artifact
                && (1..=4 * 1024 * 1024).contains(&current.byte_size)
                && current.sha256.len() == 64
                && current
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid artifact identity or byte bound"
        );
        if let Some(expected) = &metadata {
            ensure!(
                &current == expected,
                "artifact metadata changed during download"
            );
        }
        let encoded = chunk["data_base64"]
            .as_str()
            .context("artifact chunk missing")?;
        ensure!(
            encoded.len() <= 65536usize.div_ceil(3) * 4,
            "artifact chunk exceeds byte bound"
        );
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| anyhow::anyhow!("invalid artifact chunk encoding"))?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= 65536,
            "invalid artifact chunk length"
        );
        let next = offset
            .checked_add(bytes.len() as u64)
            .context("artifact cursor overflow")?;
        ensure!(
            chunk["offset"].as_u64() == Some(offset)
                && chunk["next_offset"].as_u64() == Some(next)
                && next <= current.byte_size,
            "invalid artifact cursor"
        );
        let eof = chunk["eof"]
            .as_bool()
            .context("artifact completion flag missing")?;
        ensure!(
            eof == (next == current.byte_size),
            "invalid artifact completion boundary"
        );
        file.write_all(&bytes)?;
        digest.update(&bytes);
        offset = next;
        metadata = Some(current);
        if eof {
            break;
        }
    }
    let metadata = metadata.context("artifact metadata missing")?;
    ensure!(
        hex::encode(digest.finalize()) == metadata.sha256,
        "artifact integrity verification failed"
    );
    file.as_file().sync_all()?;
    file.persist_noclobber(&absolute)
        .map_err(|error| error.error)
        .context("artifact destination must not already exist")?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(
        json!({"session_id": session, "artifact_id": artifact, "byte_size": offset,
        "sha256": metadata.sha256, "path": absolute}),
    )
}
