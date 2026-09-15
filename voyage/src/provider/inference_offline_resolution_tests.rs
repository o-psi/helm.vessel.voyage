use super::*;

#[test]
fn catalog_defaults_intersect_transport_and_oauth_default_is_not_sent() {
    let mut model = ModelInfo::minimal("offline");
    model.reasoning_support_known = true;
    model.reasoning_efforts = vec!["low".into(), "high".into(), "invented".into()];
    model.default_reasoning_effort = Some("low".into());
    model.service_support_known = true;
    model.service_tiers = vec!["default".into(), "priority".into(), "invented".into()];
    model.default_service_tier = Some("default".into());
    for provider in [
        ProviderKind::OpenaiChat,
        ProviderKind::OpenaiResponses,
        ProviderKind::ChatGptOauth,
    ] {
        let resolved = resolve_inference_values(&provider, "offline", None, None, Some(&model));
        assert_eq!(resolved.thinking.support, Support::Advertised);
        assert_eq!(resolved.thinking.values, ["low", "high"]);
        assert_eq!(resolved.thinking.effective.as_deref(), Some("low"));
        assert_eq!(resolved.thinking.source, "authenticated_model_catalog");
        assert_eq!(resolved.service.effective.as_deref(), Some("default"));
        assert_eq!(
            resolved.service.wire_value.as_deref(),
            if provider == ProviderKind::ChatGptOauth {
                None
            } else {
                Some("default")
            }
        );
        validate_resolution(&provider, &resolved).unwrap();
        let requested = resolve_inference_values(
            &provider,
            "offline",
            Some("high"),
            Some("priority"),
            Some(&model),
        );
        assert_eq!(requested.thinking.source, "authenticated_model_catalog");
        assert_eq!(requested.thinking.wire_value.as_deref(), Some("high"));
        assert_eq!(requested.service.wire_value.as_deref(), Some("priority"));
        validate_resolution(&provider, &requested).unwrap();
        let unadvertised =
            resolve_inference_values(&provider, "offline", Some("medium"), None, Some(&model));
        assert!(validate_resolution(&provider, &unadvertised).is_err());
    }
    let anthropic = resolve_inference_values(
        &ProviderKind::Anthropic,
        "offline",
        None,
        None,
        Some(&model),
    );
    assert_eq!(anthropic.thinking.support, Support::Unsupported);
    assert!(anthropic.thinking.values.is_empty());
    assert!(anthropic.service.effective.is_none());
}

#[test]
fn stale_future_and_other_model_catalogs_cannot_override_transport() {
    let mut model = ModelInfo::minimal("offline");
    model.reasoning_support_known = true;
    model.service_support_known = true;
    for (id, observed) in [
        ("other", None),
        ("offline", Some(0)),
        ("offline", Some(u64::MAX)),
    ] {
        model.observed_at_ms = observed;
        let resolution = resolve_inference_values(
            &ProviderKind::OpenaiResponses,
            id,
            Some("high"),
            Some("priority"),
            Some(&model),
        );
        assert_eq!(resolution.thinking.support, Support::Unknown);
        assert_eq!(resolution.service.support, Support::Unknown);
        validate_model_effort(id, Some("high"), Some(&model)).unwrap();
    }
    model.observed_at_ms = Some(super::super::catalog::now_ms());
    let unsupported = resolve_inference_values(
        &ProviderKind::OpenaiResponses,
        "offline",
        None,
        None,
        Some(&model),
    );
    assert_eq!(unsupported.thinking.support, Support::Unsupported);
    assert_eq!(unsupported.service.support, Support::Unsupported);
    assert!(validate_model_effort("offline", Some("high"), Some(&model)).is_err());
    validate_model_effort("offline", None, Some(&model)).unwrap();
}

#[test]
fn malformed_override_values_fail_before_encoding() {
    for provider in [
        ProviderKind::OpenaiChat,
        ProviderKind::OpenaiResponses,
        ProviderKind::ChatGptOauth,
        ProviderKind::Anthropic,
    ] {
        for effort in ["", "HIGH", "unknown", "high\n"] {
            assert!(validate_values(&provider, Some(effort), None).is_err());
        }
        for tier in ["", "inherit", "a b", "a/b", "é", "priority\n"] {
            assert!(validate_values(&provider, None, Some(tier)).is_err());
        }
        assert!(validate_values(&provider, None, Some(&"x".repeat(65))).is_err());
    }
    for tier in [
        "auto",
        "default",
        "flex",
        "scale",
        "priority",
        "custom-tier_1",
    ] {
        assert!(valid_tier(tier));
    }
    for tier in ["auto", "scale"] {
        assert!(validate_values(&ProviderKind::ChatGptOauth, None, Some(tier)).is_ok());
    }
    assert!(validate_values(&ProviderKind::Anthropic, Some("high"), None).is_err());
    assert!(validate_values(&ProviderKind::Anthropic, None, Some("default")).is_err());
}
