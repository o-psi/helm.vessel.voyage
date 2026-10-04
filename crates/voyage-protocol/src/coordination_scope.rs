//! Review-only launch preparation. Preferences are not execution authority.
use crate::{accounts::AccountBinding, start_settings::StartSettings};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfilePin {
    pub profile_id: Uuid,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedLaunch {
    pub vessel_id: Uuid,
    pub workspace: std::path::PathBuf,
    pub account: AccountBinding,
    pub settings: StartSettings,
    pub profile: Option<ProfilePin>,
    /// This read neither reserves a launch nor grants authority. Creation revalidates.
    pub execution_authorized: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_pin_requires_exact_revision_and_rejects_credentials() {
        let pin = serde_json::json!({"profile_id":Uuid::new_v4(),"revision":3});
        assert!(serde_json::from_value::<ProfilePin>(pin.clone()).is_ok());
        let mut missing = pin.clone();
        missing.as_object_mut().unwrap().remove("revision");
        assert!(serde_json::from_value::<ProfilePin>(missing).is_err());
        let mut forged = pin;
        forged["token"] = serde_json::json!("not-accepted");
        assert!(serde_json::from_value::<ProfilePin>(forged).is_err());
    }
}

/// Public authority pin: never a bearer credential or client-asserted permission.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DestinationPin {
    pub alias: String,
    pub vessel_id: Uuid,
    pub workspace: std::path::PathBuf,
    pub grant: crate::process::GrantBinding,
    pub rights: Vec<crate::process::ProcessRight>,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScopeSelection {
    pub session_id: Uuid,
    pub context: crate::process::GrantBinding,
    pub expected_revision: u64,
    pub destinations: Vec<DestinationPin>,
}

impl ScopeSelection {
    /// Called with server-observed current context and destination pins. Never
    /// union different Helm contexts, even when their principal IDs coincide.
    pub fn validate(&self, current: &Self, now_ms: u64) -> Result<(), &'static str> {
        if self.session_id.is_nil()
            || self.session_id != current.session_id
            || self.context != current.context
            || self.expected_revision != current.expected_revision
        {
            return Err("scope context or revision changed");
        }
        if self.destinations.len() > 32 {
            return Err("scope destination limit");
        }
        let mut aliases = std::collections::BTreeSet::new();
        let mut vessels = std::collections::BTreeSet::new();
        for pin in &self.destinations {
            if pin.alias.is_empty()
                || pin.alias.len() > 64
                || !pin
                    .alias
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || !aliases.insert(&pin.alias)
                || !vessels.insert(pin.vessel_id)
                || pin.vessel_id.is_nil()
                || !pin.workspace.is_absolute()
                || pin.grant.grant_id.is_nil()
                || pin.grant.principal_id.is_nil()
                || pin.expires_at_ms <= now_ms
                || pin.rights.is_empty()
            {
                return Err("invalid destination pin");
            }
            let permitted = current
                .destinations
                .iter()
                .find(|p| {
                    p.vessel_id == pin.vessel_id
                        && p.workspace == pin.workspace
                        && p.grant == pin.grant
                        && p.alias == pin.alias
                })
                .ok_or("destination unavailable to current context")?;
            if pin.expires_at_ms > permitted.expires_at_ms
                || pin.rights.iter().any(|r| !permitted.rights.contains(r))
            {
                return Err("destination scope exceeds current context");
            }
        }
        Ok(())
    }

    /// Existing destination scope may continue while a less privileged Helm
    /// observes. Input/control must have every retained destination permission.
    pub fn authorize_control(&self, current: &Self, now_ms: u64) -> Result<(), &'static str> {
        if self.session_id != current.session_id
            || self.context.principal_id != current.context.principal_id
        {
            return Err("scope session changed");
        }
        let candidate = Self {
            context: current.context.clone(),
            expected_revision: current.expected_revision,
            ..self.clone()
        };
        candidate.validate(current, now_ms)
    }
}

/// Private host-side provisioning reference, excluded from model tool schemas.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteReference {
    pub alias: String,
    pub credential_path: std::path::PathBuf,
}
