//! Preparation shares account/profile validation, never writes frozen launch state.
use super::{accounts::Scope, service::Supervisor};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use voyage_protocol::{coordination_scope::PreparedLaunch, process::*};

impl Supervisor {
    pub(super) async fn prepare_start_settings(
        &self,
        command: VesselCommand,
        scope: Scope,
    ) -> Result<Value> {
        let VesselCommand::PrepareStartSettings {
            workspace,
            mut settings,
            mut binding,
            profile,
        } = command
        else {
            anyhow::bail!("invalid launch preparation");
        };
        let right = if matches!(scope, Scope::Session(_)) {
            ProcessRight::Lifecycle
        } else {
            ProcessRight::Create
        };
        ensure!(workspace.is_absolute(), "target workspace must be absolute");
        let workspace = workspace.canonicalize()?;
        ensure!(workspace.is_dir(), "target workspace is not a directory");
        scope.check(&self.directory, &workspace, right)?;
        scope.check(&self.directory, &workspace, ProcessRight::AccountUse)?;
        if let Some(pin) = &profile {
            ensure!(
                binding.is_none(),
                "profile and explicit account are mutually exclusive"
            );
            let catalogue = {
                #[cfg(target_os = "linux")]
                {
                    if super::runtime_storage::has_bound_layout(&self.directory) {
                        self.identity_profile_catalogue(&workspace, &scope).await?
                    } else {
                        self.profile_catalogue(&workspace, &scope)?
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    self.profile_catalogue(&workspace, &scope)?
                }
            };
            ensure!(
                catalogue.revision == pin.revision,
                "profile revision changed"
            );
            let selected = catalogue
                .profiles
                .into_iter()
                .find(|p| p.id == pin.profile_id)
                .ok_or_else(|| anyhow::anyhow!("profile unavailable"))?;
            binding = Some(selected.account);
            settings.model.get_or_insert(selected.model);
            settings
                .reasoning_effort
                .get_or_insert(selected.reasoning_effort);
            settings.service_tier.get_or_insert(selected.service_tier);
        }
        // Explicit selection prevents a preview silently depending on another host's defaults.
        let account =
            binding.ok_or_else(|| anyhow::anyhow!("exact account or profile required"))?;
        let preview =
            {
                #[cfg(target_os = "linux")]
                {
                    if super::runtime_storage::has_bound_layout(&self.directory) {
                        let projection = self.identity_account_scope(&scope, &workspace)?;
                        self.identity_account_helper(&scope, &workspace, ProcessRight::AccountUse,
                    voyage_protocol::identity_helper::IdentityHelperOperation::PrepareLaunch {
                        scope: projection, account: account.clone(), settings: settings.clone()
                    }).await?
                    } else {
                        ordinary_preview(&self.directory, &scope, &workspace, &account, &settings)?
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    ordinary_preview(&self.directory, &scope, &workspace, &account, &settings)?
                }
            };
        scope.check(&self.directory, &workspace, right)?;
        scope.check(&self.directory, &workspace, ProcessRight::AccountUse)?;
        Ok(serde_json::to_value(PreparedLaunch {
            vessel_id: super::identity::public(&self.directory)?.vessel_id,
            workspace,
            account,
            settings: serde_json::from_value(preview["settings"].clone())?,
            profile,
            execution_authorized: false,
        })?)
    }
}

fn ordinary_preview(
    root: &std::path::Path,
    scope: &Scope,
    workspace: &std::path::Path,
    account: &voyage_protocol::accounts::AccountBinding,
    settings: &voyage_protocol::start_settings::StartSettings,
) -> Result<Value> {
    scope.use_account(root, workspace, account)?;
    let mut config = voyage_runtime::Config::load(None)?;
    config.select_account(account.clone())?;
    voyage_runtime::start_settings::apply(&mut config, settings)?;
    let portable = voyage_runtime::start_settings::portable(&config, config.access_mode());
    ensure!(
        !voyage_runtime::build::redactor(&config)
            .contains_secret(&serde_json::to_string(&portable)?),
        "launch preferences contain private data"
    );
    Ok(json!({"settings":portable}))
}
