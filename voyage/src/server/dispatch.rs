//! Reserve immutable command identity before authoritative dispatch.
use super::*;
use serde_json::Value;
use voyage_protocol::process::RuntimeCommand;
pub(super) async fn dispatch(
    state: &Arc<State>,
    command: RuntimeCommand,
    authorization: super::authorization::Authorization,
) -> Result<Value> {
    let public = match &command {
        RuntimeCommand::Resolve {
            original: Some(original),
            ..
        } => original.as_ref(),
        _ => &command,
    };
    validate_public(public, &*state.config.read().await)?;
    if matches!(
        state.registration.initialize,
        Some(voyage_protocol::process::RuntimeInitialization::Outbound { .. })
    ) {
        ensure!(
            !matches!(
                command,
                RuntimeCommand::UploadImage { .. }
                    | RuntimeCommand::SubmitContent { .. }
                    | RuntimeCommand::Submit { .. }
                    | RuntimeCommand::WorkflowSubmit { .. }
                    | RuntimeCommand::OperatorTool { .. }
                    | RuntimeCommand::ExecuteTool { .. }
                    | RuntimeCommand::Github { .. }
                    | RuntimeCommand::SetAccess { .. }
                    | RuntimeCommand::Configure { .. }
                    | RuntimeCommand::SetModel { .. }
                    | RuntimeCommand::SetInference { .. }
                    | RuntimeCommand::Clear { .. }
                    | RuntimeCommand::Compact { .. }
                    | RuntimeCommand::Steer { .. }
            ),
            "outbound execution requires its current remote connection grant"
        );
    }
    let command_id = match &command {
        RuntimeCommand::Clear { command_id, .. }
        | RuntimeCommand::Compact { command_id, .. }
        | RuntimeCommand::OperatorTool { command_id, .. }
        | RuntimeCommand::Github { command_id, .. }
        | RuntimeCommand::SetAccess { command_id, .. }
        | RuntimeCommand::Configure { command_id, .. }
        | RuntimeCommand::Relinquish { command_id, .. }
        | RuntimeCommand::WorkflowSubmit { command_id, .. }
        | RuntimeCommand::SubmitContent { command_id, .. }
        | RuntimeCommand::Submit { command_id, .. }
        | RuntimeCommand::Cancel { command_id, .. }
        | RuntimeCommand::Steer { command_id, .. }
        | RuntimeCommand::Rename { command_id, .. }
        | RuntimeCommand::SetModel { command_id, .. }
        | RuntimeCommand::SetInference { command_id, .. }
        | RuntimeCommand::Respond { command_id, .. }
        | RuntimeCommand::Archive { command_id, .. }
        | RuntimeCommand::Delete { command_id, .. }
        | RuntimeCommand::Branch { command_id, .. }
        | RuntimeCommand::ExecuteTool { command_id, .. } => Some(*command_id),
        _ => None,
    };
    if let Some(id) = command_id {
        state
            .owner
            .bind_process_command(id, authorization.actor.principal_id, command.clone())
            .await?;
    }
    if let Some(id) = command_id
        && let Some(receipt) = state.owner.process_receipt(id).await?
    {
        if receipt["status"] == "rejected" || receipt["status"] == "not_admitted" {
            return Err(Rejected(receipt).into());
        }
        if receipt["status"] == "deleted" || receipt["status"] == "transferred" {
            return Ok(receipt);
        }
    }
    let principal = authorization.actor.principal_id;
    let workflow = match &command {
        RuntimeCommand::WorkflowSubmit {
            command_id,
            private_inputs_id,
            ..
        } => Some((*command_id, *private_inputs_id)),
        _ => None,
    };
    match super::commands::dispatch_admitted(state, command, authorization).await {
        Ok(result) => Ok(result),
        Err(error) => {
            if let Some((id, input)) = workflow {
                state.workflows.discard(principal, id, input).await;
            }
            if let Some(id) = command_id
                && let Some(receipt) = state
                    .owner
                    .reject_unadmitted(id, principal, error.to_string())
                    .await?
            {
                return Err(Rejected(receipt).into());
            }
            Err(error)
        }
    }
}
#[derive(Debug)]
pub(super) struct Rejected(pub Value);
impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.0["reason"].as_str().unwrap_or("command rejected")
        )
    }
}
impl std::error::Error for Rejected {}

/// Resolution accepts the same public envelope as dispatch, never private input.
pub(super) fn validate_public(command: &RuntimeCommand, config: &Config) -> Result<()> {
    if let RuntimeCommand::ExecuteTool {
        name, arguments, ..
    }
    | RuntimeCommand::OperatorTool {
        name, arguments, ..
    } = command
    {
        ensure!(
            !(name == "process" && arguments["action"] == "write"),
            "human terminal input requires private channel"
        );
    }
    if matches!(
        command,
        RuntimeCommand::SetModel { .. }
            | RuntimeCommand::SetInference { .. }
            | RuntimeCommand::OperatorTool { .. }
            | RuntimeCommand::ExecuteTool { .. }
            | RuntimeCommand::Github { .. }
            | RuntimeCommand::WorkflowSubmit { .. }
    ) {
        let public = serde_json::to_string(&command)?;
        ensure!(
            !crate::build::redactor(config).contains_secret(&public),
            "command contains configured secret; use private input channel"
        );
    }
    Ok(())
}
