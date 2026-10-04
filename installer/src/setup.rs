//! Additive setup DTOs and user-plan preparation. Not a privileged entrypoint.
//! Journal storage and authenticated admission must be supplied by the local owner.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub vessel_id: Uuid,
    pub helm_origin: String,
    pub owner_principal_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    User,
    System,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u32,
    pub command_id: Uuid,
    pub transaction_id: Uuid,
    pub target: Target,
    pub scope: Scope,
    /// Digest of versioned canonical reviewed plan, not free-form screen text.
    pub reviewed_plan_sha256: String,
    pub archive_sha256: String,
    pub manifest_sha256: String,
    pub protected_facts_sha256: String,
    pub expires_at_ms: u64,
}
impl Request {
    pub fn validate(&self, now_ms: u64) -> Result<()> {
        ensure!(self.schema == 1, "Unsupported setup schema");
        ensure!(
            !self.command_id.is_nil()
                && !self.transaction_id.is_nil()
                && !self.target.vessel_id.is_nil()
                && !self.target.owner_principal_id.is_nil(),
            "Missing setup identity"
        );
        ensure!(
            self.target.helm_origin.starts_with("https://")
                && !self.target.helm_origin.contains(['\r', '\n']),
            "Setup requires the reviewed HTTPS Helm origin"
        );
        ensure!(now_ms < self.expires_at_ms, "Setup review expired");
        for value in [
            &self.reviewed_plan_sha256,
            &self.archive_sha256,
            &self.manifest_sha256,
            &self.protected_facts_sha256,
        ] {
            ensure!(
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "Invalid setup digest"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Pending,
    Uncertain,
    Complete,
    RollbackRequired,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub request: Request,
    pub phase: Phase,
    pub effect_possible: bool,
    pub outcome_unknown: bool,
    pub cleanup_pending: bool,
}
/// Immutable duplicate lookup precedes expiry validation. A completed receipt is
/// not reinterpreted merely because its original admission window has expired.
pub fn duplicate<'a>(request: &Request, receipts: &'a [Receipt]) -> Result<Option<&'a Receipt>> {
    if let Some(receipt) = receipts
        .iter()
        .find(|entry| entry.request.command_id == request.command_id)
    {
        ensure!(
            &receipt.request == request,
            "Setup command reused with different content"
        );
        return Ok(Some(receipt));
    }
    Ok(None)
}
/// Effect-free ordinary-user preparation through the existing installer flow.
/// This validates source/options but does not authenticate a browser request,
/// publish a durable intent, execute an installation or claim service readiness.
pub fn prepare_user(options: &crate::cli::Options) -> Result<crate::install::Report> {
    ensure!(
        options.action.is_some(),
        "Choose a user installation action"
    );
    crate::flow::plan(options)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_unknown_fields_refuse() {
        assert!(serde_json::from_value::<Target>(serde_json::json!({"vessel_id":Uuid::from_u128(1),"helm_origin":"https://helm.example","owner_principal_id":Uuid::from_u128(2),"credential":"private"})).is_err());
    }
}
