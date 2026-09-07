//! Publish a complete revision-bound public transcript through the owner protocol.
use super::transport::Client;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{io::Write, path::Path};
use uuid::Uuid;
use voyage_protocol::vessel::{ProcessInfo, VesselCommand, VoyageCommand};

pub async fn markdown(client: &Client, session: Uuid, destination: &Path) -> Result<Value> {
    let process: ProcessInfo = serde_json::from_value(
        client
            .request(VesselCommand::Inspect {
                session_id: session,
            })
            .await?,
    )?;
    let snapshot = client
        .voyage(session, process.incarnation, VoyageCommand::Snapshot)
        .await?;
    let revision = snapshot["revision"]
        .as_u64()
        .context("snapshot revision missing")?;
    let total = snapshot["total_messages"]
        .as_u64()
        .context("message count missing")?;
    ensure!(total <= 100_000, "export message bound exceeded");
    let absolute = if destination.is_absolute() {
        destination.to_owned()
    } else {
        std::env::current_dir()?.join(destination)
    };
    let parent = absolute.parent().context("export directory missing")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    writeln!(file, "# Voyage {session}\n\nRevision: {revision}\n")?;
    let mut bytes = 0usize;
    for index in 0..total {
        let mut encoded = String::new();
        loop {
            let offset = encoded.len() as u64;
            let chunk = client
                .voyage(
                    session,
                    process.incarnation,
                    VoyageCommand::MessageChunk {
                        index,
                        offset,
                        limit: 65536,
                        expected_revision: revision,
                    },
                )
                .await?;
            ensure!(
                chunk["session_id"] == json!(session)
                    && chunk["revision"] == revision
                    && chunk["index"] == index
                    && chunk["offset"] == offset,
                "export identity changed"
            );
            let data = chunk["data"].as_str().context("message chunk missing")?;
            ensure!(
                chunk["next_offset"].as_u64() == Some(offset + data.len() as u64),
                "invalid message cursor"
            );
            bytes = bytes
                .checked_add(data.len())
                .context("export size overflow")?;
            ensure!(
                bytes <= 64 * 1024 * 1024,
                "public transcript exceeds 64 MiB export bound"
            );
            encoded.push_str(data);
            if chunk["has_more"] != true {
                break;
            }
            ensure!(!data.is_empty(), "message cursor did not advance");
        }
        let message: Value = serde_json::from_str(&encoded)?;
        writeln!(
            file,
            "## {}\n\n{}\n",
            super::safe(message["role"].as_str().context("message role missing")?),
            message["content"].as_str().unwrap_or("")
        )?;
        if let Some(calls) = message.get("tool_calls").filter(|value| !value.is_null()) {
            writeln!(
                file,
                "```json\n{}\n```\n",
                serde_json::to_string_pretty(calls)?
            )?;
        }
    }
    // Empty transcripts also receive a final revision check before publication.
    client
        .voyage(
            session,
            process.incarnation,
            VoyageCommand::History {
                offset: total,
                limit: 1,
                expected_revision: Some(revision),
            },
        )
        .await?;
    file.as_file().sync_all()?;
    file.persist_noclobber(&absolute)
        .map_err(|error| error.error)
        .context("export destination must not already exist")?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(json!({"session_id":session,"revision":revision,"messages":total,"path":absolute}))
}
