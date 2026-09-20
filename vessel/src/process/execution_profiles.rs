//! Host-local execution preferences; starts retain resolved values, never references.
use super::{accounts::Scope, database, service::Supervisor};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::path::Path;
use uuid::Uuid;
use voyage_protocol::{
    execution_profiles::{ExecutionProfile, ProfileCatalogue},
    process::{ProcessRight, VesselCommand},
};
use voyage_runtime::accounts::Registry;

fn can_manage(scope: &Scope) -> bool {
    matches!(scope, Scope::Owner) || matches!(scope, Scope::Connection(g) if g.full_access)
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value == value.trim()
        && value.len() <= max
        && !value.chars().any(char::is_control)
}
fn validate(profile: &ExecutionProfile) -> Result<()> {
    ensure!(!profile.id.is_nil(), "invalid profile identity");
    ensure!(valid_text(&profile.name, 80), "invalid profile name");
    ensure!(valid_text(&profile.model, 256), "invalid profile model");
    for value in [&profile.reasoning_effort, &profile.service_tier]
        .into_iter()
        .flatten()
    {
        ensure!(valid_text(value, 64), "invalid profile setting");
    }
    Ok(())
}
fn validate_host(profile: &ExecutionProfile) -> Result<()> {
    validate(profile)?;
    let mut config = voyage_runtime::Config::load(None)?;
    config.select_account(profile.account.clone())?;
    config.model = profile.model.clone();
    config.reasoning_effort = profile.reasoning_effort.clone();
    config.service_tier = profile.service_tier.clone();
    config.validate_account()?;
    voyage_runtime::provider::validate_inference_settings(&config)?;
    let redactor = voyage_runtime::build::redactor(&config);
    ensure!(
        [&profile.name, &profile.model]
            .into_iter()
            .all(|s| !redactor.contains_secret(s)),
        "profile text contains private account material"
    );
    Ok(())
}

fn bootstrap() -> Result<Option<ExecutionProfile>> {
    let (_, account) = Registry::default_host()?.default_account()?;
    let Some(account) = account else {
        return Ok(None);
    };
    let mut config = voyage_runtime::Config::load(None)?;
    config.select_account(account.clone())?;
    let profile = ExecutionProfile {
        id: Uuid::new_v4(),
        name: "Default".into(),
        account,
        model: config.model,
        reasoning_effort: config.reasoning_effort,
        service_tier: config.service_tier,
    };
    validate_host(&profile)?;
    Ok(Some(profile))
}

// The catalogue and exact, actor-bound command receipts commit together. No
// process-local lock can protect concurrent supervisor connections on its own.
fn transact(
    root: &Path,
    seed: Option<ExecutionProfile>,
    mutation: Option<(&VesselCommand, &str)>,
) -> Result<ProfileCatalogue> {
    transact_validated(root, seed, mutation, || Ok(()))
}

