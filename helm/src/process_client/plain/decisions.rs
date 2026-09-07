//! Attended answers remain bound to the exact displayed decision and run.
use super::{input, output, session::Connection};
use crate::process_client::safe;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use uuid::Uuid;
use voyage_protocol::process::RuntimeCommand;

pub(super) async fn follow(connection: &Connection<'_>, run_id: Uuid) -> Result<()> {
    let mut lines = input::lines();
    let mut offset = 0;
    let mut shown = None::<Value>;
    let snapshot = connection.snapshot().await?;
    let cursor = snapshot["observation_cursor"]
        .as_u64()
        .context("snapshot observation cursor missing")?;
    let mut events = connection.event_stream(cursor).await?;
    loop {
        let state = output::drain(connection, run_id, &mut offset).await?;
        if !matches!(
            state.as_str(),
            "accepted" | "running" | "awaiting_decision" | "cancel_requested"
        ) {
            println!();
            eprintln!("Run {run_id}: {state}");
            ensure!(state == "completed", "run ended with state {state}");
            return Ok(());
        }
        let decisions = connection.forward(RuntimeCommand::Decisions).await?;
        let next = decisions
            .as_array()
            .into_iter()
            .flatten()
            .find(|decision| decision["run_id"] == json!(run_id));
        if shown.as_ref().map(|value| &value["decision_id"])
            != next.map(|value| &value["decision_id"])
        {
            shown = next.cloned();
            if let Some(decision) = &shown {
                eprintln!(
                    "\nDecision {}: {}",
                    decision["decision_id"],
                    safe(&decision["request"].to_string())
                );
                eprintln!(
                    "{}",
                    if decision["request"]["kind"] == "approval" {
                        "Approve? Type yes or no."
                    } else {
                        "Enter your answer or the option number."
                    }
                );
            }
        }
        tokio::select! {
            result=wait_update(connection,&mut events)=>result?,
            line=lines.recv()=>{
                let Some(line)=line else { eprintln!("Input closed; voyage continues. Decisions expire at their runtime deadlines.");return output::follow(connection,run_id).await; };
                let line=line?;
                let Some(decision)=shown.as_ref() else {eprintln!("No pending question; use chat to steer this run.");continue};
                if let Err(error)=respond(connection,decision,&line).await {eprintln!("{}",safe(&error.to_string()));}
            }
        }
    }
}

async fn wait_update(
    connection: &Connection<'_>,
    events: &mut Option<
        futures_util::stream::BoxStream<'static, Result<voyage_protocol::process::VesselEvent>>,
    >,
) -> Result<()> {
    connection.wait_update(events).await
}

async fn respond(connection: &Connection<'_>, shown: &Value, line: &str) -> Result<()> {
    let snapshot = connection.snapshot().await?;
    let decision = snapshot["decisions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["decision_id"] == shown["decision_id"] && d["run_id"] == shown["run_id"])
        .context("displayed decision expired or was answered elsewhere")?;
    let response = if decision["request"]["kind"] == "approval" {
        match line.trim().to_ascii_lowercase().as_str() {
            "yes" | "y" => json!("approved"),
            "no" | "n" => json!("denied"),
            _ => anyhow::bail!("type yes or no; decision remains pending"),
        }
    } else {
        let options = decision["request"]["question"]["options"].as_array();
        if let Some(index) = line
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            && let Some(answer) = options.and_then(|values| values.get(index))
        {
            json!({"status":"selected","index":index,"answer":answer})
        } else {
            ensure!(!line.trim().is_empty(), "answer is empty");
            json!({"status":"custom","answer":line})
        }
    };
    let command_id = Uuid::new_v4();
    eprintln!("Decision response command {command_id}");
    connection
        .forward(RuntimeCommand::Respond {
            command_id,
            expected_revision: snapshot["revision"]
                .as_u64()
                .context("snapshot revision missing")?,
            expires_at_ms: super::session::deadline()?.min(
                decision["expires_at_ms"]
                    .as_u64()
                    .context("decision deadline missing")?,
            ),
            run_id: serde_json::from_value(decision["run_id"].clone())?,
            decision_id: serde_json::from_value(decision["decision_id"].clone())?,
            response,
        })
        .await?;
    Ok(())
}
