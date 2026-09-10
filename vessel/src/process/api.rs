//! Version adapter between the public Vessel API and private runtime IPC.
use super::{registry, routing, service::Supervisor};
use anyhow::{Result, ensure};
use serde_json::Value;
use voyage_protocol::{
    process::*,
    vessel::{TerminalAction, VoyageCommand, VoyageReply, VoyageRequest},
};

pub(super) fn runtime(command: VoyageCommand) -> Result<RuntimeCommand> {
    Ok(match command {
        VoyageCommand::Browser { operation } => RuntimeCommand::Browser { operation },
        VoyageCommand::Clear {
            command_id,
            expected_revision,
            expires_at_ms,
            confirm_session_id,
        } => RuntimeCommand::Clear {
            command_id,
            expected_revision,
            expires_at_ms,
            confirm_session_id,
        },
        VoyageCommand::Compact {
            command_id,
            expected_revision,
            expires_at_ms,
            retain,
        } => RuntimeCommand::Compact {
            command_id,
            expected_revision,
            expires_at_ms,
            retain,
        },
        VoyageCommand::AssignmentObserve {
            run_id,
            assignment_id,
            participant,
            cancel,
        } => RuntimeCommand::AssignmentObserve {
            run_id,
            assignment_id,
            participant,
            cancel,
        },
        VoyageCommand::OperatorTool {
            command_id,
            expected_revision,
            expires_at_ms,
            name,
            arguments,
        } => RuntimeCommand::OperatorTool {
            command_id,
            expected_revision,
            expires_at_ms,
            name,
            arguments,
        },
        VoyageCommand::Github {
            command_id,
            expected_revision,
            expires_at_ms,
            words,
        } => RuntimeCommand::Github {
            command_id,
            expected_revision,
            expires_at_ms,
            words,
        },
        VoyageCommand::SetAccess {
            command_id,
            expected_revision,
            expires_at_ms,
            access,
        } => RuntimeCommand::SetAccess {
            command_id,
            expected_revision,
            expires_at_ms,
            access,
        },
        VoyageCommand::Configure {
            command_id,
            expected_revision,
            expires_at_ms,
            config_path,
        } => RuntimeCommand::Configure {
            command_id,
            expected_revision,
            expires_at_ms,
            config_path,
        },
        VoyageCommand::WorkflowInputs { input_id, values } => {
            RuntimeCommand::WorkflowInputs { input_id, values }
        }
        VoyageCommand::WorkflowPreview {
            id,
            scope,
            user_directory,
            inputs,
            trust_digest,
        } => RuntimeCommand::WorkflowPreview {
            id,
            scope,
            user_directory,
            inputs,
            trust_digest,
        },
        VoyageCommand::WorkflowSubmit {
            command_id,
            expected_revision,
            expires_at_ms,
            id,
            scope,
            user_directory,
            inputs,
            trust_digest,
            private_inputs_id,
        } => RuntimeCommand::WorkflowSubmit {
            command_id,
            expected_revision,
            expires_at_ms,
            id,
            scope,
            user_directory,
            inputs,
            trust_digest,
            private_inputs_id,
        },
        VoyageCommand::Controls { run_id, section } => RuntimeCommand::Controls { run_id, section },
        VoyageCommand::ExecuteTool {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            name,
            arguments,
        } => RuntimeCommand::ExecuteTool {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            name,
            arguments,
        },
        VoyageCommand::Terminal {
            run_id,
            terminal_id,
            operation,
        } => RuntimeCommand::Terminal {
            run_id,
            terminal_id,
            operation: match operation {
                TerminalAction::Attach => TerminalOperation::Attach,
                TerminalAction::Snapshot => TerminalOperation::Snapshot,
                TerminalAction::Write { bytes } => TerminalOperation::Write { bytes },
                TerminalAction::Resize { columns, rows } => {
                    TerminalOperation::Resize { columns, rows }
                }
            },
        },
        VoyageCommand::Snapshot => RuntimeCommand::Snapshot,
        VoyageCommand::History {
            offset,
            limit,
            expected_revision,
        } => RuntimeCommand::History {
            offset,
            limit,
            expected_revision,
        },
        VoyageCommand::MessageChunk {
            index,
            offset,
            limit,
            expected_revision,
        } => RuntimeCommand::MessageChunk {
            index,
            offset,
            limit,
            expected_revision,
        },
        VoyageCommand::RunOutput {
            run_id,
            offset,
            limit,
        } => RuntimeCommand::RunOutput {
            run_id,
            offset,
            limit,
        },
        VoyageCommand::ReadArtifact {
            artifact_id,
            offset,
            limit,
        } => RuntimeCommand::ReadArtifact {
            artifact_id,
            offset,
            limit,
        },
        VoyageCommand::UploadImage {
            upload_id,
            name,
            data_base64,
        } => RuntimeCommand::UploadImage {
            upload_id,
            name,
            data_base64,
        },
        VoyageCommand::SubmitContent {
            command_id,
            expected_revision,
            expires_at_ms,
            content,
        } => RuntimeCommand::SubmitContent {
            command_id,
            expected_revision,
            expires_at_ms,
            content,
        },
        VoyageCommand::Submit {
            coordination,
            command_id,
            expected_revision,
            expires_at_ms,
            prompt,
        } => RuntimeCommand::Submit {
            coordination,
            command_id,
            expected_revision,
            expires_at_ms,
            prompt,
        },
        VoyageCommand::Receipt { command_id } => RuntimeCommand::Receipt { command_id },
        VoyageCommand::Resolve {
            command_id,
            original,
        } => RuntimeCommand::Resolve {
            command_id,
            original: original
                .map(|command| runtime(*command).map(Box::new))
                .transpose()?,
        },
        VoyageCommand::Cancel {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
        } => RuntimeCommand::Cancel {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
        },
        VoyageCommand::Steer {
            coordination,
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            prompt,
        } => RuntimeCommand::Steer {
            coordination,
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            prompt,
        },
        VoyageCommand::Rename {
            command_id,
            expected_revision,
            expires_at_ms,
            name,
        } => RuntimeCommand::Rename {
            command_id,
            expected_revision,
            expires_at_ms,
            name,
        },
        VoyageCommand::SetInference {
            command_id,
            expected_revision,
            expires_at_ms,
            model,
            reasoning_effort,
            service_tier,
        } => RuntimeCommand::SetInference {
            command_id,
            expected_revision,
            expires_at_ms,
            model,
            reasoning_effort,
            service_tier,
        },
        VoyageCommand::SetModel {
            command_id,
            expected_revision,
            expires_at_ms,
            model,
        } => RuntimeCommand::SetModel {
            command_id,
            expected_revision,
            expires_at_ms,
            model,
        },
        VoyageCommand::Archive {
            command_id,
            expected_revision,
            expires_at_ms,
            archived,
        } => RuntimeCommand::Archive {
            command_id,
            expected_revision,
            expires_at_ms,
            archived,
        },
        VoyageCommand::Delete {
            command_id,
            expected_revision,
            expires_at_ms,
            confirm_session_id,
        } => RuntimeCommand::Delete {
            command_id,
            expected_revision,
            expires_at_ms,
            confirm_session_id,
        },
        VoyageCommand::Events {
            after,
            limit,
            wait_ms,
        } => RuntimeCommand::Events {
            after,
            limit,
            wait_ms,
        },
        VoyageCommand::Decisions => RuntimeCommand::Decisions,
        VoyageCommand::Respond {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            decision_id,
            response,
        } => RuntimeCommand::Respond {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            decision_id,
            response,
        },
    })
}

