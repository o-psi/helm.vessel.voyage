//! Merge bounded metadata pages without exposing canonical transcript content.
use super::*;
use helm::process_client::transport::Client;
use voyage_protocol::vessel::{ProcessInfo, VesselCommand};

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
        let mut row = json!({"id":process.session_id,"state":process.state,"workspace":process.workspace,"incarnation":process.incarnation,"name":process.name});
        if let Some(metadata) = process.catalogue {
            row["observation"] = json!(if metadata.stale { "stale" } else { "catalogue" });
            if let Some(summary) = metadata.summary {
                row["revision"] = json!(summary.revision);
                row["model"] = json!(summary.model);
                row["pending_cleanup_run"] = json!(summary.pending_cleanup_run);
                row["lifecycle"] = json!({"archived":summary.archived,"deleted":summary.deleted});
            }
        }
        rows.insert(process.session_id, row);
    }
    let more = legacy_more || rows.len() > limit;
    let mut sessions = Vec::with_capacity(limit);
    for (_, row) in rows.into_iter().take(limit) {
        sessions.push(row);
    }
    let next = more
        .then(|| sessions.last().map(|row| row["id"].clone()))
        .flatten();
    Ok(json!({"event":"session_list","sessions":sessions,"next_after":next}))
}
