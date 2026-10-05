//! Native transaction admission: durable intent before service/binary effects.
//! OS adapters must persist every transition before acting. This is not an installer.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasePin {
    pub target: String,
    pub version: String,
    pub manifest_sha256: String,
    pub source_sha: String,
}
impl ReleasePin {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(
                self.target.as_str(),
                "x86_64-apple-darwin" | "x86_64-pc-windows-msvc"
            ),
            "Unsupported native target"
        );
        ensure!(
            semver::Version::parse(&self.version).is_ok(),
            "Invalid pinned version"
        );
        ensure!(
            hex(&self.manifest_sha256, 64) && hex(&self.source_sha, 40),
            "Invalid release identity"
        );
        Ok(())
    }
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Reviewed,
    PublishIntent,
    Published,
    ActivationIntent,
    Ready,
    RecoveryRequired,
    Restored,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema_version: u32,
    pub operation_id: String,
    /// Exact native identity attestation, never translated from Linux UID/GID.
    pub owner_identity: String,
    pub previous: Option<ReleasePin>,
    pub candidate: ReleasePin,
    pub prior_definition_sha256: Option<String>,
    pub candidate_definition_sha256: String,
    pub prior_active: bool,
    pub phase: Phase,
}
impl Receipt {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && uuid::Uuid::parse_str(&self.operation_id).is_ok(),
            "Invalid native receipt"
        );
        ensure!(
            !self.owner_identity.is_empty()
                && self.owner_identity.len() <= 256
                && !self.owner_identity.chars().any(char::is_control),
            "Invalid native owner identity"
        );
        self.candidate.validate()?;
        if let Some(previous) = &self.previous {
            previous.validate()?;
            ensure!(
                previous.target == self.candidate.target,
                "Cross-target switch refused"
            );
        }
        ensure!(
            hex(&self.candidate_definition_sha256, 64)
                && self
                    .prior_definition_sha256
                    .as_ref()
                    .is_none_or(|v| hex(v, 64)),
            "Invalid service definition pin"
        );
        ensure!(
            !self.prior_active || self.prior_definition_sha256.is_some(),
            "Active service lacks exact definition"
        );
        Ok(())
    }
    pub fn transition(&mut self, next: Phase) -> Result<()> {
        self.validate()?;
        ensure!(
            matches!(
                (&self.phase, &next),
                (Phase::Reviewed, Phase::PublishIntent)
                    | (Phase::PublishIntent, Phase::Published)
                    | (Phase::Published, Phase::ActivationIntent)
                    | (Phase::ActivationIntent, Phase::Ready)
                    | (
                        Phase::PublishIntent | Phase::Published | Phase::ActivationIntent,
                        Phase::RecoveryRequired
                    )
                    | (Phase::RecoveryRequired, Phase::Restored)
            ),
            "Invalid or replayed native transition"
        );
        self.phase = next;
        Ok(())
    }
    pub fn automatic_binary_compensation_allowed(&self) -> bool {
        // Activation intent may already have accessed/migrated private state.
        matches!(self.phase, Phase::Reviewed | Phase::Published)
    }
    pub fn verify_recovery(&self, observed: &Self, candidate_state_untouched: bool) -> Result<()> {
        ensure!(
            self == observed && self.phase == Phase::RecoveryRequired,
            "Recovery receipt changed"
        );
        ensure!(
            candidate_state_untouched,
            "Candidate state access unresolved; rollback cannot be inferred safe"
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn receipt() -> Receipt {
        Receipt {
            schema_version: 1,
            operation_id: uuid::Uuid::nil().to_string(),
            owner_identity: "native-owned-attestation".into(),
            previous: None,
            candidate: ReleasePin {
                target: "x86_64-pc-windows-msvc".into(),
                version: "1.1.0-nightly.20261004.1.1".into(),
                manifest_sha256: "a".repeat(64),
                source_sha: "b".repeat(40),
            },
            prior_definition_sha256: None,
            candidate_definition_sha256: "c".repeat(64),
            prior_active: false,
            phase: Phase::Reviewed,
        }
    }
    #[test]
    fn intent_precedes_effect_and_no_replay() {
        let mut r = receipt();
        assert!(r.transition(Phase::Ready).is_err());
        r.transition(Phase::PublishIntent).unwrap();
        assert!(!r.automatic_binary_compensation_allowed());
        assert!(r.transition(Phase::PublishIntent).is_err());
        r.transition(Phase::Published).unwrap();
        r.transition(Phase::ActivationIntent).unwrap();
        assert!(!r.automatic_binary_compensation_allowed());
        r.transition(Phase::RecoveryRequired).unwrap();
        assert!(r.verify_recovery(&r, false).is_err());
        let mut changed = r.clone();
        changed.owner_identity.push('x');
        assert!(r.verify_recovery(&changed, true).is_err());
        r.verify_recovery(&r, true).unwrap();
    }
    #[test]
    fn identity_and_service_pins_are_required() {
        let mut r = receipt();
        r.prior_active = true;
        assert!(r.validate().is_err());
        r.prior_definition_sha256 = Some("d".repeat(64));
        r.validate().unwrap();
        r.candidate.source_sha = "main".into();
        assert!(r.validate().is_err());
    }
}
