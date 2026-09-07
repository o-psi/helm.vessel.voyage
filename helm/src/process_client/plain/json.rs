//! Bounded machine-readable observation; never resubmits an admitted command.
use super::{VoyageCommand, session::Connection};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use uuid::Uuid;

pub(super) async fn follow(connection: &Connection<'_>, run_id: Uuid) -> Result<()> {
    let mut offset = 0u64;
    let mut shown = std::collections::BTreeSet::new();
    let snapshot = connection.snapshot().await?;
    let cursor = snapshot["observation_cursor"]
        .as_u64()
        .context("snapshot observation cursor missing")?;
    let mut events = connection.event_stream(cursor).await?;
    loop {
        let output = connection
            .voyage(VoyageCommand::RunOutput {
                run_id,
                offset,
                limit: 65536,
            })
            .await?;
        ensure!(
            output["run_id"] == json!(run_id) && output["offset"].as_u64() == Some(offset),
            "output identity or offset mismatch"
        );
        let data = output["data"].as_str().context("output data missing")?;
        let next = output["next_offset"]
            .as_u64()
            .context("output cursor missing")?;
        ensure!(
            next == offset.saturating_add(data.len() as u64),
            "invalid output cursor"
        );
        if !data.is_empty() {
            println!(
                "{}",
                json!({"event":"output","run_id":run_id,"offset":offset,"data":data,"next_offset":next})
            );
        }
        offset = next;
        let state = output["state"].as_str().context("run state missing")?;
        if output["has_more"] != true
            && !matches!(
                state,
                "accepted" | "running" | "awaiting_decision" | "cancel_requested"
            )
        {
            println!(
                "{}",
                json!({"event":"run_finished","run_id":run_id,"state":state})
            );
            ensure!(state == "completed", "run ended with state {state}");
            return Ok(());
        }
        let decisions = connection.voyage(VoyageCommand::Decisions).await?;
        for decision in decisions.as_array().into_iter().flatten() {
            if decision["run_id"] == json!(run_id)
                && let Some(id) = decision["decision_id"].as_str()
                && shown.insert(id.to_owned())
            {
                println!("{}", json!({"event":"decision","decision":decision}));
            }
        }
        if output["has_more"] != true {
            connection.wait_update(&mut events).await?;
        }
    }
}
