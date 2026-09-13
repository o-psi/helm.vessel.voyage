//! Credential-free portable launch settings; host policy remains authoritative.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartAccessMode {
    ReadOnly,
    Approval,
    Unrestricted,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct StartSettings {
    pub model: Option<String>,
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub reasoning_effort: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub service_tier: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub temperature: Option<Option<f32>>,
    pub max_output_tokens: Option<u32>,
    pub context_window: Option<usize>,
    pub access_mode: Option<StartAccessMode>,
    pub terminal_max_count: Option<usize>,
    pub terminal_max_unread_bytes: Option<usize>,
    pub subagent_max_concurrency: Option<usize>,
    pub command_timeout_secs: Option<u64>,
    pub max_output_bytes: Option<usize>,
}
fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nullable_round_trip_preserves_missing_and_reset() {
        for input in [
            serde_json::json!({}),
            serde_json::json!({"reasoning_effort":null}),
            serde_json::json!({"reasoning_effort":"high"}),
        ] {
            let settings: StartSettings = serde_json::from_value(input.clone()).unwrap();
            let output = serde_json::to_value(&settings).unwrap();
            assert_eq!(
                output.get("reasoning_effort"),
                input.get("reasoning_effort")
            );
            assert_eq!(
                serde_json::from_value::<StartSettings>(output).unwrap(),
                settings
            );
        }
    }
}
