//! Convert only portable preferences; never serialize a Config across hosts.
use crate::{Config, config::AccessMode};
use anyhow::{Result, ensure};
use voyage_protocol::start_settings::{StartAccessMode, StartSettings};

pub fn portable(config: &Config, access: AccessMode) -> StartSettings {
    StartSettings {
        model: Some(config.model.clone()),
        reasoning_effort: Some(config.reasoning_effort.clone()),
        service_tier: Some(config.service_tier.clone()),
        temperature: Some(config.temperature),
        max_output_tokens: Some(config.max_tokens),
        context_window: Some(config.context_window),
        access_mode: Some(match access {
            AccessMode::ReadOnly => StartAccessMode::ReadOnly,
            AccessMode::Approval => StartAccessMode::Approval,
            AccessMode::Unrestricted => StartAccessMode::Unrestricted,
        }),
        terminal_max_count: Some(config.terminal_max_count),
        terminal_max_unread_bytes: Some(config.terminal_max_unread_bytes),
        subagent_max_concurrency: Some(config.subagent_max_concurrency),
        command_timeout_secs: Some(config.command_timeout_secs),
        max_output_bytes: Some(config.max_output_bytes),
    }
}

pub fn overlay(base: &mut StartSettings, overrides: &StartSettings) {
    macro_rules! copy { ($($field:ident),*) => {$(if overrides.$field.is_some() { base.$field = overrides.$field.clone(); })*}; }
    copy!(
        model,
        reasoning_effort,
        service_tier,
        temperature,
        max_output_tokens,
        context_window,
        access_mode,
        terminal_max_count,
        terminal_max_unread_bytes,
        subagent_max_concurrency,
        command_timeout_secs,
        max_output_bytes
    );
}

pub fn apply(config: &mut Config, settings: &StartSettings) -> Result<()> {
    if let Some(model) = &settings.model {
        ensure!(
            !model.trim().is_empty() && model.len() <= 1024,
            "invalid model"
        );
        config.model = model.clone();
    }
    macro_rules! optional { ($($field:ident),*) => {$(if let Some(value) = &settings.$field { config.$field = value.clone(); })*}; }
    optional!(reasoning_effort, service_tier, temperature, context_window);
    if let Some(value) = settings.max_output_tokens {
        ensure!(value > 0, "max_output_tokens must be positive");
        config.max_tokens = value;
    }
    macro_rules! positive { ($($field:ident),*) => {$(if let Some(value) = settings.$field { ensure!(value > 0, concat!(stringify!($field), " must be positive")); config.$field = value; })*}; }
    positive!(
        terminal_max_count,
        terminal_max_unread_bytes,
        subagent_max_concurrency,
        command_timeout_secs,
        max_output_bytes
    );
    if let Some(mode) = settings.access_mode {
        config.access = Some(match mode {
            StartAccessMode::ReadOnly => AccessMode::ReadOnly,
            StartAccessMode::Approval => AccessMode::Approval,
            StartAccessMode::Unrestricted => AccessMode::Unrestricted,
        });
        // Mark requested access explicit so host defaults cannot silently replace it.
        config.policy_explicit.access = config.access;
    }
    config.validate()?;
    crate::provider::validate_inference_settings(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_live_access_and_explicit_null_override() {
        let mut config = Config {
            access: Some(AccessMode::Approval),
            reasoning_effort: Some("high".into()),
            ..Config::default()
        };
        let mut inherited = portable(&config, AccessMode::Unrestricted);
        assert_eq!(inherited.access_mode, Some(StartAccessMode::Unrestricted));
        let settings: StartSettings = serde_json::from_value(
            serde_json::json!({"reasoning_effort":null,"max_output_tokens":2048}),
        )
        .unwrap();
        overlay(&mut inherited, &settings);
        assert_eq!(inherited.reasoning_effort, Some(None));
        assert_eq!(inherited.model.as_deref(), Some(config.model.as_str()));
        apply(&mut config, &inherited).unwrap();
        assert_eq!(config.access_mode(), AccessMode::Unrestricted);
        assert_eq!(
            config.policy_explicit.access,
            Some(AccessMode::Unrestricted)
        );
        assert_eq!(config.reasoning_effort, None);
        assert_eq!(config.max_tokens, 2048);
    }
    #[test]
    fn host_base_is_preserved_and_invalid_settings_refuse() {
        let mut config = Config::default();
        let model = config.model.clone();
        apply(&mut config, &StartSettings::default()).unwrap();
        assert_eq!(config.model, model);
        let invalid = StartSettings {
            command_timeout_secs: Some(0),
            ..Default::default()
        };
        assert!(apply(&mut config, &invalid).is_err());
        assert!(
            serde_json::from_value::<StartSettings>(serde_json::json!({"env":{"SECRET":"no"}}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<StartSettings>(serde_json::json!({"allow_write":["/"]}))
                .is_err()
        );
        let portable = serde_json::to_value(portable(&config, AccessMode::Approval)).unwrap();
        for forbidden in [
            "api_key_env",
            "env",
            "vessel_context",
            "provider_authority",
            "allow_write",
            "browser",
        ] {
            assert!(portable.get(forbidden).is_none());
        }
    }
}
