//! Content-free request context observations. Capacity is not cumulative billing.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextScope {
    pub model: String,
    pub transport: String,
    /// SHA-256 of the configured endpoint, never a credential or host path.
    pub endpoint_fingerprint: Option<String>,
    pub account: Option<crate::accounts::AccountBinding>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CountPrecision {
    #[default]
    Unknown,
    ProviderExact,
    ProviderEstimate,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestTokenCount {
    pub scope: ContextScope,
    pub observed_at_ms: u64,
    /// Count of the encoded input, including instructions, schemas and modalities.
    /// None is unknown; zero is a measured zero and never means unavailable.
    pub input_tokens: Option<u64>,
    pub precision: CountPrecision,
    pub method: String,
    pub complete: bool,
    /// Identifies the counted input projection; output reserve is separate.
    pub input_fingerprint: Option<String>,
    pub limitations: Vec<String>,
}

impl RequestTokenCount {
    pub fn reliable_input_tokens(&self) -> Option<u64> {
        if self.complete && self.precision == CountPrecision::ProviderExact {
            self.input_tokens
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelContextCapacity {
    pub scope: ContextScope,
    pub observed_at_ms: u64,
    pub source: String,
    pub default_window_tokens: Option<u64>,
    pub maximum_selectable_window_tokens: Option<u64>,
    /// A maximum advertised in a catalogue does not populate this field.
    pub enabled_window_tokens: Option<u64>,
    pub default_output_tokens: Option<u64>,
    pub maximum_output_tokens: Option<u64>,
}
