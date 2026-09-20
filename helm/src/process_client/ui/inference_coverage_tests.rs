use super::super::coverage_support;
use super::*;

#[test]
fn slash_parser_accepts_only_exact_supported_commands() {
    for (text, field, value) in [
        ("/model test-model ", Field::Model, "test-model"),
        ("/thinking high", Field::Thinking, "high"),
        ("/service", Field::Service, ""),
        ("/account", Field::Account, ""),
    ] {
        let (actual, argument) = parse(text).unwrap();
        assert!(actual == field);
        assert_eq!(argument, value);
        assert!(field.command().starts_with('/'));
        assert!(!field.name().is_empty());
    }
    for text in ["/models", "hello /model", "", "/modelx"] {
        assert!(parse(text).is_none());
    }
}

#[test]
fn labels_distinguish_explicit_unknown_and_account_defaults() {
    let mut settings = Settings {
        model: "test-model".into(),
        ..Default::default()
    };
    assert_eq!(settings.label(Field::Model), "test-model");
    assert_eq!(settings.label(Field::Account), "Host default (unresolved)");
    assert_eq!(
        settings.label(Field::Thinking),
        "Provider-managed (unknown)"
    );
    settings.reasoning_effort = Some("high".into());
    settings.service_tier = Some("priority".into());
    assert_eq!(settings.label(Field::Thinking), "high (explicit)");
    assert_eq!(settings.label(Field::Service), "priority (explicit)");
    settings.provider = "not-a-provider".into();
    settings.resolve(&[]);
    assert!(settings.resolution.is_none());
}

#[tokio::test]
async fn controls_render_registers_and_clears_click_targets() {
    let (_fixture, mut app, _) = coverage_support::app();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 3)).unwrap();
    terminal
        .draw(|frame| app.draw_inference_controls(frame, frame.area()))
        .unwrap();
    assert!(app.inference.options_hit.get().is_some());
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Profile:"));
    app.clear_inference_hits();
    assert!(app.inference.options_hit.get().is_none());
    assert!(app.inference.choices.borrow().is_empty());
    assert!(!app.inference.visible.get());
    app.selected = None;
    terminal
        .draw(|frame| app.draw_inference_controls(frame, frame.area()))
        .unwrap();
    assert!(app.inference.options_hit.get().is_none());
}
