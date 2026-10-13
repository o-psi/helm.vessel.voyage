//! Independent Voyage owns selected scope and exact receipts, not its Helm.
//! Context facts are read from a private trusted registry; request pins are never authority.
use super::*;
use crate::attachment::local_actor::storage::Directory;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use voyage_protocol::coordination_scope::ScopeSelection;
const MAX: usize = 65_536;
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    revision: u64,
    selected: Option<ScopeSelection>,
    receipts: std::collections::BTreeMap<Uuid, Receipt>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    selection: ScopeSelection,
    result: Value,
}

fn storage(state: &State) -> Result<Directory> {
    Directory::open(&state.directory.join("coordination-scope"))
}
fn load(storage: &Directory) -> Result<Journal> {
    storage
        .read_bounded("journal.json", MAX)?
        .map(|bytes| serde_json::from_slice(&bytes))
        .transpose()
        .map(|v| v.unwrap_or_default())
        .map_err(Into::into)
}
fn current(
    state: &State,
    auth: &super::authorization::Authorization,
    revision: u64,
) -> Result<ScopeSelection> {
    let binding = auth
        .grant
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("explicit actor context required"))?;
    let source = auth
        .scope_source
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("current actor authority unavailable"))?;
    let grant = source.current()?;
    ensure!(
        grant.grant_id == binding.grant_id
            && grant.revision == binding.revision
            && grant.principal_id == binding.principal_id
            && grant.session_id == state.registration.session_id,
        "actor scope changed"
    );
    // This registry must be provisioned through authenticated host-side setup,
    // never by a model tool or a public scope request.
    let dir = Directory::open_existing(&state.directory.join("coordination-contexts"))?;
    let name = format!("{}-{}.json", binding.grant_id, binding.revision);
    let bytes = dir
        .read_bounded(&name, MAX)?
        .ok_or_else(|| anyhow::anyhow!("destination context not provisioned"))?;
    let record: ProvisionRecord = serde_json::from_slice(&bytes)?;
    let mut context = record.observed;
    ensure!(
        context.context == *binding && context.session_id == grant.session_id,
        "destination context identity mismatch"
    );
    context.expected_revision = revision;
    Ok(context)
}
pub(super) async fn read(
    state: &Arc<State>,
    auth: &super::authorization::Authorization,
) -> Result<Value> {
    let storage = storage(state)?;
    let _lock = storage.lock()?;
    let journal = load(&storage)?;
    let context = current(state, auth, journal.revision)?;
    Ok(json!({"scope_revision":journal.revision,"selected":journal.selected,"available":context}))
}
pub(super) async fn commit(
    state: &Arc<State>,
    auth: &super::authorization::Authorization,
    command: Uuid,
    selection: ScopeSelection,
) -> Result<Value> {
    let _admission = state.admission.lock().await;
    ensure!(
        state.active.lock().await.is_none(),
        "scope update requires idle Voyage"
    );
    ensure!(!command.is_nil(), "scope command ID required");
    let storage = storage(state)?;
    let _lock = storage.lock()?;
    let mut journal = load(&storage)?;
    let context = current(state, auth, journal.revision)?;
    if let Some(prior) = journal.receipts.get(&command) {
        ensure!(
            prior.selection == selection && prior.selection.context == context.context,
            "scope command payload or context conflict"
        );
        return Ok(prior.result.clone());
    }
    selection
        .validate(
            &context,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
                .try_into()?,
        )
        .map_err(anyhow::Error::msg)?;
    ensure!(
        journal.receipts.len() < 128,
        "scope receipt capacity exhausted"
    );
    journal.revision = journal
        .revision
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("scope revision overflow"))?;
    let result = json!({"command_id":command,"status":"scope_committed","session_id":selection.session_id,"scope_revision":journal.revision});
    journal.selected = Some(selection.clone());
    journal.receipts.insert(
        command,
        Receipt {
            selection,
            result: result.clone(),
        },
    );
    let bytes = serde_json::to_vec(&journal)?;
    ensure!(bytes.len() <= MAX, "scope journal capacity exhausted");
    storage.publish("journal.json", &bytes)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_checkpoint_keeps_exact_scope_receipt_across_reopen() {
        let root = tempfile::tempdir().unwrap();
        let dir = Directory::open(root.path()).unwrap();
        let id = Uuid::new_v4();
        let context = voyage_protocol::process::GrantBinding {
            grant_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            revision: 7,
        };
        let selection = ScopeSelection {
            session_id: Uuid::new_v4(),
            context,
            expected_revision: 0,
            destinations: Vec::new(),
        };
        let mut journal = Journal {
            revision: 1,
            selected: Some(selection.clone()),
            ..Default::default()
        };
        journal.receipts.insert(
            id,
            Receipt {
                selection: selection.clone(),
                result: json!({"command_id":id,"scope_revision":1}),
            },
        );
        dir.publish("journal.json", &serde_json::to_vec(&journal).unwrap())
            .unwrap();
        drop(dir);
        let reopened = Directory::open_existing(root.path()).unwrap();
        let restored = load(&reopened).unwrap();
        assert_eq!(restored.selected.unwrap(), selection);
        assert_eq!(restored.receipts[&id].result["scope_revision"], 1);
        assert_eq!(restored.receipts[&id].selection, selection);
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvisionRecord {
    command_id: Uuid,
    context: voyage_protocol::process::GrantBinding,
    routes: Vec<voyage_protocol::coordination_scope::RouteReference>,
    observed: ScopeSelection,
}

pub(super) async fn provision(
    state: &Arc<State>,
    auth: &super::authorization::Authorization,
    command_id: Uuid,
    context: voyage_protocol::process::GrantBinding,
    routes: Vec<voyage_protocol::coordination_scope::RouteReference>,
) -> Result<Value> {
    // A paired human connection is NOT host authority to import bearer material.
    ensure!(
        auth.grant.is_none(),
        "destination provisioning requires executing host owner authority"
    );
    ensure!(
        !command_id.is_nil() && !context.grant_id.is_nil() && routes.len() <= 32,
        "invalid provisioning identity or capacity"
    );
    let _admission = state.admission.lock().await;
    ensure!(
        state.active.lock().await.is_none(),
        "provisioning requires idle Voyage"
    );
    // Owner approves a destination selection, not an invented execution identity.
    // Authenticate the exact existing local process grant before any remote read.
    ensure!(
        state.registration.peer_uids.is_none(),
        "protected context provisioning requires identity-scoped broker adapter"
    );
    let root = state
        .directory
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| anyhow::anyhow!("supervised grant root unavailable"))?;
    let grant_path = root
        .join("access/grants")
        .join(format!("{}.json", context.grant_id));
    super::authorization::read_current(&grant_path, &context, state.registration.session_id)?;

    let directory = Directory::open(&state.directory.join("coordination-contexts"))?;
    let _lock = directory.lock()?;
    let operation = format!("receipt-{command_id}.json");
    if let Some(bytes) = directory.read_bounded(&operation, MAX)? {
        let prior: ProvisionRecord = serde_json::from_slice(&bytes)?;
        ensure!(
            prior.context == context && prior.routes == routes,
            "provision command payload conflict"
        );
        directory.publish(
            &format!("{}-{}.json", context.grant_id, context.revision),
            &serde_json::to_vec(&prior)?,
        )?;
        return Ok(
            json!({"command_id":command_id,"status":"context_provisioned","context":context}),
        );
    }
    let mut destinations = Vec::new();
    for route in &routes {
        ensure!(
            route.credential_path.is_absolute(),
            "private route reference must be absolute"
        );
        let bytes = crate::tools::vessel::read_setup_credential(&route.credential_path)?;
        let credential: voyage_protocol::process::WorkspaceCredential =
            serde_json::from_slice(&bytes)?;
        ensure!(
            credential.principal_id == context.principal_id,
            "route principal differs from actor context"
        );
        let capabilities =
            crate::tools::vessel::observe_setup_route(&route.credential_path).await?;
        ensure!(
            capabilities["vessel_id"] == credential.vessel_id.to_string(),
            "destination identity changed"
        );
        let revision = capabilities["grant_revision"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("route grant revision unavailable"))?;
        let expires_at_ms = capabilities["expires_at_ms"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("route expiry unavailable"))?;
        let rights = serde_json::from_value(capabilities["rights"].clone())?;
        let workspaces: Vec<voyage_protocol::process::ApprovedWorkspace> =
            serde_json::from_value(capabilities["workspaces"].clone())?;
        ensure!(
            workspaces.len() == 1,
            "provisioning requires one exact approved destination workspace"
        );
        destinations.push(voyage_protocol::coordination_scope::DestinationPin {
            alias: route.alias.clone(),
            vessel_id: credential.vessel_id,
            workspace: workspaces[0].path.clone(),
            grant: voyage_protocol::process::GrantBinding {
                grant_id: credential.grant_id,
                principal_id: credential.principal_id,
                revision,
            },
            rights,
            expires_at_ms,
        });
    }
    let observed = ScopeSelection {
        session_id: state.registration.session_id,
        context: context.clone(),
        expected_revision: 0,
        destinations,
    };
    observed
        .validate(
            &observed,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
                .try_into()?,
        )
        .map_err(anyhow::Error::msg)?;
    let record = ProvisionRecord {
        command_id,
        context: context.clone(),
        routes,
        observed: observed.clone(),
    };
    let bytes = serde_json::to_vec(&record)?;
    ensure!(bytes.len() <= MAX, "provisioning record capacity exhausted");
    directory.publish_new(&operation, &bytes)?;
    directory.publish(
        &format!("{}-{}.json", context.grant_id, context.revision),
        &bytes,
    )?;
    Ok(json!({"command_id":command_id,"status":"context_provisioned","context":context}))
}
