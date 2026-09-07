use super::session::Connection;
use crate::process_client::safe;
use anyhow::{Context, Result, ensure};
use std::io::Write;
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;

pub(super) async fn drain(
    connection: &Connection<'_>,
    run_id: Uuid,
    offset: &mut u64,
) -> Result<String> {
    // A bounded batch leaves time for input even if a provider floods output.
    for _ in 0..8 {
        let value = connection
            .voyage(VoyageCommand::RunOutput {
                run_id,
                offset: *offset,
                limit: 65536,
            })
            .await?;
        ensure!(
            value["run_id"] == serde_json::to_value(run_id)?
                && value["offset"].as_u64() == Some(*offset),
            "output identity or offset mismatch"
        );
        let data = value["data"].as_str().context("output data missing")?;
        let next = value["next_offset"]
            .as_u64()
            .context("output cursor missing")?;
        ensure!(
            next == offset.saturating_add(data.len() as u64),
            "invalid output cursor"
        );
        print!("{}", safe(data));
        std::io::stdout().flush()?;
        *offset = next;
        if value["has_more"] != true {
            return Ok(value["state"]
                .as_str()
                .context("run state missing")?
                .to_owned());
        }
    }
    Ok("running".into())
}

pub(super) async fn follow(connection: &Connection<'_>, run_id: Uuid) -> Result<()> {
    let mut offset = 0;
    let mut shown = std::collections::BTreeSet::new();
    let snapshot = connection.snapshot().await?;
    let cursor = snapshot["observation_cursor"]
        .as_u64()
        .context("snapshot observation cursor missing")?;
    let mut events = connection.event_stream(cursor).await?;
    loop {
        let state = drain(connection, run_id, &mut offset).await?;
        if !matches!(
            state.as_str(),
            "accepted" | "running" | "awaiting_decision" | "cancel_requested"
        ) {
            println!();
            eprintln!("Run {run_id}: {state}");
            ensure!(state == "completed", "run ended with state {state}");
            return Ok(());
        }
        let decisions = connection.voyage(VoyageCommand::Decisions).await?;
        for decision in decisions.as_array().into_iter().flatten() {
            if let Some(id) = decision["decision_id"].as_str()
                && shown.insert(id.to_owned())
            {
                eprintln!(
                    "\nDecision {id}: {}. Respond from helm connect chat or the TUI before its deadline.",
                    safe(&decision["request"].to_string())
                );
            }
        }
        connection.wait_update(&mut events).await?;
    }
}
