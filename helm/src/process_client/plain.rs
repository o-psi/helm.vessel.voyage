//! Line and one-turn interfaces observe the same independent owner as the TUI.
mod decisions;
mod input;
mod json;
mod output;
mod session;

use super::{safe, transport::Client};
use anyhow::{Context, Result, ensure};
use std::io::{IsTerminal, Write};
use uuid::Uuid;
use voyage_protocol::process::RuntimeCommand;

pub async fn run(
    client: &Client,
    session: Uuid,
    command: Option<Uuid>,
    prompt: String,
) -> Result<()> {
    let connection = session::Connection::open(client, session).await?;
    let run = connection.submit(command, prompt).await?;
    tokio::select! {
        result = follow_connection(&connection, run) => result,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("Detached from voyage {session}, run {run}; accepted work continues.");
            Ok(())
        }
    }
}

pub async fn follow(client: &Client, session: Uuid, run: Uuid) -> Result<()> {
    let connection = session::Connection::open(client, session).await?;
    tokio::select! {
        result=follow_connection(&connection,run)=>result,
        _=tokio::signal::ctrl_c()=>{ eprintln!("Detached from voyage {session}, run {run}; accepted work continues.");Ok(()) }
    }
}

pub async fn chat(client: &Client, session: Uuid) -> Result<()> {
    let connection = session::Connection::open(client, session).await?;
    let interactive = std::io::stdin().is_terminal();
    eprintln!(
        "Voyage {session}. /cancel, /approve ID, /deny ID, /answer ID text, /quit. Input during a run steers it. EOF detaches."
    );
    let mut input = input::lines();
    let mut run = None;
    let mut offset = 0;
    let mut decisions = std::collections::BTreeSet::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(400));
    loop {
        if interactive {
            std::io::stdout().flush()?;
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            line = input.recv() => {
                let Some(line) = line else { break };
                let line = line?;
                if line.trim() == "/quit" { break; }
                if line.trim().is_empty() { continue; }
                let snapshot = connection.snapshot().await?;
                let result = if line.starts_with('/') {
                    command(&connection, &snapshot, &line).await
                } else {
                    connection.send(&snapshot, line).await.map(|id| {
                        if run != Some(id) { run = Some(id); offset = 0; }
                    })
                };
                if let Err(error) = result { eprintln!("{}", safe(&error.to_string())); }
            },
            _ = tick.tick() => {
                let snapshot = connection.snapshot().await?;
                if let Some(current) = snapshot.get("run").filter(|value| value.is_object()) {
                    let id = serde_json::from_value::<Uuid>(current["run_id"].clone())?;
                    if run != Some(id) { run = Some(id); offset = 0; }
                    output::drain(&connection, id, &mut offset).await?;
                }
                for decision in snapshot["decisions"].as_array().into_iter().flatten() {
                    if let Some(id) = decision["decision_id"].as_str()
                        && decisions.insert(id.to_owned()) {
                        eprintln!("\nDecision {id}: {}", safe(&decision["request"].to_string()));
                    }
                }
            }
        }
    }
    eprintln!("Detached from voyage {session}; accepted work continues.");
    Ok(())
}

async fn command(
    connection: &session::Connection<'_>,
    snapshot: &serde_json::Value,
    line: &str,
) -> Result<()> {
    let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
    let command_id = Uuid::new_v4();
    let expected_revision = snapshot["revision"]
        .as_u64()
        .context("snapshot revision missing")?;
    let expires_at_ms = session::deadline()?;
    let run_id =
        serde_json::from_value(snapshot["run"]["run_id"].clone()).context("no current run")?;
    let command = match name {
        "/cancel" => RuntimeCommand::Cancel {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
        },
        "/approve" | "/deny" | "/answer" => {
            let (id, answer) = rest.split_once(' ').unwrap_or((rest, ""));
            let decision_id: Uuid = id.parse().context("decision UUID required")?;
            let decision = snapshot["decisions"]
                .as_array()
                .context("no pending decisions")?
                .iter()
                .find(|d| d["decision_id"].as_str() == Some(id))
                .context("decision is not pending in this voyage")?;
            ensure!(
                decision["run_id"] == serde_json::to_value(run_id)?,
                "decision run changed"
            );
            let response = if name == "/answer" {
                ensure!(
                    !answer.is_empty() && decision["request"]["kind"] == "question",
                    "question answer required"
                );
                serde_json::json!({"status":"custom","answer":answer})
            } else {
                ensure!(
                    decision["request"]["kind"] == "approval",
                    "not an approval decision"
                );
                serde_json::json!(if name == "/approve" {
                    "approved"
                } else {
                    "denied"
                })
            };
            RuntimeCommand::Respond {
                command_id,
                expected_revision,
                expires_at_ms,
                run_id,
                decision_id,
                response,
            }
        }
        _ => anyhow::bail!(
            "unknown command; use /cancel, /approve ID, /deny ID, /answer ID text or /quit"
        ),
    };
    eprintln!("Command {command_id}");
    let receipt = connection.forward(command).await?;
    eprintln!("{}", safe(&receipt.to_string()));
    Ok(())
}

async fn follow_connection(connection: &session::Connection<'_>, run: Uuid) -> Result<()> {
    if std::io::stdin().is_terminal() {
        decisions::follow(connection, run).await
    } else {
        output::follow(connection, run).await
    }
}

/// Observe an accepted run as JSON lines. Interrupting detaches the interface.
pub async fn follow_json(client: &Client, session: Uuid, run: Uuid) -> Result<()> {
    let connection = session::Connection::open(client, session).await?;
    tokio::select! {
        result = json::follow(&connection, run) => result,
        _ = tokio::signal::ctrl_c() => {
            println!("{}", serde_json::json!({"event":"detached","session_id":session,"run_id":run}));
            Ok(())
        }
    }
}
