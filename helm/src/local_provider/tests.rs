use super::*;
#[test]
fn presets_resolve_to_native_no_auth_configuration_without_changing_policy() {
    for preset in PRESETS {
        let args = EndpointArgs {
            preset,
            endpoint: None,
            model: Some("fixture-model".into()),
            api_key_env: None,
            transport: Transport::Chat,
            chat_max_completion_tokens: false,
            timeout_secs: 1,
        };
        let config = args.resolve().unwrap();
        assert_eq!(config.provider, ProviderKind::OpenaiChat);
        assert_eq!(config.base_url.as_deref(), preset.endpoint());
        assert!(!config.api_key_required);
        assert_eq!(config.api_key().unwrap(), "");
        assert_eq!(config.access_mode(), Config::default().access_mode());
        assert!(config.api_key_for_redaction().is_none_or(|v| v.is_empty()));
    }
}
#[test]
fn no_auth_is_explicit_and_cannot_follow_a_provider_switch() {
    let mut config = Config::default();
    assert!(config.api_key_required);
    config.api_key_required = false;
    assert!(config.validate().is_err());
    config.base_url = Some("http://127.0.0.1:1234/v1".into());
    assert!(config.validate().is_ok());
    config.select_provider(ProviderKind::Anthropic);
    assert!(config.api_key_required);
    config.api_key_required = false;
    assert!(config.validate().is_err());
}
#[test]
fn endpoint_validation_rejects_embedded_secrets_and_nonloopback_cleartext() {
    for endpoint in [
        "http://example.org/v1",
        "http://localhost:1234/v1",
        "https://user:secret@example.org/v1",
        "https://example.org/v1?key=secret",
        "https://example.org/v1#secret",
        "file:///tmp/socket",
        "",
        "https://example.org/\n",
    ] {
        assert!(validate_endpoint(endpoint).is_err(), "{endpoint}");
    }
    for endpoint in [
        "http://127.0.0.1:1234/v1",
        "http://[::1]:1234/v1",
        "https://example.org/v1",
    ] {
        assert!(validate_endpoint(endpoint).is_ok(), "{endpoint}");
    }
}
#[test]
fn save_is_create_only_and_roundtrips_without_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("config.toml");
    save(&Config::default(), &output).unwrap();
    let original = std::fs::read(&output).unwrap();
    assert!(
        save(
            &Config {
                model: "replacement".into(),
                ..Config::default()
            },
            &output
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&output).unwrap(), original);
    assert!(Config::load(Some(&output)).unwrap().api_key_required);
}

#[test]
fn config_overrides_reset_auth_only_when_provider_changes() {
    let original = Config {
        provider: ProviderKind::OpenaiChat,
        base_url: Some("http://127.0.0.1:1234/v1".into()),
        api_key_required: false,
        ..Config::default()
    };
    for provider in ["openai-responses", "anthropic", "chatgpt-oauth"] {
        let mut config = original.clone();
        config.apply_override("provider", provider).unwrap();
        assert!(config.api_key_required);
    }
    let mut config = original;
    config.apply_override("provider", "openai-chat").unwrap();
    config.apply_override("model", "manual").unwrap();
    assert!(!config.api_key_required);
}

#[test]
fn fixed_scan_candidates_and_model_boundaries_are_explicit() {
    let ports = PRESETS.map(|preset| {
        let url = reqwest::Url::parse(preset.endpoint().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.path(), "/v1");
        assert!(url.username().is_empty() && url.password().is_none());
        url.port().unwrap()
    });
    assert_eq!(ports, [11434, 1234, 8000, 8080]);
    assert!(validate_model(&"x".repeat(512)).is_ok());
    assert!(validate_model(&"x".repeat(513)).is_err());
    assert!(validate_model("\u{1b}malicious").is_err());
    assert!(validate_model("  ").is_err());
}