fn transact_validated(
    root: &Path,
    seed: Option<ExecutionProfile>,
    mutation: Option<(&VesselCommand, &str)>,
    validate_new: impl FnOnce() -> Result<()>,
) -> Result<ProfileCatalogue> {
    let mut db = database::open(root)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS execution_profiles (id INTEGER PRIMARY KEY CHECK(id=1), state TEXT NOT NULL CHECK(json_valid(state))) STRICT;
        CREATE TABLE IF NOT EXISTS execution_profile_commands (id TEXT PRIMARY KEY, request TEXT NOT NULL, result TEXT NOT NULL CHECK(json_valid(result))) STRICT;")?;
    let saved: Option<String> = tx
        .query_row("SELECT state FROM execution_profiles WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()?;
    let mut catalogue: ProfileCatalogue = saved
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    if saved.is_none()
        && let Some(profile) = seed
    {
        catalogue.default_profile_id = Some(profile.id);
        catalogue.profiles.push(profile);
        catalogue.revision = 1;
    }
    let mut receipt = None;
    if let Some((command, actor)) = mutation {
        let (id, expected) = match command {
            VesselCommand::SaveProfile {
                command_id,
                expected_revision,
                ..
            }
            | VesselCommand::DeleteProfile {
                command_id,
                expected_revision,
                ..
            }
            | VesselCommand::SetDefaultProfile {
                command_id,
                expected_revision,
                ..
            } => (*command_id, *expected_revision),
            _ => anyhow::bail!("invalid profile mutation"),
        };
        ensure!(!id.is_nil(), "invalid profile command identity");
        let request = serde_json::to_string(&json!({"actor":actor,"command":command}))?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT request,result FROM execution_profile_commands WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((original, result)) = old {
            ensure!(original == request, "profile command identity conflict");
            return Ok(serde_json::from_str(&result)?);
        }
        ensure!(
            catalogue.revision == expected,
            "profiles changed; reload before editing"
        );
        validate_new()?;
        let count: u64 =
            tx.query_row("SELECT count(*) FROM execution_profile_commands", [], |r| {
                r.get(0)
            })?;
        ensure!(count < 4096, "profile command storage limit reached");
        match command {
            VesselCommand::SaveProfile {
                profile,
                make_default,
                ..
            } => {
                validate(profile)?;
                ensure!(
                    !catalogue.profiles.iter().any(|p| p.id != profile.id
                        && p.name.to_lowercase() == profile.name.to_lowercase()),
                    "profile name already exists"
                );
                if let Some(existing) = catalogue.profiles.iter_mut().find(|p| p.id == profile.id) {
                    *existing = profile.clone();
                } else {
                    ensure!(catalogue.profiles.len() < 64, "profile limit reached");
                    catalogue.profiles.push(profile.clone());
                }
                if *make_default || catalogue.default_profile_id.is_none() {
                    catalogue.default_profile_id = Some(profile.id);
                }
            }
            VesselCommand::DeleteProfile { profile_id, .. } => {
                ensure!(
                    catalogue.profiles.iter().any(|p| p.id == *profile_id),
                    "profile no longer exists"
                );
                catalogue.profiles.retain(|p| p.id != *profile_id);
                if catalogue.default_profile_id == Some(*profile_id) {
                    catalogue.default_profile_id = catalogue.profiles.first().map(|p| p.id);
                }
            }
            VesselCommand::SetDefaultProfile { profile_id, .. } => {
                ensure!(
                    catalogue.profiles.iter().any(|p| p.id == *profile_id),
                    "profile no longer exists"
                );
                catalogue.default_profile_id = Some(*profile_id);
            }
            _ => unreachable!(),
        }
        catalogue.revision = catalogue
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("profile revision exhausted"))?;
        receipt = Some((id, request));
    }
    let state = serde_json::to_string(&catalogue)?;
    // Do not mark an unconfigured host initialized: first account setup can
    // still bootstrap. An explicit last-profile deletion does persist emptiness.
    if saved.is_some() || catalogue.revision > 0 {
        tx.execute("INSERT INTO execution_profiles VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state", [&state])?;
    }
    if let Some((id, request)) = receipt {
        tx.execute(
            "INSERT INTO execution_profile_commands VALUES(?1,?2,?3)",
            params![id.to_string(), request, state],
        )?;
    }
    tx.commit()?;
    Ok(catalogue)
}

fn filter_catalogue(
    catalogue: &mut ProfileCatalogue,
    registry: &Registry,
    scope: &Scope,
    workspace: &Path,
) {
    catalogue.profiles.retain(|p| {
        scope.account_allowed(
            registry,
            p.account.account_id,
            p.account.connection_id,
            workspace,
        )
    });
    catalogue.default_profile_id = catalogue
        .default_profile_id
        .filter(|id| catalogue.profiles.iter().any(|p| p.id == *id));
    catalogue.can_manage = can_manage(scope);
}

impl Supervisor {
    pub(super) fn profile_catalogue(
        &self,
        workspace: &Path,
        scope: &Scope,
    ) -> Result<ProfileCatalogue> {
        scope.check(&self.directory, workspace, ProcessRight::AccountUse)?;
        let seed = if can_manage(scope) {
            bootstrap().ok().flatten()
        } else {
            None
        };
        let mut catalogue = transact(&self.directory, seed, None)?;
        let registry = Registry::default_host()?;
        filter_catalogue(&mut catalogue, &registry, scope, workspace);
        scope.check(&self.directory, workspace, ProcessRight::AccountUse)?;
        Ok(catalogue)
    }
    pub(super) fn execution_profiles(&self, command: VesselCommand, scope: Scope) -> Result<Value> {
        let workspace = match &command {
            VesselCommand::Profiles { workspace }
            | VesselCommand::SaveProfile { workspace, .. }
            | VesselCommand::DeleteProfile { workspace, .. }
            | VesselCommand::SetDefaultProfile { workspace, .. } => workspace,
            _ => anyhow::bail!("invalid profile operation"),
        };
        scope.check(&self.directory, workspace, ProcessRight::AccountUse)?;
        if matches!(command, VesselCommand::Profiles { .. }) {
            return Ok(serde_json::to_value(
                self.profile_catalogue(workspace, &scope)?,
            )?);
        }
        ensure!(
            can_manage(&scope),
            "profile management requires a full-access human connection"
        );
        let actor = scope.actor(workspace).principal;
        let mut result =
            transact_validated(&self.directory, None, Some((&command, &actor)), || {
                scope.check(&self.directory, workspace, ProcessRight::AccountUse)?;
                if let VesselCommand::SaveProfile { profile, .. } = &command {
                    scope.use_account(&self.directory, workspace, &profile.account)?;
                    validate_host(profile)?;
                }
                Ok(())
            })?;
        result.can_manage = true;
        scope.check(&self.directory, workspace, ProcessRight::AccountUse)?;
        Ok(serde_json::to_value(result)?)
    }
}

#[cfg(test)]
#[path = "execution_profiles_tests.rs"]
mod tests;
