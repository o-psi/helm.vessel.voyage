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
