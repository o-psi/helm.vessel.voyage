//! Read-only local credential diagnostics; enrollment belongs to Vessel.
use anyhow::Result;
pub(crate) fn chatgpt_token_store() -> Result<helm::provider::ChatGptTokenStore> {
    Ok(helm::provider::ChatGptTokenStore::new(
        helm::provider::ChatGptTokenStore::default_path()?,
    ))
}

pub(crate) fn token_status_json(status: &helm::provider::TokenStatus) -> serde_json::Value {
    serde_json::json!({
        "authenticated": status.authenticated,
        "expires_at": status.expires_at,
        "refreshable": status.refreshable,
    })
}
