//! Non-secret inference observations. Advertised support is not account entitlement
//! and a requested/default value is not evidence of what a provider delivered.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    #[default]
    Unknown,
    Unsupported,
    Advertised,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingResolution {
    pub support: Support,
    /// Encodable suggestions when support is unknown; advertised choices otherwise.
    pub values: Vec<String>,
    pub requested: Option<String>,
    pub default: Option<String>,
    /// Explicit request or advertised model default, never a delivered-value claim.
    pub effective: Option<String>,
    /// Adapter wire value; None means omit. ChatGPT's explicit default sentinel
    /// suppresses catalog selection but is not an API service_tier value.
    pub wire_value: Option<String>,
    /// Last completed provider response in this turn, not a billing guarantee.
    pub provider_reported: Option<String>,
    pub source: String,
    pub observed_at_ms: Option<u64>,
    pub account_support: Support,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceResolution {
    pub thinking: SettingResolution,
    pub service: SettingResolution,
}
