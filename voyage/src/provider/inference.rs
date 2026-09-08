//! Offline inference override validation. These are transport capabilities, not
//! promises of model support, account entitlement, availability, or billing.
use super::{ModelInfo, ProviderError};
use crate::{
    config::{Config, ProviderKind},
    model::ModelRequest,
};

const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh"];
const TIERS: &[&str] = &["auto", "default", "flex", "scale", "priority"];

fn transport_options(
    provider: &ProviderKind,
) -> (&'static [&'static str], &'static [&'static str]) {
    match provider {
        ProviderKind::OpenaiResponses | ProviderKind::OpenaiChat | ProviderKind::ChatGptOauth => {
            (EFFORTS, TIERS)
        }
        // Anthropic thinking/effort and tier semantics differ. The compatibility
        // bridge does not implement these overrides; never silently discard them.
        ProviderKind::Anthropic | ProviderKind::CodexSubscription => (&[], &[]),
    }
}

/// Options encodable by this adapter. Empty lists mean explicit overrides are
/// unsupported. Omission (None), not a sentinel string, selects provider defaults.
/// Config has no model catalog: use the catalog-aware variant when one is available.
pub fn inference_capabilities(config: &Config) -> (Vec<String>, Vec<String>) {
    inference_capabilities_with_model(config, None)
}

/// Narrow transport options using metadata for the selected model, when supplied.
/// An empty catalog effort list means unknown, not proof of lack of support.
pub fn inference_capabilities_with_model(
    config: &Config,
    model: Option<&ModelInfo>,
) -> (Vec<String>, Vec<String>) {
    let (efforts, tiers) = transport_options(&config.provider);
    let model =
        model.filter(|model| model.id == config.model && !model.reasoning_efforts.is_empty());
    (
        efforts
            .iter()
            .filter(|effort| {
                model.is_none_or(|model| {
                    model
                        .reasoning_efforts
                        .iter()
                        .any(|known| known == **effort)
                })
            })
            .map(|s| (*s).to_owned())
            .collect(),
        tiers.iter().map(|s| (*s).to_owned()).collect(),
    )
}

/// Validate locally, without credentials or network I/O. Success only means the
/// adapter can encode the values; the remote endpoint can still reject them.
pub fn validate_inference_settings(config: &Config) -> anyhow::Result<()> {
    validate_inference_settings_with_model(config, None)
}

/// As above, additionally enforcing advertised efforts for matching model metadata.
pub fn validate_inference_settings_with_model(
    config: &Config,
    model: Option<&ModelInfo>,
) -> anyhow::Result<()> {
    validate_values(
        &config.provider,
        config.reasoning_effort.as_deref(),
        config.service_tier.as_deref(),
    )?;
    validate_model_effort(&config.model, config.reasoning_effort.as_deref(), model)?;
    Ok(())
}

pub(crate) fn validate_model_effort(
    model_id: &str,
    effort: Option<&str>,
    model: Option<&ModelInfo>,
) -> Result<(), ProviderError> {
    if let (Some(effort), Some(model)) = (effort, model)
        && model.id == model_id
        && !model.reasoning_efforts.is_empty()
        && !model.reasoning_efforts.iter().any(|known| known == effort)
    {
        return Err(ProviderError::Request(
            "reasoning_effort is not advertised by the selected model".into(),
        ));
    }
    Ok(())
}

fn validate_values(
    provider: &ProviderKind,
    effort: Option<&str>,
    tier: Option<&str>,
) -> Result<(), ProviderError> {
    let (efforts, tiers) = transport_options(provider);
    for (name, value, supported) in [
        ("reasoning_effort", effort, efforts),
        ("service_tier", tier, tiers),
    ] {
        if let Some(value) = value
            && !supported.contains(&value)
        {
            // Do not echo arbitrary config/request values into diagnostics.
            return Err(ProviderError::Request(format!(
                "unsupported {name} override for {} transport; omit it for provider defaults",
                provider.profile().id
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_request(
    provider: &ProviderKind,
    request: &ModelRequest,
) -> Result<(), ProviderError> {
    validate_values(
        provider,
        request.reasoning_effort.as_deref(),
        request.service_tier.as_deref(),
    )
}
