//! Merge bounded metadata pages without exposing canonical transcript content.
use super::*;
use helm::process_client::transport::Client;
use voyage_protocol::process::{ProcessInfo, RuntimeCommand, VesselCommand};

pub(super) async fn list(
    client: &Client,
    directory: &std::path::Path,
    after: Option<Uuid>,
    limit: usize,
) -> Result<Value> {
    ensure!(
        (1..=100).contains(&limit),
        "managed list limit must be 1..100"
    );
    let entries: Vec<ProcessInfo> =
        serde_json::from_value(client.request(VesselCommand::Catalogue).await?)?;
    let mut rows = std::collections::BTreeMap::new();
    let mut legacy_more = false;
    if directory.join("journal/journal.sqlite3").exists() {
        let journal = Journal::open(directory.join("journal"))?;
        let page = journal.list_session_summaries(after, limit)?;
        legacy_more = page.next_after.is_some();
        for entry in page.sessions {
            rows.insert(entry.id, serde_json::to_value(entry)?);
        }
    }
    for process in entries
        .into_iter()
        .filter(|p| after.is_none_or(|after| p.session_id > after))
    {
        rows.insert(process.session_id, json!({"id":process.session_id,"state":process.state,"workspace":process.workspace,"incarnation":process.incarnation}));
    }
    let more = legacy_more || rows.len() > limit;
    let mut sessions = Vec::with_capacity(limit);
    for (id, mut row) in rows.into_iter().take(limit) {
        if let Some(incarnation) = row.get("incarnation") {
            let incarnation = serde_json::from_value(incarnation.clone())?;
            match client
                .forward(id, incarnation, RuntimeCommand::Snapshot)
                .await
            {
                Ok(snapshot) => {
                    for key in [
                        "revision",
                        "name",
                        "model",
                        "pending_cleanup_run",
                        "lifecycle",
                    ] {
                        row[key] = snapshot[key].clone();
                    }
                }
                Err(_) => row["observation"] = json!("unavailable"),
            }
        }
        sessions.push(row);
    }
    let next = more
        .then(|| sessions.last().map(|row| row["id"].clone()))
        .flatten();
    Ok(json!({"event":"session_list","sessions":sessions,"next_after":next}))
}
