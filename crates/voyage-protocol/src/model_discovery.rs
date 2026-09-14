//! Secret-free catalogue failure vocabulary shared by helper, supervisor and UI.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Configuration,
    Workspace,
    Policy,
    Authentication,
    RateLimit,
    Network,
    InvalidResponse,
    DisplayValidation,
    Timeout,
    Unavailable,
}
impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Workspace => "workspace",
            Self::Policy => "policy",
            Self::Authentication => "authentication",
            Self::RateLimit => "rate_limit",
            Self::Network => "network",
            Self::InvalidResponse => "invalid_response",
            Self::DisplayValidation => "display_validation",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Configuration => {
                "Model discovery configuration could not be resolved on the executing host."
            }
            Self::Workspace => "Model discovery workspace is unavailable or not authorized.",
            Self::Policy => "Executing-host policy prevented model discovery.",
            Self::Authentication => {
                "The provider rejected model-catalogue authentication. Review this account's sign-in."
            }
            Self::RateLimit => "The provider rate-limited model discovery. Retry later.",
            Self::Network => "The executing host could not reach the model catalogue.",
            Self::InvalidResponse => {
                "The provider returned model metadata this client could not parse."
            }
            Self::DisplayValidation => "Model metadata failed safe-display validation.",
            Self::Timeout => "Model discovery timed out. Retry the catalogue read.",
            Self::Unavailable => "Model discovery is unavailable on the executing host.",
        }
    }
    /// Parse only an allowlisted marker, never display surrounding diagnostics.
    pub fn from_diagnostic(text: &str) -> Option<Self> {
        [
            Self::Configuration,
            Self::Workspace,
            Self::Policy,
            Self::Authentication,
            Self::RateLimit,
            Self::Network,
            Self::InvalidResponse,
            Self::DisplayValidation,
            Self::Timeout,
            Self::Unavailable,
        ]
        .into_iter()
        .find(|v| text.contains(&format!("[model_catalog:{}]", v.code())))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub model_catalog_error: Failure,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_allowlisted_diagnostics_cross_boundary() {
        assert_eq!(
            Failure::from_diagnostic("secret [model_catalog:authentication] raw body"),
            Some(Failure::Authentication)
        );
        assert!(Failure::from_diagnostic("[model_catalog:secret]").is_none());
        assert!(
            serde_json::from_value::<ErrorResponse>(
                serde_json::json!({"model_catalog_error":"network","body":"secret"})
            )
            .is_err()
        );
    }
}
