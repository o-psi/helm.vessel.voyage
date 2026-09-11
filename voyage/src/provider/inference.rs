//! Model/account-scoped inference resolution. Transport encoding is not proof of
//! model support, account entitlement, billing, or the tier ultimately delivered.
use super::{ModelInfo, ProviderError};
use crate::{
    config::{Config, ProviderKind},
    model::ModelRequest,
};
use voyage_protocol::inference::{InferenceResolution, SettingResolution, Support};

const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh"];
const TIERS: &[&str] = &["auto", "default", "flex", "scale", "priority"];

fn transport_options(
    provider: &ProviderKind,
) -> (&'static [&'static str], &'static [&'static str]) {
    match provider {
        ProviderKind::OpenaiResponses | ProviderKind::OpenaiChat => (EFFORTS, TIERS),
        ProviderKind::ChatGptOauth => (EFFORTS, &["default", "flex", "priority"]),
        // These adapters do not encode explicit overrides. Catalog metadata cannot
        // broaden adapter support (Anthropic's thinking semantics are different).
        ProviderKind::Anthropic => (&[], &[]),
    }
}

pub fn resolve_inference(config: &Config, model: Option<&ModelInfo>) -> InferenceResolution {
    resolve_inference_values(
        &config.provider,
        &config.model,
        config.reasoning_effort.as_deref(),
        config.service_tier.as_deref(),
        model,
    )
}

pub fn resolve_inference_values(
    provider: &ProviderKind,
    model_id: &str,
    effort: Option<&str>,
    tier: Option<&str>,
    model: Option<&ModelInfo>,
) -> InferenceResolution {
    fn transport(values: &[&str], requested: Option<&str>) -> SettingResolution {
        SettingResolution {
            support: if values.is_empty() {
                Support::Unsupported
            } else {
                Support::Unknown
            },
            values: values.iter().map(|s| (*s).into()).collect(),
            requested: requested.map(str::to_owned),
            effective: requested.map(str::to_owned),
            wire_value: requested.map(str::to_owned),
            source: "transport".into(),
            ..Default::default()
        }
    }
    let (efforts, tiers) = transport_options(provider);
    let mut thinking = transport(efforts, effort);
    let mut service = transport(tiers, tier);
    // Expired metadata must not reject requests or masquerade as a current default.
    let model = model.filter(|m| {
        m.id == model_id
            && m.observed_at_ms.is_none_or(|at| {
                let now = super::catalog::now_ms();
                at <= now && now.saturating_sub(at) < 300_000
            })
    });
    if !efforts.is_empty()
        && let Some(model) = model
    {
        thinking.observed_at_ms = model.observed_at_ms;
        if model.reasoning_support_known || !model.reasoning_efforts.is_empty() {
            thinking
                .values
                .retain(|v| model.reasoning_efforts.contains(v));
            thinking.support = if thinking.values.is_empty() {
                Support::Unsupported
            } else {
                Support::Advertised
            };
            thinking.source = "authenticated_model_catalog".into();
        }
        thinking.default = model.default_reasoning_effort.clone();
        if thinking.default.is_some() {
            thinking.source = "authenticated_model_catalog".into();
        }
        thinking.effective = thinking
            .requested
            .clone()
            .or_else(|| thinking.default.clone());
    }
    if !tiers.is_empty()
        && let Some(model) = model
    {
        service.observed_at_ms = model.observed_at_ms;
        if model.service_support_known || !model.service_tiers.is_empty() {
            service.values = model
                .service_tiers
                .iter()
                .filter(|v| {
                    tiers.contains(&v.as_str())
                        || (*provider == ProviderKind::ChatGptOauth && valid_tier(v))
                })
                .cloned()
                .collect();
            if *provider == ProviderKind::ChatGptOauth
                && !service.values.iter().any(|v| v == "default")
            {
                service.values.insert(0, "default".into());
            }
            service.support = if service.values.is_empty() {
                Support::Unsupported
            } else {
                Support::Advertised
            };
            service.source = "authenticated_model_catalog".into();
        }
        service.default = model.default_service_tier.clone();
        if service.default.is_some() {
            service.source = "authenticated_model_catalog".into();
        }
        service.effective = service
            .requested
            .clone()
            .or_else(|| service.default.clone());
    }
    thinking.wire_value = thinking.effective.clone();
    service.wire_value = service
        .effective
        .clone()
        .filter(|v| *provider != ProviderKind::ChatGptOauth || v != "default");
    InferenceResolution { thinking, service }
}

