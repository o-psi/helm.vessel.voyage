use super::*;
use voyage_protocol::process::RuntimeCommand;
pub(super) async fn configure(
    state: &Arc<State>,
    command: RuntimeCommand,
    authorization: super::authorization::Authorization,
) -> Result<serde_json::Value> {
    // Finish durable acceptance and live publication even if the transport waiter leaves.
    let state = state.clone();
    tokio::spawn(async move { configure_inner(&state, command, authorization).await }).await?
}
async fn configure_inner(
    state: &Arc<State>,
    command: RuntimeCommand,
    mut authorization: super::authorization::Authorization,
) -> Result<serde_json::Value> {
    ensure!(
        authorization.grant.is_none()
            || matches!(command, RuntimeCommand::SetAccountInference { .. }),
        "configuration requires executing-account owner authority"
    );
    let (RuntimeCommand::Configure {
        command_id,
        expected_revision,
        ..
    }
    | RuntimeCommand::SetAccountInference {
        command_id,
        expected_revision,
        ..
    }
    | RuntimeCommand::SetInference {
        command_id,
        expected_revision,
        ..
    }
    | RuntimeCommand::SetAccess {
        command_id,
        expected_revision,
        ..
    }) = &command
    else {
        anyhow::bail!("not configure")
    };
    let access_only = matches!(&command, RuntimeCommand::SetAccess { .. });
    let inference_only = matches!(
        &command,
        RuntimeCommand::SetInference { .. } | RuntimeCommand::SetAccountInference { .. }
    );
    let _admission = state.admission.lock().await;
    let active = state.active.lock().await.is_some();
    ensure!(
        !state.shutdown.is_cancelled() && (access_only || inference_only || !active),
        "configuration requires idle runtime"
    );
    state
        .owner
        .bind_process_command(
            *command_id,
            authorization.actor.principal_id,
            command.clone(),
        )
        .await?;
    if let Some(receipt) = state.owner.process_receipt(*command_id).await? {
        return Ok(receipt);
    }
    let mut config = match &command {
        RuntimeCommand::Configure { config_path, .. } => {
            ensure!(
                config_path.is_absolute(),
                "configuration path must be absolute"
            );
            bootstrap::load_config(Some(config_path), &state.registration.workspace)?
        }
        RuntimeCommand::SetAccountInference {
            model,
            reasoning_effort,
            service_tier,
            ..
        }
        | RuntimeCommand::SetInference {
            model,
            reasoning_effort,
            service_tier,
            ..
        } => {
            let mut config = state.config.read().await.clone();
            if let RuntimeCommand::SetAccountInference { account, .. } = &command {
                config.select_account(account.clone())?;
            }
            config.model = model.clone();
            config.reasoning_effort = reasoning_effort.clone();
            config.service_tier = service_tier.clone();
            crate::provider::validate_inference_settings(&config)?;
            config
        }
        RuntimeCommand::SetAccess { access, .. } => {
            let mut config = state.config.read().await.clone();
            let mode = match access.as_str() {
                "read-only" => crate::config::AccessMode::ReadOnly,
                "approval" => crate::config::AccessMode::Approval,
                "unrestricted" => crate::config::AccessMode::Unrestricted,
                _ => anyhow::bail!("use read-only, approval or unrestricted"),
            };
            // Validate existing sources before applying the one explicit override.
            crate::runtime_policy::RuntimePolicy::resolve(&config, &state.registration.workspace)?;
            {
                let session = state.owner.snapshot().await?.session;
                config.model = session.pending_model.unwrap_or(session.model);
            }
            config.access = Some(mode);
            config.policy_explicit.access = Some(mode);
            if let Some(selection) = config.policy_profile.take() {
                let mut request = selection.request().clone();
                request.explicit.access = Some(mode);
                let preview = crate::policy_profile::selection::Selection::preview(
                    &config,
                    &state.registration.workspace,
                    &request,
                )?;
                // Owner's confirmed command changes only access; other profile fields stay bound.
                config.policy_profile = Some(crate::policy_profile::selection::Selection::bind(
                    &config,
                    &state.registration.workspace,
                    request,
                    Some(&preview.transition_digest),
                )?);
            }
            config
        }
        _ => unreachable!(),
    };
    super::authorization::account_authority(state, &mut authorization, &config)?;
    bootstrap::limit_participant(&mut config, &state.registration)?;
    let known = if access_only {
        state.controls.known_model(&config).await
    } else {
        state
            .controls
            .resolve_model(&config, &state.registration.workspace)
            .await
    };
    crate::provider::validate_inference_settings_with_model(&config, known.as_ref())?;
    if let RuntimeCommand::SetAccess { access, .. } = &command {
        ensure!(
            config.access_mode().to_string() == *access,
            "requested access exceeds parent policy"
        );
        let effective =
            crate::runtime_policy::RuntimePolicy::resolve(&config, &state.registration.workspace)?;
        ensure!(
            serde_json::to_value(effective.policy().access_mode())? == *access,
            "requested access is restricted by the executing machine or parent policy"
        );
    }
    if access_only {
        // A live override must not smuggle in changed roots, environment, or other rules.
        let previous = state.config.read().await;
        let before = crate::runtime_policy::RuntimePolicy::resolve(
            &previous,
            &state.registration.workspace,
        )?;
        let after =
            crate::runtime_policy::RuntimePolicy::resolve(&config, &state.registration.workspace)?;
        let mut rules = before.policy().effective().rules().clone();
        rules.access = after.policy().effective().rules().access;
        ensure!(
            rules == *after.policy().effective().rules(),
            "access update changed other policy rules; wait for idle and reconfigure"
        );
    }
    let effective_access =
        crate::runtime_policy::RuntimePolicy::resolve(&config, &state.registration.workspace)?
            .policy()
            .access_mode();
    let secrets = crate::build::redactor(&config);
    crate::provider::validate_models_for_display(
        &[crate::provider::ModelInfo::minimal(config.model.clone())],
        |value| secrets.contains_secret(value),
    )?;
    let launch =
        crate::launch_config::LaunchConfig::capture(&config, &state.registration.workspace)?;
    if access_only {
        state
            .owner
            .check_access_revision(*expected_revision)
            .await?;
    } else {
        ensure!(
            state.owner.snapshot().await?.revision == *expected_revision,
            "session revision conflict"
        );
    }
    if !inference_only && !active && !launch.matches_config(&*state.config.read().await)? {
        state
            .controls
            .close_for_command(&state.owner, &command)
            .await?;
    }
    let mut current = state.config.write().await;
    let receipt = state
        .owner
        .configure(
            command,
            serde_json::to_string(&launch)?,
            config.model.clone(),
        )
        .await?;
    // Publish only after durable acceptance. Stale approvals fail closed.
    if let Some(live) = &current.live_access {
        if !inference_only {
            live.update(effective_access);
        }
        config.live_access = Some(live.clone());
    }
    config.vessel_context = current.vessel_context.clone();
    config.artifact_scope = current.artifact_scope.clone();
    *current = config;
    Ok(receipt)
}

/// Public, non-secret settings projection. These are requested values, not proof of
/// an account entitlement or of the tier ultimately delivered by a provider.
pub(super) fn inference_snapshot(config: &Config) -> serde_json::Value {
    inference_snapshot_with_model(config, None)
}
pub(super) fn inference_snapshot_with_model(
    config: &Config,
    model: Option<&crate::provider::ModelInfo>,
) -> serde_json::Value {
    let resolution = crate::provider::resolve_inference(config, model);
    let reasoning_efforts = &resolution.thinking.values;
    let service_tiers = &resolution.service.values;
    serde_json::json!({
        "resolution": resolution,
        "model": config.model,
        "account": config.account,
        "reasoning_effort": config.reasoning_effort,
        "service_tier": config.service_tier,
        "provider": config.provider,
        "reasoning_efforts": reasoning_efforts,
        "service_tiers": service_tiers,
    })
}
