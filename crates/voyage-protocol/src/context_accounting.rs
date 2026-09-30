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
        if self.complete
            && self.precision == CountPrecision::ProviderExact
            && self
                .input_fingerprint
                .as_ref()
                .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextPressure {
    Unknown,
    WithinBudget,
    PreparationNeeded,
    Irreducible,
}

/// One bounded automatic-preparation sequence, attached to its final observation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextPreparation {
    pub trigger: String,
    pub before_generation: u64,
    pub before_count: RequestTokenCount,
    pub reduced_messages: usize,
    pub steps: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextObservation {
    pub execution_id: uuid::Uuid,
    pub projection_generation: u64,
    pub count: RequestTokenCount,
    pub capacity: Option<ModelContextCapacity>,
    pub operator_limit: Option<u64>,
    pub output_reserve: Option<u64>,
    pub reserve_source: String,
    pub safety_margin: u64,
    pub pressure: ContextPressure,
    /// This observation describes one prepared request, never future tool results.
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<ContextPreparation>,
}

impl ContextObservation {
    /// No private account identity is needed by an observing Helm client.
    pub fn public_view(&self) -> Self {
        let mut value = self.clone();
        value.count.scope.account = None;
        if let Some(preparation) = &mut value.preparation {
            preparation.before_count.scope.account = None;
        }
        if let Some(capacity) = &mut value.capacity {
            capacity.scope.account = None;
        }
        value
    }
    pub fn status_text(&self) -> String {
        let input = self
            .count
            .reliable_input_tokens()
            .map_or_else(|| "unknown".into(), |n| n.to_string());
        let window = self
            .capacity
            .as_ref()
            .and_then(|c| c.enabled_window_tokens)
            .map_or_else(|| "unknown".into(), |n| n.to_string());
        let reserve = self
            .output_reserve
            .map_or_else(|| "unknown".into(), |n| n.to_string());
        format!(
            "Last prepared input: {input} tokens · window: {window} · reserve: {reserve} · projection {}",
            self.projection_generation
        )
    }
}
