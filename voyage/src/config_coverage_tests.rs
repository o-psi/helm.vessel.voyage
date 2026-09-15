use super::*;
#[test]
fn validation_rejects_independent_invalid_limits_without_environment_access() {
    let valid = Config::default();
    valid.validate().unwrap();
    let mutations: Vec<Box<dyn Fn(&mut Config)>> = vec![
        Box::new(|c| c.model = "  ".into()),
        Box::new(|c| c.max_output_bytes = 1023),
        Box::new(|c| c.terminal_max_count = 0),
        Box::new(|c| c.terminal_max_unread_bytes = 1023),
        Box::new(|c| c.subagent_max_concurrency = 0),
        Box::new(|c| c.subagent_event_history = 0),
        Box::new(|c| c.provider_response_timeout_ms = 0),
        Box::new(|c| c.provider_stream_idle_ms = 0),
        Box::new(|c| c.provider_retry_elapsed_ms = 0),
        Box::new(|c| c.provider_retry_attempts = 0),
        Box::new(|c| c.provider_retry_initial_ms = 0),
        Box::new(|c| {
            c.context_window = 100;
            c.max_tokens = 100;
        }),
        Box::new(|c| c.provider_retry_max_ms = c.provider_retry_initial_ms - 1),
    ];
    for mutation in mutations {
        let mut invalid = valid.clone();
        mutation(&mut invalid);
        assert!(invalid.validate().is_err());
    }
    for url in [
        "ftp://example.invalid",
        "https://user:password@example.invalid",
        "https://example.invalid/#fragment",
    ] {
        let mut invalid = valid.clone();
        invalid.mcp_servers.insert(
            "offline".into(),
            McpServerConfig {
                command: String::new(),
                url: Some(url.into()),
                bearer_token_env: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        assert!(invalid.validate().is_err());
    }
    for token in [
        Some("1BAD".into()),
        Some("bad-name".into()),
        Some("".into()),
    ] {
        let mut invalid = valid.clone();
        invalid.mcp_servers.insert(
            "offline".into(),
            McpServerConfig {
                command: String::new(),
                url: Some("https://example.invalid/mcp".into()),
                bearer_token_env: token,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        assert!(invalid.validate().is_err());
    }
    for (command, url, token) in [
        ("", None, None),
        ("cmd", Some("https://example.invalid".into()), None),
        ("cmd", None, Some("TOKEN".into())),
    ] {
        let mut invalid = valid.clone();
        invalid.mcp_servers.insert(
            "offline".into(),
            McpServerConfig {
                command: command.into(),
                url,
                bearer_token_env: token,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        assert!(invalid.validate().is_err());
    }
}
#[test]
fn provider_defaults_legacy_access_and_diagnostic_serialization() {
    let mut config = Config::default();
    for (approval, expected) in [
        (ApprovalMode::Never, AccessMode::Unrestricted),
        (ApprovalMode::Always, AccessMode::Approval),
        (ApprovalMode::OnRisk, AccessMode::Approval),
    ] {
        config.approval = approval;
        config.access = None;
        assert_eq!(config.access_mode(), expected);
        config.access = Some(AccessMode::ReadOnly);
        assert_eq!(config.access_mode(), AccessMode::ReadOnly);
    }
    config.select_provider(ProviderKind::Anthropic);
    assert_eq!(config.api_key_env, "ANTHROPIC_API_KEY");
    assert_eq!(config.model, "claude-sonnet-4-0");
    config.select_provider(ProviderKind::OpenaiChat);
    assert_eq!(config.api_key_env, "OPENAI_API_KEY");
    assert_eq!(config.model, "gpt-5");
    config.api_key_env = "CUSTOM_TEST_NAME".into();
    config.model = "custom-model".into();
    config.select_provider(ProviderKind::Anthropic);
    assert_eq!(config.model, "custom-model");
    assert_eq!(config.api_key_env, "CUSTOM_TEST_NAME");
    config.select_provider(ProviderKind::OpenaiResponses);
    config.base_url = Some("http://127.0.0.1:8080/v1".into());
    config.api_key_required = false;
    assert!(config.provider_profile().credential.contains("no-auth"));
    assert!(config.provider_profile().billing.contains("endpoint"));
    config.command_timeout_secs = 123;
    assert_eq!(config.timeout(), Duration::from_secs(123));
    let diagnostic = config.diagnostic_toml().unwrap();
    assert!(!diagnostic.is_empty());
    let _: toml::Value = toml::from_str(&diagnostic).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    config.workspace = Some(workspace.path().to_owned());
    assert_eq!(
        config.resolve_workspace(None).unwrap(),
        workspace.path().canonicalize().unwrap()
    );
    let explicit = tempfile::tempdir().unwrap();
    assert_eq!(
        config
            .resolve_workspace(Some(explicit.path().to_owned()))
            .unwrap(),
        explicit.path().canonicalize().unwrap()
    );
}
