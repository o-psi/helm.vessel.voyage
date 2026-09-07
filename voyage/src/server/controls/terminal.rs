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
            ensure!(bytes.len() <= 65536, "private input frame exceeds 64 KiB");
            // Enter private capture before accepting bytes, even without an earlier attach.
            manager.attach(id).await?;
            manager.write(id, bytes).await?;
            None
        }
        TerminalOperation::Resize { columns, rows } => {
            ensure!(
                (1..=500).contains(&columns) && (1..=500).contains(&rows),
                "invalid terminal size"
            );
            manager.attach(id).await?;
            manager.resize(id, columns, rows).await?;
            None
        }
    };
    policy.check_execution_authority()?;
    Ok(match snapshot {
        Some(screen) => {
            json!({"terminal_id":id,"run_id":run,"title":screen.title,"revision":screen.revision,"state":screen.state,"cursor":screen.cursor,"rows":screen.cells.iter().map(|row|row.iter().map(|cell|cell.text.as_str()).collect::<String>()).collect::<Vec<_>>(),"privacy":"human_only"})
        }
        None => json!({"terminal_id":id,"run_id":run,"accepted":true,"replay":"never"}),
    })
}
