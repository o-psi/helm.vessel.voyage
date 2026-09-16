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
            "## {}\n",
            super::safe(message["role"].as_str().context("message role missing")?)
        )?;
        let parts: Vec<voyage_protocol::content::ContentPart> = message
            .get("parts")
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .map_err(|_| anyhow::anyhow!("Invalid image metadata in transcript"))?
            .unwrap_or_default();
        if parts.is_empty() {
            writeln!(file, "{}\n", message["content"].as_str().unwrap_or(""))?;
        } else {
            for part in parts {
                match part {
                    voyage_protocol::content::ContentPart::Text { text } => write!(file, "{text}")?,
                    voyage_protocol::content::ContentPart::Image { attachment } => {
                        // Export metadata only. Never resolve or embed the private blob.
                        writeln!(
                            file,
                            "\n[Image {}: {} · {:?} · {} bytes · {}×{} · SHA-256 {}]\n",
                            attachment.id,
                            super::safe(&attachment.name),
                            attachment.media_type,
                            attachment.byte_size,
                            attachment.width,
                            attachment.height,
                            attachment.sha256
                        )?;
                    }
                }
            }
            writeln!(file)?;
        }
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

/// Fetch complete canonical public assistant text, never a rendered/history
/// projection. Revision changes fail closed; no partial copy is returned.
pub async fn response_text(
    client: &Client,
    session: Uuid,
    incarnation: Uuid,
    revision: u64,
    index: u64,
) -> Result<String> {
    let mut encoded = String::new();
    loop {
        let offset = encoded.len() as u64;
        let chunk = client
            .voyage(
                session,
                incarnation,
                VoyageCommand::MessageChunk {
                    index,
                    offset,
                    limit: 65536,
                    expected_revision: revision,
                },
            )
            .await?;
        let more = append_response_chunk(&mut encoded, &chunk, session, revision, index)?;
        if !more {
            break;
        }
    }
    canonical_response(&serde_json::from_str(&encoded)?)
}

const COPY_LIMIT: usize = 1024 * 1024;
fn append_response_chunk(
    encoded: &mut String,
    chunk: &Value,
    session: Uuid,
    revision: u64,
    index: u64,
) -> Result<bool> {
    ensure!(
        chunk["session_id"] == json!(session)
            && chunk["revision"] == revision
            && chunk["index"] == index
            && chunk["offset"] == encoded.len() as u64
            && chunk["encoding"] == "public_message_json_utf8",
        "Response identity changed; copy cancelled"
    );
    let total = chunk["total_bytes"]
        .as_u64()
        .context("Response size missing")?;
    ensure!(
        total <= COPY_LIMIT as u64,
        "Response exceeds 1 MiB copy bound; use transcript export"
    );
    let data = chunk["data"].as_str().context("Response text missing")?;
    let end = encoded
        .len()
        .checked_add(data.len())
        .context("Response size overflow")?;
    let more = chunk["has_more"]
        .as_bool()
        .context("Response continuation missing")?;
    ensure!(
        end <= COPY_LIMIT
            && end as u64 <= total
            && chunk["next_offset"] == end as u64
            && (if more {
                !data.is_empty() && (end as u64) < total
            } else {
                end as u64 == total
            }),
        "Invalid response continuation; copy cancelled"
    );
    encoded.push_str(data);
    Ok(more)
}

fn canonical_response(message: &Value) -> Result<String> {
    ensure!(
        message["role"] == "assistant"
            && !message["projection_truncated"].as_bool().unwrap_or(false),
        "Select a complete assistant response"
    );
    let text = message["content"]
        .as_str()
        .context("Canonical response text unavailable")?;
    ensure!(
        !text.is_empty() && text.len() <= COPY_LIMIT,
        "Response is empty or too large to copy"
    );
    Ok(text.into())
}

/// An explicit human copy request only. Base64 prevents canonical controls from
/// becoming terminal instructions. OSC52 has no portable acknowledgment: UI must
/// say "copy requested", not "clipboard verified". Never use it automatically.
pub fn response_clipboard_sequence(text: &str) -> Result<String> {
    use base64::Engine;
    ensure!(
        !text.is_empty() && text.len() <= COPY_LIMIT,
        "Response is empty or too large to copy"
    );
    Ok(format!(
        "\x1b]52;c;{}\x07",
        base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
    ))
}

#[cfg(test)]
mod response_tests {
    use super::*;
    #[test]
    fn copy_preserves_canonical_markdown_unicode_and_controls_not_rendering() {
        let text = "**Hello**\n\t世界\u{1b}[31m";
        assert_eq!(
            canonical_response(&json!({"role":"assistant","content":text})).unwrap(),
            text
        );
        assert!(canonical_response(&json!({"role":"tool","content":text})).is_err());
        assert!(
            canonical_response(
                &json!({"role":"assistant","content":text,"projection_truncated":true})
            )
            .is_err()
        );
        let seq = response_clipboard_sequence(text).unwrap();
        assert!(!seq.contains("Hello"));
        use base64::Engine;
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(
                    seq.strip_prefix("\x1b]52;c;")
                        .unwrap()
                        .strip_suffix('\x07')
                        .unwrap()
                )
                .unwrap(),
            text.as_bytes()
        );
    }
    #[test]
    fn stale_remote_or_incomplete_response_never_returns_partial_copy() {
        let session = Uuid::new_v4();
        let mut encoded = String::new();
        let chunk = json!({"session_id":session,"revision":7,"index":2,"offset":0,"next_offset":2,"total_bytes":4,"data":"é","has_more":true,"encoding":"public_message_json_utf8"});
        assert!(append_response_chunk(&mut encoded, &chunk, session, 8, 2).is_err());
        assert!(encoded.is_empty());
        assert!(append_response_chunk(&mut encoded, &chunk, session, 7, 2).unwrap());
        assert!(append_response_chunk(&mut encoded, &chunk, session, 7, 2).is_err());
        assert_eq!(encoded, "é");
    }
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod coverage_tests;