/// Retain an authoritative rejection payload and its uncertainty without leaking
/// the runtime wire envelope into a public response.
#[derive(Debug)]
pub(super) struct OperationFailure {
    pub result: Value,
    pub message: String,
    pub outcome_unknown: bool,
}
impl std::fmt::Display for OperationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for OperationFailure {}

pub(super) fn reply(response: RuntimeResponse) -> Result<Value> {
    ensure!(
        response.protocol == PROCESS_PROTOCOL,
        "unsupported private runtime protocol"
    );
    if let Some(message) = response.error {
        return Err(OperationFailure {
            result: response.result,
            message,
            outcome_unknown: response.outcome_unknown,
        }
        .into());
    }
    if response.outcome_unknown {
        return Err(
            anyhow::anyhow!("runtime response did not establish command outcome")
                .context(routing::OutcomeUnknown),
        );
    }
    Ok(serde_json::to_value(VoyageReply {
        session_id: response.session_id,
        incarnation: response.incarnation,
        result: response.result,
    })?)
}

pub(super) fn response(result: Result<Value>) -> voyage_protocol::vessel::VesselResponse {
    use voyage_protocol::vessel::{VESSEL_API_VERSION, VesselResponse};
    match result {
        Ok(result) => VesselResponse {
            protocol: VESSEL_API_VERSION,
            result,
            error: None,
            outcome_unknown: false,
        },
        Err(error) => {
            let failure = error.downcast_ref::<OperationFailure>();
            VesselResponse {
                protocol: VESSEL_API_VERSION,
                result: failure.map_or(Value::Null, |failure| failure.result.clone()),
                error: Some(
                    error
                        .to_string()
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(512)
                        .collect(),
                ),
                outcome_unknown: failure.is_some_and(|failure| failure.outcome_unknown)
                    || error.downcast_ref::<routing::OutcomeUnknown>().is_some(),
            }
        }
    }
}

