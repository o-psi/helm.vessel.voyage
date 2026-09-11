use super::*;
pub(super) async fn execute(
    controls: &LiveControls,
    run: Uuid,
    id: Uuid,
    operation: TerminalOperation,
) -> Result<Value> {
    use crate::terminal::InteractiveTerminals;
    let retained = controls.retained(run).await?;
    let manager = &retained.manager;
    let policy = &retained.policy;
    policy.check_execution_authority()?;
    let id = crate::terminal::TerminalId(id);
    let snapshot = match operation {
        TerminalOperation::Attach => Some(manager.attach(id).await?),
        TerminalOperation::Snapshot => Some(manager.attach(id).await?),
        TerminalOperation::Write { bytes } => {
            ensure!(
                policy.access_mode() != crate::config::AccessMode::ReadOnly,
                "private input is disabled in read-only access mode"
            );
            ensure!(bytes.len() <= 65536, "private input frame exceeds 64 KiB");
            // Enter private capture before accepting bytes, even without an earlier attach.
            manager.attach(id).await?;
            manager.write(id, bytes).await?;
            None
        }
        TerminalOperation::Resize { columns, rows } => {
            ensure!(
                (1..=voyage_protocol::terminal::MAX_COLUMNS).contains(&columns)
                    && (1..=voyage_protocol::terminal::MAX_ROWS).contains(&rows),
                "invalid terminal size"
            );
            manager.attach(id).await?;
            manager.resize(id, columns, rows).await?;
            None
        }
    };
    policy.check_execution_authority()?;
    let frame = match snapshot {
        Some(screen) => {
            let typed = crate::terminal::TerminalScreen {
                version: voyage_protocol::terminal::SCREEN_VERSION,
                columns: screen.cells.first().map_or(0, |row| row.len() as u16),
                height: screen.cells.len() as u16,
                cells: screen.cells.clone(),
                modes: screen.modes,
            };
            typed.validate().map_err(anyhow::Error::msg)?;
            json!({"terminal_id":id,"run_id":run,"title":screen.title,"revision":screen.revision,"state":screen.state,"cursor":screen.cursor,"rows":screen.cells.iter().map(|row|row.iter().map(|cell|cell.text.as_str()).collect::<String>()).collect::<Vec<_>>(),"privacy":"human_only","screen":typed})
        }
        None => json!({"terminal_id":id,"run_id":run,"accepted":true,"replay":"never"}),
    };
    ensure!(
        serde_json::to_vec(&frame)?.len() < 4 * 1024 * 1024,
        "terminal frame exceeds transport limit"
    );
    Ok(frame)
}