pub fn inference_capabilities(config: &Config) -> (Vec<String>, Vec<String>) {
    inference_capabilities_with_model(config, None)
}
pub fn inference_capabilities_with_model(
    config: &Config,
    model: Option<&ModelInfo>,
) -> (Vec<String>, Vec<String>) {
    let resolution = resolve_inference(config, model);
    (resolution.thinking.values, resolution.service.values)
}
pub fn validate_inference_settings(config: &Config) -> anyhow::Result<()> {
    validate_inference_settings_with_model(config, None)
}
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
    let resolution = resolve_inference(config, model);
    validate_resolution(&config.provider, &resolution)?;
    Ok(())
}
pub(crate) fn validate_model_effort(
    model_id: &str,
    effort: Option<&str>,
    model: Option<&ModelInfo>,
) -> Result<(), ProviderError> {
    if let (Some(effort), Some(model)) = (effort, model)
        && model.id == model_id
        && model.observed_at_ms.is_none_or(|at| {
            let now = super::catalog::now_ms();
            at <= now && now.saturating_sub(at) < 300_000
        })
        && (model.reasoning_support_known || !model.reasoning_efforts.is_empty())
        && !model.reasoning_efforts.iter().any(|known| known == effort)
    {
        return Err(ProviderError::Request("reasoning_effort is not advertised by the selected model; clear the override or refresh its catalog".into()));
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
            && !(name == "service_tier"
                && *provider == ProviderKind::ChatGptOauth
                && valid_tier(value))
        {
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

/// Private cache identity, never serialized or logged. Credentials remain on the
/// executing host; changing endpoint, credential, or subscription login invalidates
/// observations even when the provider kind and model name stay the same.
pub async fn inference_context(config: &Config) -> Option<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(
        serde_json::to_vec(&(
            &config.provider,
            &config.base_url,
            &config.chatgpt_base_url,
            &config.api_key_env,
            config.api_key_required,
        ))
        .ok()?,
    );
    if let Some(binding) = &config.account {
        config.validate_account().ok()?;
        let descriptor = crate::accounts::Registry::default_host()
            .ok()?
            .validate_binding(binding)
            .ok()?;
        hash.update(serde_json::to_vec(&(binding, descriptor.capability_revision)).ok()?);
        return Some(hash.finalize().into());
    }
    match config.provider {
        ProviderKind::ChatGptOauth => {
            let path = super::ChatGptTokenStore::default_path().ok()?;
            // Bound local reads too. A missing/invalid login cannot reuse a catalog.
            if tokio::fs::metadata(&path).await.ok()?.len() > 65536 {
                return None;
            }
            let tokens = super::ChatGptTokenStore::new(path).load().await.ok()??;
            // Refresh rotation is not an account switch. No token or account ID
            // leaves this private digest; logout/missing credentials invalidate it.
            hash.update(tokens.account_id.as_bytes());
        }
        _ => hash.update(config.api_key().ok()?.as_bytes()),
    }
    Some(hash.finalize().into())
}

fn valid_tier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && value != "inherit"
}
pub(crate) fn validate_resolution(
    provider: &ProviderKind,
    resolution: &InferenceResolution,
) -> Result<(), ProviderError> {
    validate_values(
        provider,
        resolution.thinking.effective.as_deref(),
        resolution.service.effective.as_deref(),
    )?;
    for (name, setting) in [
        ("reasoning_effort", &resolution.thinking),
        ("service_tier", &resolution.service),
    ] {
        if let Some(value) = &setting.effective
            && setting.support != Support::Unknown
            && !setting.values.contains(value)
        {
            return Err(ProviderError::Request(format!(
                "{name} is not advertised by the selected model; clear the override or refresh its catalog"
            )));
        }
    }
    Ok(())
}