impl Supervisor {
    pub(super) async fn voyage(
        &self,
        request: VoyageRequest,
        authorization: Option<GrantBinding>,
    ) -> Result<Value> {
        // Owner selection is a service responsibility. Live-resource commands
        // carry a fence; ordinary session operations always select the current owner.
        let exact = request.command.requires_incarnation();
        ensure!(
            !exact || request.incarnation.is_some(),
            "operation requires observed incarnation"
        );
        // Persist the immutable inference intent before forwarding. This is not
        // an applied receipt: the owning Voyage remains the configuration authority.
        // Resolution checks the same envelope but never dispatches it automatically.
        let inference = match &request.command {
            command @ VoyageCommand::SetInference { .. } => Some((command.clone(), true)),
            VoyageCommand::Resolve {
                command_id,
                original: Some(original),
            } if matches!(original.as_ref(), VoyageCommand::SetInference { .. }) => {
                ensure!(
                    original.mutation_id() == Some(*command_id),
                    "resolution identity mismatch"
                );
                Some((original.as_ref().clone(), false))
            }
            _ => None,
        };
        if let Some((command, reserve)) = inference {
            ensure!(
                authorization.is_none(),
                "inference settings require owner authority"
            );
            self.registration(request.session_id).await?;
            let id = command.mutation_id().expect("inference mutation");
            ensure!(!id.is_nil(), "nil command ID");
            registry::command_record(
                &self.directory,
                id,
                &VesselCommand::Voyage(VoyageRequest {
                    session_id: request.session_id,
                    incarnation: None,
                    command,
                }),
                reserve,
            )?;
        }
        let command = runtime(request.command)?;
        let result = self
            .dispatch_session(
                request.session_id,
                if exact { request.incarnation } else { None },
                command,
                authorization,
            )
            .await?;
        reply(result)
    }
}

pub(super) fn required_right(command: &VoyageCommand) -> Option<ProcessRight> {
    match command {
        VoyageCommand::Resolve {
            command_id,
            original: Some(original),
        } if original.mutation_id() == Some(*command_id) => required_right(original),
        // Legacy payload-free resolution can reserve an unknown ID and therefore
        // requires the host's local authority, not a read-only scoped grant.
        VoyageCommand::Resolve { .. } => None,
        VoyageCommand::Events { .. } => Some(ProcessRight::Observe),
        VoyageCommand::Controls { section, .. } if section == "host_resources" => None,
        VoyageCommand::Controls { section, .. }
            if matches!(section.as_str(), "tools" | "policy") =>
        {
            Some(ProcessRight::Observe)
        }
        VoyageCommand::Controls { .. } | VoyageCommand::WorkflowPreview { .. } => {
            Some(ProcessRight::History)
        }
        VoyageCommand::AssignmentObserve { .. } => Some(ProcessRight::History),
        VoyageCommand::Snapshot
        | VoyageCommand::History { .. }
        | VoyageCommand::MessageChunk { .. }
        | VoyageCommand::RunOutput { .. }
        | VoyageCommand::ReadArtifact { .. }
        | VoyageCommand::Receipt { .. } => Some(ProcessRight::History),
        VoyageCommand::Decisions => Some(ProcessRight::Decide),
        VoyageCommand::UploadImage { .. }
        | VoyageCommand::SubmitContent { .. }
        | VoyageCommand::Submit { .. }
        | VoyageCommand::ExecuteTool { .. }
        | VoyageCommand::OperatorTool { .. }
        | VoyageCommand::WorkflowInputs { .. }
        | VoyageCommand::WorkflowSubmit { .. } => Some(ProcessRight::Execute),
        VoyageCommand::Steer { .. } => Some(ProcessRight::Steer),
        VoyageCommand::Respond { .. } => Some(ProcessRight::Decide),
        VoyageCommand::Cancel { .. } => Some(ProcessRight::Cancel),
        VoyageCommand::Clear { .. }
        | VoyageCommand::Compact { .. }
        | VoyageCommand::Rename { .. }
        | VoyageCommand::SetModel { .. }
        | VoyageCommand::Archive { .. }
        | VoyageCommand::Delete { .. } => Some(ProcessRight::Lifecycle),
        VoyageCommand::Browser { .. } => Some(ProcessRight::Execute),
        VoyageCommand::Terminal { .. } => Some(ProcessRight::Terminal),
        VoyageCommand::Configure { .. }
        | VoyageCommand::SetAccess { .. }
        | VoyageCommand::SetInference { .. } => None,
        _ => None,
    }
}
