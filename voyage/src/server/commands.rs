use super::*;
use crate::attachment::{
    journal::{SteeringActor, SteeringAdmission, TurnAdmission},
    runtime::Admission,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;
struct Events;
#[async_trait]
impl crate::EventSink for Events {
    async fn emit(&self, _: crate::AgentEvent) {}
}

pub(super) async fn dispatch(state: &Arc<State>, command: RuntimeCommand) -> Result<Value> {
    match command {
        RuntimeCommand::Health => Ok(
            json!({"pid":std::process::id(),"session_id":state.registration.session_id,"incarnation":state.registration.incarnation,"capabilities":["snapshot","history","message_chunk","run_output","submit","receipt","cancel","steer","rename","set_model","decisions","respond","stop"],"decisions":"bounded_120_seconds"}),
        ),
        RuntimeCommand::Snapshot => {
            let mut snapshot = state.owner.process_snapshot().await?;
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
        RuntimeCommand::Submit {
            command_id,
            expected_revision,
            expires_at_ms,
            prompt,
        } => {
            let _admission = state.admission.lock().await;
            ensure!(!state.shutdown.is_cancelled(), "runtime stopping");
            let saved = state.owner.snapshot().await?;
            let mut config = state.config.clone();
            config.model = saved.session.model;
            ensure!(
                config.provider != crate::ProviderKind::CodexSubscription,
                "compatibility bridge process cleanup is unsupported"
            );
            ensure!(
                config.access_mode() == crate::config::AccessMode::ReadOnly
                    || config.mcp_servers.is_empty(),
                "effectful MCP cleanup is unsupported"
            );
            crate::policy::Policy::new(&config, saved.session.workspace.clone())?;
            let request = TurnAdmission {
                command_id,
                machine_id: state.actor.installation_id,
                principal_id: state.actor.principal_id,
                session_id: state.registration.session_id,
                expected_revision,
                expires_at_ms: i64::try_from(expires_at_ms)?,
                prompt,
            };
            let mut run = match state.owner.admit(request).await? {
                Admission::Existing(run) => {
                    return Ok(
                        json!({"command_id":command_id,"run_id":run.id,"status":"accepted","state":run.state,"duplicate":true}),
                    );
                }
                Admission::New(run) => run,
            };
            run.register_local_cleanup().await?;
            let run_id = run.record().await?.id;
            let actor = state.actor;
            let session_id = state.registration.session_id;
            let steering = run.enable_steering(Arc::new(
                move |candidate: SteeringActor, session: Uuid, run: Uuid| {
                    ensure!(
                        candidate.machine_id == actor.installation_id
                            && candidate.principal_id == actor.principal_id
                            && session == session_id
                            && run == run_id,
                        "steering authority mismatch"
                    );
                    Ok(())
                },
            ))?;
            let cancel = CancellationToken::new();
            *state.active.lock().await = Some(ActiveRun {
                id: run_id,
                cancel: cancel.clone(),
                steering,
            });
            let approver = Arc::new(super::decisions::Decisions {
                owner: state.owner.clone(),
                run: run_id,
                incarnation: state.registration.incarnation,
                cancel: cancel.clone(),
                timeout: config.timeout(),
            });
            let state = state.clone();
            tokio::spawn(async move {
                let result = crate::execution::execute_admitted(
                    &state.owner,
                    &mut run,
                    &config,
                    saved.session.workspace,
                    Arc::new(Events),
                    cancel,
                    std::future::pending(),
                    None,
                    Some(approver),
                )
                .await;
                match result {
                    Ok(result) => {
                        tracing::info!(state=?result.actual.state, cleanup_observed=result.cleanup_observed, construction_failed=result.construction_failed,"voyage run finalized")
                    }
                    Err(error) => tracing::error!("voyage execution failed: {}", error),
                }
                drop(run);
                *state.active.lock().await = None;
            });
            Ok(
                json!({"command_id":command_id,"run_id":run_id,"status":"accepted","state":"accepted","duplicate":false}),
            )
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
                    machine_id: state.actor.installation_id,
                    principal_id: state.actor.principal_id,
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
            let receipt = active.steering.submit(request).await?;
            Ok(json!({"duplicate":receipt.duplicate,"record":receipt.record}))
        }
        command @ (RuntimeCommand::Cancel { .. }
        | RuntimeCommand::Rename { .. }
        | RuntimeCommand::SetModel { .. }) => {
            let _admission = state.admission.lock().await;
            ensure!(!state.shutdown.is_cancelled(), "runtime stopping");
            let cancel_run = match &command {
                RuntimeCommand::Cancel { run_id, .. } => Some(*run_id),
                _ => None,
            };
            let receipt = state.owner.process_metadata(state.actor, command).await?;
            if let Some(run) = cancel_run
                && let Some(active) = state.active.lock().await.as_ref()
                && active.id == run
            {
                active.cancel.cancel();
            }
            Ok(receipt)
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
