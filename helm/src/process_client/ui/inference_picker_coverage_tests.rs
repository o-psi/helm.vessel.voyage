use super::super::coverage_support;
use super::*;

fn picker(target: Target, field: Field) -> Picker {
    let original = Settings {
        model: "saved-model".into(),
        ..Default::default()
    };
    Picker {
        id: Uuid::new_v4(),
        chooser: chooser::Draft::new(&original),
        incarnation: None,
        destination: Destination::Live(target),
        original,
        models: vec![],
        field,
        query: String::new(),
        selected: 8,
        options: vec![],
        loading: false,
        models_loaded: false,
        notice: String::new(),
        confirmation: None,
        command_text: String::new(),
        preserve_draft: true,
    }
}

#[tokio::test]
async fn unavailable_catalog_preserves_custom_model_and_resets_selection() {
    let (_fixture, _app, target) = coverage_support::app();
    let mut p = picker(target, Field::Model);
    p.install_models(None);
    assert_eq!(p.options, vec!["saved-model"]);
    assert_eq!(p.selected, 0);
    assert!(p.notice.contains("Couldn’t load models"));
    p.install_models(Some(vec![]));
    assert!(p.notice.is_empty());
    assert_eq!(p.options, vec!["saved-model"]);
}

#[tokio::test]
async fn override_options_keep_explicit_value_without_claiming_support() {
    let (_fixture, _app, target) = coverage_support::app();
    for field in [Field::Thinking, Field::Service] {
        let mut p = picker(target, field);
        p.original.reasoning_effort = Some("custom-thinking".into());
        p.original.service_tier = Some("custom-service".into());
        p.install_models(None);
        assert_eq!(p.options[0], "inherit");
        assert_eq!(p.options, vec!["inherit"]);
        assert_eq!(
            p.original.reasoning_effort.as_deref(),
            Some("custom-thinking")
        );
        assert_eq!(p.original.service_tier.as_deref(), Some("custom-service"));
    }
}

#[tokio::test]
async fn option_filtering_is_case_insensitive_and_confirmation_ignores_query() {
    let (_fixture, _app, target) = coverage_support::app();
    let mut p = picker(target, Field::Model);
    p.options = vec!["Alpha".into(), "beta".into()];
    p.query = "alp".into();
    assert_eq!(p.options(), vec!["alp", "Alpha"]);
    p.query = "Alpha".into();
    assert_eq!(p.options(), vec!["Alpha"]);
    p.query = "   ".into();
    assert!(p.options().is_empty());
    p.confirmation = Some(p.original.clone());
    assert_eq!(p.options().len(), 3);
    assert_eq!(p.options()[0], "Cancel");
}
