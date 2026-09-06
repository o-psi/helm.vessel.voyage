pub(super) use super::dispatch::{Rejected, dispatch};
use super::*;
use crate::attachment::journal::{SteeringActor, SteeringAdmission};
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;
pub(super) async fn dispatch_admitted(
    state: &Arc<State>,
    command: RuntimeCommand,
    authorization: super::authorization::Authorization,
) -> Result<Value> {
    let command = if let RuntimeCommand::WorkflowSubmit {
        command_id,
        user_directory,
        ..
    } = &command
    {
        ensure!(
            authorization.grant.is_none() || user_directory.is_none(),
            "custom workflow directories require local owner authority"
        );
        if let Some(receipt) = state.owner.process_receipt(*command_id).await? {
            return Ok(receipt);
        }
        state
            .workflows
            .prepare(
                &state.registration.workspace,
                authorization.actor.principal_id,
                &command,
            )
            .await?
    } else {
        command
    };

    match command {
        command @ RuntimeCommand::Github { .. } => {
            super::github::submit(state, authorization, command).await
        }
        command @ RuntimeCommand::Configure { .. } => {
            super::configuration::configure(state, command, authorization).await
        }
        RuntimeCommand::WorkflowInputs { input_id, values } => {
            state
                .workflows
                .store(authorization.actor.principal_id, input_id, values)
                .await
        }
        RuntimeCommand::WorkflowPreview {
            id,
            scope,
            user_directory,
            inputs,
            trust_digest,
        } => {
            ensure!(
                authorization.grant.is_none() || user_directory.is_none(),
                "custom workflow directories require local owner authority"
            );
            super::workflows::preview(
                &state.registration.workspace,
                &id,
                scope.as_deref(),
                user_directory.as_deref(),
                &inputs,
                trust_digest.as_deref(),
            )
        }
        RuntimeCommand::WorkflowSubmit { .. } => {
            unreachable!("workflow translated before dispatch")
        }
        command @ RuntimeCommand::Relinquish { .. } => {
            super::transfer::relinquish(state, command).await
        }
        RuntimeCommand::AssignmentObserve {
            run_id,
            assignment_id,
            participant,
            cancel,
        } => {
            let config = state.config.read().await.clone();
            crate::participant::reconcile(
                state.owner.clone(),
                run_id,
                assignment_id,
                &participant,
                cancel,
                &config,
            )
            .await
        }
        RuntimeCommand::Events {
            after,
            limit,
            wait_ms,
        } => super::observations::observe(state, after, limit, wait_ms).await,
        RuntimeCommand::Health => {
            let mut capabilities = vec![
                "snapshot",
                "history",
                "message_chunk",
                "run_output",
                "submit",
                "receipt",
                "cancel",
                "steer",
                "rename",
                "set_model",
                "decisions",
                "respond",
                "archive",
                "delete",
                "branch",
                "clear",
                "compact",
                "events",
                "controls",
                "operator_tool",
                "configure",
                "workflow_submit",
                "terminal",
                "assignment_observe",
                "relinquish",
                "stop",
            ];
            if matches!(
                state.registration.initialize,
                Some(voyage_protocol::process::RuntimeInitialization::Outbound { .. })
            ) {
                capabilities.retain(|capability| {
                    !matches!(
                        *capability,
                        "submit"
                            | "steer"
                            | "set_model"
                            | "operator_tool"
                            | "configure"
                            | "workflow_submit"
                    )
                });
            }
            Ok(
                json!({"pid":std::process::id(),"session_id":state.registration.session_id,"incarnation":state.registration.incarnation,"capabilities":capabilities,"outbound":state.outbound_status.lock().await.clone(),"decisions":"bounded_120_seconds"}),
            )
        }
        RuntimeCommand::Snapshot => {
            let mut snapshot = state.owner.process_snapshot().await?;
            snapshot["outbound"] = state.outbound_status.lock().await.clone();
            snapshot["decisions"] = state
                .owner
                .decisions(state.registration.incarnation)
                .await?;
            Ok(snapshot)
        }
        RuntimeCommand::History {
            offset,
            limit,
            expected_revision,
        } => {
            state
                .owner
                .process_history(offset, limit, expected_revision)
                .await
        }
        RuntimeCommand::MessageChunk {
            index,
            offset,
            limit,
            expected_revision,
        } => {
            state
                .owner
                .process_message_chunk(index, offset, limit, expected_revision)
                .await
        }
        RuntimeCommand::RunOutput {
            run_id,
            offset,
            limit,
        } => state.owner.process_run_output(run_id, offset, limit).await,
        RuntimeCommand::Receipt { command_id } => Ok(state
            .owner
            .process_receipt(command_id)
            .await?
            .unwrap_or(json!({"command_id":command_id,"status":"unknown"}))),
        command @ (RuntimeCommand::Submit { .. } | RuntimeCommand::OperatorTool { .. }) => {
            super::submission::submit(state, authorization, command).await
        }
        RuntimeCommand::Steer {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            prompt,
        } => {
            let _admission = state.admission.lock().await;
            let request = SteeringAdmission {
                receipt_id: command_id,
                session_id: state.registration.session_id,
                run_id,
                actor: SteeringActor {
                    machine_id: authorization.actor.installation_id,
                    principal_id: authorization.actor.principal_id,
                },
                expected_revision,
                expires_at_ms: i64::try_from(expires_at_ms)?,
                text: prompt,
            };
            if let Some(prior) = state.owner.process_receipt(command_id).await? {
                ensure!(
                    prior.get("request") == Some(&serde_json::to_value(&request)?),
                    "command ID payload conflict"
                );
                return Ok(json!({"duplicate":true,"record":prior}));
            }
            ensure!(!state.shutdown.is_cancelled(), "runtime stopping");
            let active = state.active.lock().await;
            let active = active.as_ref().context("no active run")?;
            ensure!(active.id == run_id, "run mismatch");
            let receipt = active
                .steering
                .as_ref()
                .context("operator run does not accept steering")?
                .submit(request)
                .await?;
            Ok(json!({"duplicate":receipt.duplicate,"record":receipt.record}))
        }
        command @ (RuntimeCommand::Cancel { .. }
        | RuntimeCommand::Rename { .. }
        | RuntimeCommand::SetModel { .. }) => {
            let _admission = state.admission.lock().await;
            ensure!(!state.shutdown.is_cancelled(), "runtime stopping");
            if let RuntimeCommand::SetModel { model, .. } = &command {
                ensure!(
                    state.active.lock().await.is_none(),
                    "model change requires idle runtime"
                );
                crate::provider::validate_model(
                    &crate::provider::ModelInfo::minimal(model.clone()),
                    &[],
                )?;
                if state.owner.snapshot().await?.session.model != *model {
                    state
                        .controls
                        .close_for_command(&state.owner, &command)
                        .await?;
                }
            }
            let cancel_run = match &command {
                RuntimeCommand::Cancel { run_id, .. } => Some(*run_id),
                _ => None,
            };
            let receipt = state
                .owner
                .process_metadata(authorization.actor, command)
                .await?;
            if let Some(run) = cancel_run
                && let Some(active) = state.active.lock().await.as_ref()
                && active.id == run
            {
                active.cancel.cancel();
            }
            Ok(receipt)
        }
        command @ (RuntimeCommand::Clear { .. }
        | RuntimeCommand::Compact { .. }
        | RuntimeCommand::Archive { .. }
        | RuntimeCommand::Delete { .. }
        | RuntimeCommand::Branch { .. }) => {
            let _admission = state.admission.lock().await;
            ensure!(!state.shutdown.is_cancelled(), "runtime stopping");
            let deleting = matches!(command, RuntimeCommand::Delete { .. });
            ensure!(
                state.active.lock().await.is_none(),
                "lifecycle requires idle runtime"
            );
            if !matches!(command, RuntimeCommand::Branch { .. }) {
                state
                    .controls
                    .close_for_command(&state.owner, &command)
                    .await?;
            }
            let configuration = if let RuntimeCommand::Branch { command_id, .. } = &command {
                if let Some(receipt) = state.owner.process_receipt(*command_id).await? {
                    return Ok(receipt);
                }
                let mut config = state.config.read().await.clone();
                config.model = state.owner.snapshot().await?.session.model;
                Some(serde_json::to_string(
                    &crate::launch_config::LaunchConfig::capture(
                        &config,
                        &state.registration.workspace,
                    )?,
                )?)
            } else {
                None
            };
            let result = state.owner.apply_lifecycle(command, configuration).await?;
            if deleting {
                state.workflows.clear().await;
            }
            Ok(result)
        }
        RuntimeCommand::Controls { run_id, section } => {
            let _admission = if section == "models" {
                Some(state.admission.lock().await)
            } else {
                None
            };
            let mut config = state.config.read().await.clone();
            config.model = state.owner.snapshot().await?.session.model;
            state
                .controls
                .inspect_or_idle(run_id, &section, &config, &state.registration.workspace)
                .await
        }
        command @ RuntimeCommand::ExecuteTool { .. } => {
            state
                .controls
                .execute(state.owner.clone(), state.registration.session_id, command)
                .await
        }
        RuntimeCommand::Terminal {
            run_id,
            terminal_id,
            operation,
        } => {
            state
                .controls
                .terminal(run_id, terminal_id, operation)
                .await
        }
        RuntimeCommand::Decisions => state.owner.decisions(state.registration.incarnation).await,
        command @ RuntimeCommand::Respond { .. } => {
            let _admission = state.admission.lock().await;
            ensure!(!state.shutdown.is_cancelled(), "runtime stopping");
            state
                .owner
                .respond_decision(state.registration.incarnation, command)
                .await
        }
        RuntimeCommand::Stop => {
            state.shutdown.cancel();
            Ok(json!({"status":"stopping","cleanup":"pending"}))
        }
    }
}
