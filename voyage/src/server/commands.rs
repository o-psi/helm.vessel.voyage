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
        RuntimeCommand::PrepareBrowser => {
            if let Some(authority) = &authorization.authority {
                authority.check()?;
            }
            // The supervisor has already followed/resumed the owner under its
            // existing lifecycle fences. Do not offer, build an agent, or infer
            // local consent here. The outer reply carries the current incarnation.
            Ok(json!({"prepared": true}))
        }
        RuntimeCommand::Browser { operation } => {
            if let Some(authority) = &authorization.authority {
                authority.check()?;
            }
            let cleanup = matches!(
                &operation,
                voyage_protocol::browser::BrowserOperation::Cleanup { observed: true, .. }
                    | voyage_protocol::browser::BrowserOperation::Result { .. }
            );
            let mut attempts = 0;
            let result = loop {
                if let Some(authority) = &authorization.authority {
                    authority.check()?;
                }
                match state
                    .browser
                    .operate(authorization.actor.principal_id, operation.clone())
                {
                    Ok(value) => break value,
                    Err(error) if crate::browser::storage_busy(&error) && attempts < 50 => {
                        attempts += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                    Err(error) => return Err(error),
                }
            };
            if cleanup {
                state.cleanup.retry().await;
            }
            Ok(serde_json::to_value(result)?)
        }
        RuntimeCommand::UploadImage {
            upload_id,
            name,
            data_base64,
        } => super::images::upload(state, authorization, upload_id, name, data_base64).await,
        RuntimeCommand::Resolve {
            command_id,
            original,
        } => {
            // Transport holds the exclusive dispatch gate for this operation.
            state
                .owner
                .resolve_process_command(command_id, authorization.actor.principal_id, original)
                .await
        }
        command @ RuntimeCommand::Github { .. } => {
            super::github::submit(state, authorization, command).await
        }
        command @ (RuntimeCommand::Configure { .. }
        | RuntimeCommand::SetAccess { .. }
        | RuntimeCommand::SetInference { .. }
        | RuntimeCommand::SetAccountInference { .. }) => {
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
            optional_secret_names,
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
                optional_secret_names.as_deref(),
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
            let capabilities = vec![
                "snapshot",
                "read_artifact",
                "history",
                "message_chunk",
                "run_output",
                "submit",
                "submit_content",
                "upload_image",
                "receipt",
                "resolve",
                "cancel",
                "steer",
                "rename",
                "set_model",
                "set_inference",
                "set_account_inference",
                "set_access",
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
                "browser",
                "stop",
            ];
            Ok(
                json!({"pid":std::process::id(),"session_id":state.registration.session_id,"incarnation":state.registration.incarnation,"capabilities":capabilities,"decisions":"bounded_120_seconds"}),
            )
        }
        RuntimeCommand::Snapshot => {
            // Serialize with settings publication so revision and values describe one state.
            let _admission = state.admission.lock().await;
            let mut snapshot = state.owner.process_snapshot().await?;
            bootstrap::annotate_workspace(&mut snapshot, &state.directory, &state.registration);
            let mut inference_config = state.config.read().await.clone();
            let saved = state.owner.snapshot().await?.session;
            inference_config.model = saved.pending_model.unwrap_or(saved.model);
            state.controls.schedule_resolution(
                inference_config.clone(),
                state.registration.workspace.clone(),
            );
            let known = state.controls.known_model(&inference_config).await;
            snapshot["inference"] = super::configuration::inference_snapshot_with_model(
                &inference_config,
                known.as_ref(),
            );
            let active = state.active.lock().await;
            snapshot["inference_next_turn"] = json!(active.is_some());
            snapshot["inference_current"] = active
                .as_ref()
                .map_or(Value::Null, |run| run.inference.clone());
            if let Some(run) = active.as_ref()
                && let Some(resolution) = state.controls.inference_resolution(run.id).await
            {
                snapshot["inference_current"]["resolution"] = json!(resolution);
            }
            drop(active);
            snapshot["access"] = crate::runtime_policy::RuntimePolicy::resolve(
                &*state.config.read().await,
                &state.registration.workspace,
            )
            .ok()
            .and_then(|p| serde_json::to_value(p.policy().access_mode()).ok())
            .unwrap_or(serde_json::Value::Null);
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
        RuntimeCommand::ReadArtifact {
            artifact_id,
            offset,
            limit,
        } => {
            ensure!(
                !state.owner.process_snapshot().await?["lifecycle"]["deleted"]
                    .as_bool()
                    .unwrap_or(false),
                "session deleted"
            );
            crate::artifacts::Store::open(
                &state.directory.join("journal"),
                state.registration.session_id,
            )?
            .chunk(artifact_id, offset, limit as usize)
            .map_err(|_| anyhow::anyhow!("artifact unavailable or invalid range"))
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
        command @ (RuntimeCommand::SubmitContent { .. }
        | RuntimeCommand::Submit { .. }
        | RuntimeCommand::OperatorTool { .. }) => {
            super::submission::submit(state, authorization, command).await
        }
        RuntimeCommand::Steer {
            coordination,
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            prompt,
        } => {
            let _admission = state.admission.lock().await;
            let request = SteeringAdmission {
                coordination,
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
                let mut candidate = state.config.read().await.clone();
                candidate.model = model.clone();
                crate::provider::validate_inference_settings(&candidate)?;
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
            let archiving = matches!(command, RuntimeCommand::Archive { archived: true, .. });
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
                {
                    let session = state.owner.snapshot().await?.session;
                    config.model = session.pending_model.unwrap_or(session.model);
                }
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
            if (archiving || deleting)
                && result["status"] != "rejected"
                && state.owner.process_snapshot().await?["lifecycle"]
                    [if deleting { "deleted" } else { "archived" }]
                    == true
            {
                *state.archive_receipt.lock().await = Some(result.clone());
                // Admission remains locked: a racing restore/submit cannot start work.
                // The listener drains this response before publishing cleanup evidence.
                state.shutdown.cancel();
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
            {
                let session = state.owner.snapshot().await?.session;
                config.model = session.pending_model.unwrap_or(session.model);
            }
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
