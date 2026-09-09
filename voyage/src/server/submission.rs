use super::*;
use crate::attachment::{
    journal::{SteeringActor, TurnAdmission},
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
pub(super) async fn submit(
    state: &Arc<State>,
    authorization: super::authorization::Authorization,
    command: RuntimeCommand,
) -> Result<Value> {
    let (command_id, expected_revision, expires_at_ms, prompt, operator, parts) = match command {
        RuntimeCommand::SubmitContent {
            command_id,
            expected_revision,
            expires_at_ms,
            content,
        } => {
            ensure!(content.len() <= 16, "too many content parts");
            (
                command_id,
                expected_revision,
                expires_at_ms,
                crate::images::text(&content),
                None,
                content,
            )
        }
        RuntimeCommand::Submit {
            command_id,
            expected_revision,
            expires_at_ms,
            prompt,
        } => (
            command_id,
            expected_revision,
            expires_at_ms,
            prompt,
            None,
            Vec::new(),
        ),
        RuntimeCommand::OperatorTool {
            command_id,
            expected_revision,
            expires_at_ms,
            name,
            arguments,
        } => {
            ensure!(
                !name.is_empty()
                    && name.len() <= 128
                    && serde_json::to_vec(&arguments)?.len() <= 65536,
                "operator tool command exceeds bounds"
            );
            let prompt = format!(
                "Operator tool {name}: {}",
                serde_json::to_string(&arguments)?
            );
            (
                command_id,
                expected_revision,
                expires_at_ms,
                prompt,
                Some((name, arguments)),
                Vec::new(),
            )
        }
        _ => anyhow::bail!("not submit"),
    };

    let _admission = state.admission.lock().await;
    ensure!(!state.shutdown.is_cancelled(), "runtime stopping");
    let request = TurnAdmission {
        operator_name: operator.as_ref().map(|(name, _)| name.clone()),
        command_id,
        machine_id: authorization.actor.installation_id,
        principal_id: authorization.actor.principal_id,
        session_id: state.registration.session_id,
        expected_revision,
        expires_at_ms: i64::try_from(expires_at_ms)?,
        prompt: prompt.clone(),
        parts: parts.clone(),
    };
    if let Some(run) = state.owner.lookup_turn(request.clone()).await? {
        return Ok(
            json!({"command_id":command_id,"run_id":run.id,"status":"accepted","state":run.state,"duplicate":true}),
        );
    }
    let saved = state.owner.snapshot().await?;
    let mut config = state.config.read().await.clone();
    config.model = saved
        .session
        .pending_model
        .clone()
        .unwrap_or(saved.session.model);
    ensure!(
        cfg!(target_os = "linux") || config.mcp_servers.is_empty(),
        "owned MCP process observation unsupported on this platform"
    );
    crate::policy::Policy::new(&config, saved.session.workspace.clone())?;
    let submitted_prompt = prompt.clone();
    if !parts.is_empty() {
        crate::images::validate_parts(&parts)?;
    }
    let has_images = saved
        .session
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .chain(&parts)
        .any(|p| matches!(p, voyage_protocol::content::ContentPart::Image { .. }));
    if has_images {
        let known = state
            .controls
            .resolve_model(&config, &state.registration.workspace)
            .await;
        crate::provider::validate_image_capability(
            &config.provider,
            &config.model,
            known.as_ref(),
        )?;
        let mut all = saved.session.messages.clone();
        let mut newest = crate::model::Message::new(crate::model::Role::User, &prompt);
        newest.parts = parts.clone();
        all.push(newest);
        let directory = state.directory.join("journal");
        let session_id = state.registration.session_id;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let store = crate::images::Store::open(&directory, session_id)?;
            let mut total = 0u64;
            let mut count = 0usize;
            for message in &all {
                for part in &message.parts {
                    if let voyage_protocol::content::ContentPart::Image { attachment } = part {
                        total = total.checked_add(attachment.byte_size).context("image request size overflow")?;
                        count += 1;
                        ensure!(total <= 2 * 1024 * 1024 && count <= 4,
                            "retained images exceed request limit; compact older image turns or start a new voyage");
                        store.resolve(attachment)?;
                    }
                }
            }
            Ok(())
        }).await??;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    if expected_revision == saved.revision
        && u128::from(expires_at_ms) > now
        && u128::from(expires_at_ms) - now <= 300_000
        && !command_id.is_nil()
        && (!submitted_prompt.trim().is_empty() || !parts.is_empty())
        && submitted_prompt.len() <= 64 * 1024
        && state.owner.process_receipt(command_id).await?.is_none()
    {
        state.cleanup.retry().await;
    }
    let admission = match &authorization.authority {
        Some(authority) => {
            state
                .owner
                .admit_authorized(request, authority.clone())
                .await?
        }
        None => state.owner.admit(request).await?,
    };
    let mut run = match admission {
        Admission::Existing(run) => {
            return Ok(
                json!({"command_id":command_id,"run_id":run.id,"status":"accepted","state":run.state,"duplicate":true}),
            );
        }
        Admission::New(run) => run,
    };
    let workflow_result: Result<()> = async {
        let known = state
            .controls
            .resolve_model(&config, &state.registration.workspace)
            .await;
        crate::provider::validate_inference_settings_with_model(&config, known.as_ref())?;
        if let Some(prepared) = state
            .workflows
            .take(command_id, authorization.actor.principal_id)
            .await?
        {
            ensure!(
                prepared.prompt == submitted_prompt,
                "workflow prompt changed"
            );
            run.bind_workflow(prepared.invocation, prepared.secrets)
                .await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = workflow_result {
        run.fail_before_execution()
            .await
            .map_err(|_| anyhow::anyhow!("workflow failure finalization uncertain"))?;
        return Err(error);
    }
    run.register_local_cleanup().await?;
    let run_id = run.record().await?.id;
    let execution_authority = authorization.authority.clone();
    let session_id = state.registration.session_id;
    let steering = run.enable_steering(Arc::new(
        move |candidate: SteeringActor, session: Uuid, run: Uuid| {
            ensure!(
                session == session_id && run == run_id && !candidate.principal_id.is_nil(),
                "steering authority mismatch"
            );
            if let Some(authority) = &execution_authority {
                authority.check()?;
            }
            Ok(())
        },
    ))?;
    let accepts_steering = operator.is_none();
    let cancel = CancellationToken::new();
    *state.active.lock().await = Some(ActiveRun {
        id: run_id,
        inference: super::configuration::inference_snapshot_with_model(
            &config,
            state.controls.known_model(&config).await.as_ref(),
        ),
        cancel: cancel.clone(),
        steering: accepts_steering.then_some(steering),
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
        let result = crate::execution::execute_admitted_with_controls(
            &state.owner,
            &mut run,
            &config,
            saved.session.workspace,
            Arc::new(Events),
            cancel,
            std::future::pending(),
            authorization.authority,
            Some(approver),
            state.cleanup.clone(),
            Some(state.controls.clone()),
            operator,
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
        if let Err(error) = super::suspension::suspend(&state).await {
            tracing::warn!("voyage suspension blocked: {error}");
        }
    });
    Ok(
        json!({"command_id":command_id,"run_id":run_id,"status":"accepted","state":"accepted","duplicate":false}),
    )
}
