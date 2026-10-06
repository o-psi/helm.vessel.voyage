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
        let origin = url::Url::parse(&self.target.helm_origin)?;
        ensure!(
            origin.scheme() == "https"
                && origin.host_str().is_some()
                && origin.username().is_empty()
                && origin.password().is_none()
                && origin.query().is_none()
                && origin.fragment().is_none()
                && origin.path() == "/"
                && self.target.helm_origin == origin.origin().ascii_serialization(),
            "Setup requires an exact normalized HTTPS origin"
        );
        ensure!(now_ms < self.expires_at_ms, "Setup review expired");
        for value in [
            &self.reviewed_plan_sha256,
            &self.archive_sha256,
            &self.manifest_sha256,
            &self.protected_facts_sha256,
        ] {
            ensure!(
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
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
impl Receipt {
    /// Persisted facts remain valid independently of the admission deadline.
    pub fn validate(&self) -> Result<()> {
        self.request.validate(0)?;
        match self.phase {
            Phase::Complete => ensure!(
                !self.outcome_unknown && !self.cleanup_pending,
                "Incomplete completion receipt"
            ),
            Phase::Uncertain => ensure!(
                self.effect_possible && self.outcome_unknown,
                "Invalid uncertain receipt"
            ),
            Phase::RollbackRequired => ensure!(
                self.effect_possible && self.cleanup_pending,
                "Missing rollback obligation"
            ),
            Phase::Pending => ensure!(
                self.effect_possible && self.outcome_unknown,
                "Pending intent must conservatively retain possible effects"
            ),
        }
        Ok(())
    }
}
/// Immutable duplicate lookup precedes expiry validation. A completed receipt is
/// not reinterpreted merely because its original admission window has expired.
pub fn duplicate<'a>(request: &Request, receipts: &'a [Receipt]) -> Result<Option<&'a Receipt>> {
    if let Some(receipt) = receipts.iter().find(|entry| {
        entry.request.command_id == request.command_id
            || entry.request.transaction_id == request.transaction_id
    }) {
        receipt.validate()?;
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
pub fn prepare_user(
    request: &Request,
    now_ms: u64,
    options: &crate::cli::Options,
) -> Result<crate::install::Report> {
    request.validate(now_ms)?;
    ensure!(
        request.scope == Scope::User,
        "System setup requires protected guardian admission"
    );
    ensure!(
        options.action.is_some(),
        "Choose a user installation action"
    );
    crate::flow::plan(options)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request {
            schema: 1,
            command_id: Uuid::from_u128(1),
            transaction_id: Uuid::from_u128(2),
            target: Target {
                vessel_id: Uuid::from_u128(3),
                helm_origin: "https://helm.example".into(),
                owner_principal_id: Uuid::from_u128(4),
            },
            scope: Scope::User,
            reviewed_plan_sha256: "a".repeat(64),
            archive_sha256: "b".repeat(64),
            manifest_sha256: "c".repeat(64),
            protected_facts_sha256: "d".repeat(64),
            expires_at_ms: 10,
        }
    }
    #[test]
    fn malformed_origin_digest_and_expiry_refuse() {
        let mut value = request();
        assert!(value.validate(9).is_ok());
        assert!(value.validate(10).is_err());
        for origin in [
            "https://",
            "https://user@helm.example",
            "https://helm.example/path",
            "https://helm.example?x",
            "https://helm.example#x",
            "https://HELM.example",
        ] {
            value.target.helm_origin = origin.into();
            assert!(value.validate(1).is_err(), "{origin}");
        }
        value = request();
        value.archive_sha256 = "A".repeat(64);
        assert!(value.validate(1).is_err());
    }
    #[test]
    fn retained_completion_and_collisions_are_exact() {
        let value = request();
        let receipt = Receipt {
            request: value.clone(),
            phase: Phase::Complete,
            effect_possible: true,
            outcome_unknown: false,
            cleanup_pending: false,
        };
        assert!(value.validate(100).is_err());
        assert!(
            duplicate(&value, std::slice::from_ref(&receipt))
                .unwrap()
                .is_some()
        );
        let mut changed = value.clone();
        changed.command_id = Uuid::from_u128(5);
        assert!(duplicate(&changed, std::slice::from_ref(&receipt)).is_err());
        changed = value;
        changed.archive_sha256 = "e".repeat(64);
        assert!(duplicate(&changed, std::slice::from_ref(&receipt)).is_err());
        let mut invalid = receipt;
        invalid.cleanup_pending = true;
        assert!(invalid.validate().is_err());
        invalid.phase = Phase::Uncertain;
        assert!(invalid.validate().is_err());
        invalid.outcome_unknown = true;
        assert!(invalid.validate().is_ok());
    }
    #[test]
    fn private_unknown_fields_refuse() {
        assert!(serde_json::from_value::<Target>(serde_json::json!({"vessel_id":Uuid::from_u128(1),"helm_origin":"https://helm.example","owner_principal_id":Uuid::from_u128(2),"credential":"private"})).is_err());
    }
}

/// Durable local intent admission. The caller must hold the installer operation
/// lock and authenticate the target owner separately; this journal grants nothing.
#[cfg(target_os = "linux")]
pub fn admit_intent(
    directory: &std::path::Path,
    request: &Request,
    now_ms: u64,
) -> Result<Receipt> {
    use crate::install::files;
    use std::os::unix::fs::MetadataExt;
    files::safe(directory)?;
    files::private_directory(directory)?;
    let metadata = std::fs::symlink_metadata(directory)?;
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
        "Setup journal must be private to its executing owner"
    );
    let mut receipts = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_some_and(|value| value == "json")
        {
            let metadata = std::fs::symlink_metadata(entry.path())?;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
                "Setup receipt is not private"
            );
            let receipt: Receipt = serde_json::from_slice(&files::read(&entry.path(), 65536)?)?;
            receipt.validate()?;
            receipts.push(receipt);
        }
    }
    if let Some(receipt) = duplicate(request, &receipts)? {
        return Ok(receipt.clone());
    }
    ensure!(
        !receipts
            .iter()
            .any(|receipt| receipt.phase != Phase::Complete
                || receipt.cleanup_pending
                || receipt.outcome_unknown),
        "Unresolved setup obligation blocks new effects"
    );
    request.validate(now_ms)?;
    ensure!(
        request.scope == Scope::User,
        "System setup requires protected guardian admission"
    );
    let receipt = Receipt {
        request: request.clone(),
        phase: Phase::Pending,
        effect_possible: true,
        outcome_unknown: true,
        cleanup_pending: false,
    };
    // Exclusive creation plus file/directory fsync precedes any external effect.
    files::write_new(
        &directory.join(format!("{}.json", request.command_id)),
        &serde_json::to_vec(&receipt)?,
    )?;
    Ok(receipt)
}
