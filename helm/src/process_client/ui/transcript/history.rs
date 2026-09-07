//! Revision-bound public history; no direct journal reads or execution effects.
use super::super::{
    App,
    observe::Update,
    state::{Message, Target},
};
use crate::process_client::transport::Client;
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::process::RuntimeCommand;

pub(super) async fn load(
    client: &Client,
    target: Target,
    incarnation: Uuid,
    revision: u64,
    from: usize,
    total: usize,
) -> Result<Vec<Message>> {
    ensure!(
        total.saturating_sub(from) <= 16_384,
        "This history window is too large. Load a smaller section."
    );
    let mut output = Vec::new();
    let mut bytes = 0usize;
    let mut offset = from;
    while offset < total {
        let page = client
            .forward(
                target.session,
                incarnation,
                RuntimeCommand::History {
                    offset: offset as u64,
                    limit: 128,
                    expected_revision: Some(revision),
                },
            )
            .await?;
        ensure!(
            page["session_id"] == target.session.to_string()
                && page["revision"] == revision
                && page["message_offset"] == offset as u64,
            "Conversation changed while loading"
        );
        let mut messages: Vec<Message> = serde_json::from_value(page["messages"].clone())?;
        ensure!(!messages.is_empty(), "History did not advance");
        for message in &mut messages {
            ensure!(
                message.message_index == offset && offset < total,
                "History order changed"
            );
            if message.projection_truncated && message.role != "tool" {
                let mut encoded = String::new();
                loop {
                    let cursor = encoded.len() as u64;
                    let chunk = client
                        .forward(
                            target.session,
                            incarnation,
                            RuntimeCommand::MessageChunk {
                                index: offset as u64,
                                offset: cursor,
                                limit: 65536,
                                expected_revision: revision,
                            },
                        )
                        .await?;
                    ensure!(
                        chunk["session_id"] == target.session.to_string()
                            && chunk["revision"] == revision
                            && chunk["index"] == offset as u64
                            && chunk["offset"] == cursor,
                        "Message changed while loading"
                    );
                    let data = chunk["data"].as_str().context("Missing message text")?;
                    ensure!(
                        chunk["next_offset"].as_u64() == Some(cursor + data.len() as u64),
                        "Message cursor did not advance"
                    );
                    ensure!(
                        bytes + encoded.len() + data.len() <= 64 * 1024 * 1024,
                        "This history exceeds the 64 MiB reading limit. Export it to read the complete conversation."
                    );
                    encoded.push_str(data);
                    if chunk["has_more"] != true {
                        break;
                    }
                    ensure!(!data.is_empty(), "Message cursor did not advance");
                }
                *message = serde_json::from_str(&encoded)?;
                message.message_index = offset;
            }
            bytes += serde_json::to_vec(message)?.len();
            ensure!(
                bytes <= 64 * 1024 * 1024,
                "This history exceeds the 64 MiB reading limit. Export it to read the complete conversation."
            );
            offset += 1;
        }
        output.extend(messages);
    }
    // Close the revision fence even for an empty page/window.
    client
        .forward(
            target.session,
            incarnation,
            RuntimeCommand::History {
                offset: total as u64,
                limit: 1,
                expected_revision: Some(revision),
            },
        )
        .await?;
    Ok(output)
}

impl App {
    pub(in crate::process_client::ui) fn refresh_transcript(&mut self) {
        let Some(target) = self.selected else { return };
        let Some(view) = self.views.get(&target) else {
            return;
        };
        if view.process.archive.is_some() || view.deleted() {
            return;
        }
        let Some(snapshot) = &view.snapshot else {
            return;
        };
        let mut state = view.transcript.borrow_mut();
        if let Some(run) = &snapshot.run
            && run.live_text_truncated
            && let Some(offset) = run.live_text_offset
        {
            let key = (run.run_id, offset, run.partial_text_bytes);
            if !state.live_loading && state.live_attempt != Some(key) {
                state.live_loading = true;
                state.live_attempt = Some(key);
                let client = self.clients[target.route].clone();
                let incarnation = view.process.incarnation;
                let sender = self.sender.clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        live(&client, target, incarnation, key),
                    )
                    .await
                    .map_err(|_| "Live text loading timed out".to_owned())
                    .and_then(|r| {
                        r.map_err(|_| {
                            "Couldn’t load the rest of this response. Ctrl+Home retries.".to_owned()
                        })
                    });
                    let _ = sender
                        .send(Update::Live {
                            target,
                            incarnation,
                            run: key.0,
                            offset: key.1,
                            total: key.2,
                            result,
                        })
                        .await;
                });
            }
        }
        let needs = snapshot
            .messages
            .iter()
            .any(|m| m.projection_truncated && m.role != "tool")
            || state.requested_from.is_some();
        let revision = snapshot.revision;
        let from = state
            .requested_from
            .unwrap_or(snapshot.message_offset)
            .min(snapshot.message_offset);
        let covered = state.loaded_revision == Some(revision)
            && state
                .messages
                .first()
                .is_none_or(|message| message.message_index <= from);
        if !needs
            || state.loading
            || covered
            || (state.attempted == Some(revision) && state.attempted_from == Some(from))
        {
            return;
        }
        state.requested_from = Some(from);
        state.attempted = Some(revision);
        state.attempted_from = Some(from);
        state.loading = true;
        state.error = None;
        let total = snapshot.total_messages;
        let incarnation = view.process.incarnation;
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                load(&client, target, incarnation, revision, from, total),
            )
            .await
            .map_err(|_| "Loading took too long. Ctrl+Home retries.".to_owned())
            .and_then(|r| r.map_err(|_| "Couldn’t load this part of the conversation. Retry, or /export the saved conversation.".to_owned()));
            let _ = sender
                .send(Update::History {
                    target,
                    incarnation,
                    revision,
                    result,
                })
                .await;
        });
    }
}

async fn live(
    client: &Client,
    target: Target,
    incarnation: Uuid,
    (run_id, start, total): (Uuid, u64, u64),
) -> Result<String> {
    ensure!(
        total >= start && total - start <= 1024 * 1024,
        "Live text exceeds the reading limit"
    );
    let mut text = String::new();
    while start + (text.len() as u64) < total {
        let offset = start + text.len() as u64;
        let chunk = client
            .forward(
                target.session,
                incarnation,
                RuntimeCommand::RunOutput {
                    run_id,
                    offset,
                    limit: (total - offset).min(65536) as u32,
                },
            )
            .await?;
        ensure!(
            chunk["session_id"] == target.session.to_string()
                && chunk["run_id"] == run_id.to_string()
                && chunk["offset"] == offset,
            "Live text changed while loading"
        );
        let data = chunk["data"].as_str().context("Missing live text")?;
        ensure!(
            !data.is_empty()
                && offset + data.len() as u64 <= total
                && chunk["next_offset"].as_u64() == Some(offset + data.len() as u64),
            "Invalid live text cursor"
        );
        text.push_str(data);
    }
    Ok(text)
}
