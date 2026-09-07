//! Owner-authorized GitHub actions execute in an admitted run without a model call.
use super::*;
use crate::attachment::{journal::TurnAdmission, runtime::Admission};
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;
mod operation;
pub(super) async fn submit(
    state: &Arc<State>,
    authorization: super::authorization::Authorization,
    command: RuntimeCommand,
) -> Result<Value> {
    ensure!(
        authorization.grant.is_none(),
        "GitHub operator requires executing-account authority"
    );
    let RuntimeCommand::Github {
        command_id,
        expected_revision,
        expires_at_ms,
        words,
    } = command
    else {
        anyhow::bail!("not GitHub command")
    };
    ensure!(
        !words.is_empty()
            && words.len() <= 64
            && words.iter().map(String::len).sum::<usize>() <= 65536,
        "invalid GitHub command bounds"
    );
    let _admission = state.admission.lock().await;
    ensure!(
        !state.shutdown.is_cancelled() && state.active.lock().await.is_none(),
        "GitHub action requires idle voyage"
    );
    if let Some(receipt) = state.owner.process_receipt(command_id).await? {
        return Ok(receipt);
    }
    let config = state.config.read().await.clone();
    let resolved =
        crate::runtime_policy::RuntimePolicy::resolve(&config, &state.registration.workspace)?;
    let request = TurnAdmission {
        operator_name: None,
        command_id,
        machine_id: authorization.actor.installation_id,
        principal_id: authorization.actor.principal_id,
        session_id: state.registration.session_id,
        expected_revision,
        expires_at_ms: i64::try_from(expires_at_ms)?,
        prompt: format!(
            "GitHub operator command: {}",
            serde_json::to_string(&words)?
        ),
    };
    let mut run = match state.owner.admit(request).await? {
        Admission::Existing(run) => {
            return Ok(
                json!({"command_id":command_id,"run_id":run.id,"status":"accepted","duplicate":true}),
            );
        }
        Admission::New(run) => run,
    };
    run.register_local_cleanup().await?;
    let run_id = run.record().await?.id;
    let cancel = CancellationToken::new();
    *state.active.lock().await = Some(ActiveRun {
        id: run_id,
        cancel: cancel.clone(),
        steering: None,
    });
    let context = crate::tools::ToolContext {
        github: crate::github::Credential::from_config(resolved.config()),
        completion: None,
        policy: Arc::new(resolved.policy().clone()),
        approver: Arc::new(super::decisions::Decisions {
            owner: state.owner.clone(),
            run: run_id,
            incarnation: state.registration.incarnation,
            cancel: cancel.clone(),
            timeout: config.timeout(),
        }),
        timeout: config.timeout(),
        max_output_bytes: config.max_output_bytes,
        environment: crate::build::tool_environment(resolved.config()),
        cancellation: cancel.clone(),
        execution_id: run_id,
        interaction: crate::tools::InteractionMode::Attended,
        redactor: crate::build::redactor(resolved.config()),
    };
    let state = state.clone();
    tokio::spawn(async move {
        let result = operation::run(&state, &mut run, context, words, cancel).await;
        if let Err(error) = result {
            tracing::error!("GitHub operator finalization failed: {}", error);
        }
        drop(run);
        *state.active.lock().await = None;
        if let Err(error) = super::suspension::suspend(&state).await {
            tracing::warn!("voyage suspension blocked: {error}");
        }
    });
    Ok(json!({"command_id":command_id,"run_id":run_id,"status":"accepted","state":"accepted"}))
}
