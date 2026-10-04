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
    let mut context: ScopeSelection = serde_json::from_slice(&bytes)?;
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
