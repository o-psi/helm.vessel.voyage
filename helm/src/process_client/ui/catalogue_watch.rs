//! Catalogue observation has its own task, so a slow metadata read cannot hold
//! transcript events or their acknowledgements. Dropping the UI aborts reads.
use crate::process_client::transport::Client;
use std::time::Duration;
use tokio::sync::watch;
use voyage_protocol::vessel::{CatalogueChanges, ProcessInfo, VesselCommand};

type State = Option<Result<Vec<ProcessInfo>, String>>;
pub(super) struct Observer(tokio::task::JoinHandle<()>);
impl Drop for Observer {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub(super) fn spawn(client: Client) -> (Observer, watch::Receiver<State>) {
    let (sender, receiver) = watch::channel(None);
    let task = tokio::spawn(async move {
        loop {
            let result = run(&client, &sender).await;
            if sender.is_closed() {
                return;
            }
            if let Err(error) = result
                && sender.send(Some(Err(error.to_string()))).is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    (Observer(task), receiver)
}
async fn catalogue(client: &Client) -> anyhow::Result<Vec<ProcessInfo>> {
    let value = tokio::time::timeout(
        Duration::from_secs(3),
        client.request(VesselCommand::Catalogue),
    )
    .await??;
    Ok(serde_json::from_value(value)?)
}
async fn changes(
    client: &Client,
    after: Option<u64>,
    wait_ms: u32,
) -> anyhow::Result<CatalogueChanges> {
    let page: CatalogueChanges = serde_json::from_value(
        client
            .request(VesselCommand::CatalogueChanges {
                after,
                limit: 128,
                wait_ms,
            })
            .await?,
    )?;
    anyhow::ensure!(
        page.cursor <= page.latest_cursor
            && page.has_more == (page.cursor < page.latest_cursor)
            && page.entries.len() <= 128
            && (!page.replay_gap || (page.entries.is_empty() && !page.has_more))
            && (after.is_some() || page.replay_gap)
            && (page.replay_gap
                || after != Some(page.cursor)
                || (page.entries.is_empty() && !page.has_more))
            && page
                .entries
                .iter()
                .map(|entry| entry.session_id)
                .collect::<std::collections::HashSet<_>>()
                .len()
                == page.entries.len()
            && (page.replay_gap || after.is_none_or(|cursor| page.cursor >= cursor)),
        "invalid catalogue change page"
    );
    Ok(page)
}
async fn run(client: &Client, sender: &watch::Sender<State>) -> anyhow::Result<()> {
    let caps = client.request(VesselCommand::Capabilities).await?;
    let feed = caps["features"].as_array().is_some_and(|features| {
        features
            .iter()
            .any(|feature| feature == "catalogue_changes")
    });
    // Capture the cursor BEFORE hydration. Changes racing the full read remain
    // observable and are coalesced to their current projection on the next page.
    let mut cursor = if feed {
        changes(client, None, 0).await?.cursor
    } else {
        0
    };
    let mut entries = catalogue(client).await?;
    loop {
        if sender.send(Some(Ok(entries.clone()))).is_err() {
            return Ok(());
        }
        if !feed {
            tokio::time::sleep(Duration::from_secs(5)).await;
            entries = catalogue(client).await?;
            continue;
        }
        loop {
            let page = changes(client, Some(cursor), 8_000).await?;
            let previous_cursor = cursor;
            cursor = page.cursor;
            if page.replay_gap {
                entries = catalogue(client).await?;
                break;
            }
            let changed = !page.entries.is_empty();
            for entry in page.entries {
                if let Some(existing) = entries
                    .iter_mut()
                    .find(|existing| existing.session_id == entry.session_id)
                {
                    *existing = entry;
                } else {
                    entries.push(entry);
                }
            }
            anyhow::ensure!(entries.len() <= 4096, "catalogue exceeds retention bound");
            if changed {
                entries.sort_by_key(|entry| entry.session_id);
                break;
            }
            // Scoped pages can contain only invisible changes. Advance their
            // cursor without inventing entries or stalling behind filtered rows.
            if cursor == previous_cursor {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            if sender.is_closed() {
                return Ok(());
            }
        }
    }
}
