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

/// Single atomic checkpoint contains current scope AND exact command receipts.
/// Callers hold the Voyage lifecycle lock and supply server-observed authority.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ScopeJournal {
    pub revision: u64,
    pub selected: Option<voyage_protocol::coordination_scope::ScopeSelection>,
    receipts: std::collections::BTreeMap<uuid::Uuid, ScopeReceipt>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeReceipt {
    selection: voyage_protocol::coordination_scope::ScopeSelection,
    result: Value,
}
impl ScopeJournal {
    pub(super) fn commit(
        path: &std::path::Path,
        command: uuid::Uuid,
        selection: voyage_protocol::coordination_scope::ScopeSelection,
        current: &voyage_protocol::coordination_scope::ScopeSelection,
        now: u64,
    ) -> Result<Value> {
        ensure!(!command.is_nil(), "invalid scope command ID");
        let mut journal: Self = if path.exists() {
            super::access::store::load_bounded(path, 1024 * 1024)?
        } else {
            Self::default()
        };
        if let Some(prior) = journal.receipts.get(&command) {
            ensure!(
                prior.selection == selection,
                "scope command payload conflict"
            );
            return Ok(prior.result.clone());
        }
        ensure!(
            journal.revision == selection.expected_revision,
            "scope revision changed"
        );
        selection
            .validate(current, now)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            journal.receipts.len() < 1024,
            "scope receipt capacity exhausted"
        );
        journal.revision = journal
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("scope revision overflow"))?;
        let result = json!({"command_id":command,"session_id":selection.session_id,
            "scope_revision":journal.revision,"status":"scope_committed"});
        journal.selected = Some(selection.clone());
        journal.receipts.insert(
            command,
            ScopeReceipt {
                selection,
                result: result.clone(),
            },
        );
        super::access::store::save_bounded(path, &journal, 1024 * 1024)?;
        Ok(result)
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use uuid::Uuid;
    use voyage_protocol::coordination_scope::{DestinationPin, ScopeSelection};
    fn current() -> ScopeSelection {
        let grant = GrantBinding {
            grant_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            revision: 1,
        };
        ScopeSelection {
            session_id: Uuid::new_v4(),
            context: grant.clone(),
            expected_revision: 0,
            destinations: ["a", "b", "c"]
                .into_iter()
                .map(|alias| DestinationPin {
                    alias: alias.into(),
                    vessel_id: Uuid::new_v4(),
                    workspace: "/work".into(),
                    grant: grant.clone(),
                    rights: vec![ProcessRight::Execute, ProcessRight::History],
                    expires_at_ms: 100,
                })
                .collect(),
        }
    }
    #[test]
    fn exact_atomic_scope_receipts_survive_all_client_disconnects() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("scope.json");
        let observed = current();
        let id = Uuid::new_v4();
        let receipt = ScopeJournal::commit(&path, id, observed.clone(), &observed, 1).unwrap();
        // Reload without any client or live relay: exact original receipt remains.
        assert_eq!(
            ScopeJournal::commit(&path, id, observed.clone(), &observed, 200).unwrap(),
            receipt
        );
        let mut conflict = observed.clone();
        conflict.destinations.remove(1);
        assert!(ScopeJournal::commit(&path, id, conflict, &observed, 1).is_err());
        let restored: ScopeJournal =
            super::super::access::store::load_bounded(&path, 1024 * 1024).unwrap();
        assert_eq!(restored.revision, 1);
        assert_eq!(restored.selected.unwrap(), observed);
    }
    #[test]
    fn abc_vs_ac_controls_never_union_contexts_or_cancel_observation() {
        let abc = current();
        let mut ac = abc.clone();
        ac.context.grant_id = Uuid::new_v4();
        ac.destinations.remove(1);
        assert!(abc.authorize_control(&ac, 1).is_err());
        let mut selected = ac.clone();
        selected.expected_revision = 0;
        selected.validate(&ac, 1).unwrap();
        selected.authorize_control(&ac, 1).unwrap();
        assert!(selected.authorize_control(&ac, 100).is_err());
        let mut forged = ac.clone();
        forged.context.revision += 1;
        assert!(selected.validate(&forged, 1).is_err());
        let mut replaced = ac.clone();
        replaced.destinations[0].vessel_id = Uuid::new_v4();
        assert!(selected.authorize_control(&replaced, 1).is_err());
        let mut revoked = ac.clone();
        revoked.destinations[0].rights.clear();
        assert!(selected.authorize_control(&revoked, 1).is_err());
        // Failed control does not mutate the owning Voyage's retained scope.
        assert_eq!(abc.destinations.len(), 3);
    }
}
