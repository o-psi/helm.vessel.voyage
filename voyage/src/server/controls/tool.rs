use super::*;
pub(super) async fn execute(
    controls: &LiveControls,
    owner: ManagedSessionOwner,
    session: Uuid,
    command: RuntimeCommand,
) -> Result<Value> {
    if let Some(receipt) = owner.control_receipt(command.clone()).await? {
        return Ok(receipt);
    }
    let RuntimeCommand::ExecuteTool {
        command_id,
        run_id,
        ref name,
        ref arguments,
        ..
    } = command
    else {
        anyhow::bail!("invalid tool command")
    };
    let active = controls.active(Some(run_id)).await?;
    ensure!(
        name.len() <= 128 && serde_json::to_vec(arguments)?.len() <= 65536,
        "tool command exceeds limits"
    );
    active.agent.operator_arguments(arguments)?;
    ensure!(
        active
            .agent
            .tool_inventory()
            .iter()
            .any(|tool| tool.name == *name),
        "tool is not in the live authorized registry"
    );
    let permit = active
        .slots
        .clone()
        .try_acquire_owned()
        .context("runtime controls busy")?;
    let (receipt, fresh) = owner.admit_control(command.clone()).await?;
    if !fresh {
        return Ok(receipt);
    }
    let name = name.clone();
    let arguments = arguments.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let result = active
            .agent
            .operator_tool(session, run_id, active.cancel.clone(), &name, arguments)
            .await;
        let outcome = match result {
            Ok(value) => {
                json!({"status":"completed","output":active.agent.redact_diagnostic(value)})
            }
            Err(error) => {
                json!({"status":"failed","error":active.agent.redact_diagnostic(error.to_string())})
            }
        };
        // A failed checkpoint leaves the durable accepted/unknown record, never a replay.
        if owner.complete_control(command_id, outcome).await.is_err() {
            active.cancel.cancel();
        }
    });
    Ok(receipt)
}
