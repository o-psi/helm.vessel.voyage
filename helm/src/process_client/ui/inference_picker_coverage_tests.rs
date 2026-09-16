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

fn key(code: KeyCode) -> Event {
    Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
}
fn frame(app: &App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| super::super::render::draw(f, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
fn live_picker(app: &mut App, target: Target, field: Field) -> Picker {
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .inference
        .get_or_insert_with(|| Settings {
            model: "synthetic-model".into(),
            ..Default::default()
        });
    let mut p = picker(target, field);
    p.original = app.inference_settings(Destination::Live(target)).unwrap();
    p.chooser = chooser::Draft::new(&p.original);
    p.incarnation = Some(app.views[&target].process.incarnation);
    p.selected = 0;
    p.options = vec!["inherit".into(), "low".into(), "high".into()];
    p
}

#[tokio::test]
async fn setting_picker_keyboard_filter_boundaries_resize_and_cancel_keep_draft() {
    let (_fixture, mut app, target) = coverage_support::app();
    for field in [Field::Thinking, Field::Service] {
        app.inference.picker = Some(live_picker(&mut app, target, field));
        assert!(!frame(&app, 100, 35).is_empty());
        assert!(app.inference.visible.get());
        app.inference_input(&key(KeyCode::Up)).unwrap();
        assert_eq!(app.inference.picker.as_ref().unwrap().selected, 2);
        app.inference_input(&key(KeyCode::Down)).unwrap();
        assert_eq!(app.inference.picker.as_ref().unwrap().selected, 0);
        app.inference_input(&Event::Paste("HI".into())).unwrap();
        assert_eq!(app.inference.picker.as_ref().unwrap().query, "HI");
        app.inference_input(&key(KeyCode::Backspace)).unwrap();
        assert_eq!(app.inference.picker.as_ref().unwrap().query, "H");
        app.inference_input(&Event::Paste("x".repeat(256))).unwrap();
        assert_eq!(app.inference.picker.as_ref().unwrap().query, "H");
        for modifier in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            app.inference_input(&Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Char('z'),
                modifier,
            )))
            .unwrap();
        }
        assert_eq!(app.inference.picker.as_ref().unwrap().query, "H");
        app.inference_input(&Event::Resize(20, 10)).unwrap();
        app.inference_input(&key(KeyCode::Enter)).unwrap();
        assert!(app.status.contains("not visible"));
        assert!(app.views[&target].pending.is_none());
        app.inference_input(&key(KeyCode::Esc)).unwrap();
        assert!(app.inference.picker.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
}

#[tokio::test]
async fn setting_picker_mouse_selection_is_bounded_and_release_does_not_activate() {
    use crossterm::event::{KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
    let (_fixture, mut app, target) = coverage_support::app();
    app.inference.picker = Some(live_picker(&mut app, target, Field::Thinking));
    frame(&app, 110, 40);
    let area = app.inference.picker_area.get().unwrap();
    let mouse = |kind| {
        Event::Mouse(MouseEvent {
            kind,
            column: area.x + 1,
            row: area.y + 1,
            modifiers: KeyModifiers::NONE,
        })
    };
    for _ in 0..10 {
        app.inference_input(&mouse(MouseEventKind::ScrollDown))
            .unwrap();
    }
    assert_eq!(app.inference.picker.as_ref().unwrap().selected, 2);
    for _ in 0..10 {
        app.inference_input(&mouse(MouseEventKind::ScrollUp))
            .unwrap();
    }
    assert_eq!(app.inference.picker.as_ref().unwrap().selected, 0);
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert!(app.inference_input(&mouse(kind)).unwrap());
    }
    app.inference_input(&Event::Key(KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    )))
    .unwrap();
    assert!(app.views[&target].pending.is_none());
    frame(&app, 110, 40);
    let cancel = app.inference.cancel_hit.get().unwrap();
    app.inference_input(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: cancel.x,
        row: cancel.y,
        modifiers: KeyModifiers::NONE,
    }))
    .unwrap();
    assert!(app.inference.picker.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[tokio::test]
async fn live_inference_apply_pins_latest_revision_and_never_rewrites_composer() {
    for (field, value, expected_field, expected) in [
        (
            Field::Thinking,
            "high",
            "reasoning_effort",
            serde_json::json!("high"),
        ),
        (
            Field::Thinking,
            "default",
            "reasoning_effort",
            serde_json::Value::Null,
        ),
        (
            Field::Service,
            "priority",
            "service_tier",
            serde_json::json!("priority"),
        ),
        (
            Field::Service,
            "inherit",
            "service_tier",
            serde_json::Value::Null,
        ),
    ] {
        let (_fixture, mut app, target) = coverage_support::app();
        let p = live_picker(&mut app, target, field);
        app.views
            .get_mut(&target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .revision = 92;
        app.select_inference(p, value).unwrap();
        let pending = app.views[&target].pending.as_ref().unwrap();
        assert!(pending.preserve_draft);
        let command = serde_json::to_value(pending.original.as_ref().unwrap()).unwrap();
        assert_eq!(command["op"], "set_inference");
        assert_eq!(command["expected_revision"], 92);
        assert_eq!(command[expected_field], expected);
        assert_eq!(command["model"], "synthetic-model");
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        for task in app.route_tasks.values().flatten() {
            task.abort();
        }
    }
}

#[tokio::test]
async fn model_override_confirmation_cancel_reset_and_keep_are_atomic() {
    for choice in [0, 1, 2] {
        let (_fixture, mut app, target) = coverage_support::app();
        app.views
            .get_mut(&target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .inference = Some(Settings {
            model: "synthetic-model".into(),
            reasoning_effort: Some("high".into()),
            ..Default::default()
        });
        let p = live_picker(&mut app, target, Field::Model);
        app.select_inference(p, "another-model").unwrap();
        let p = app.inference.picker.as_ref().unwrap();
        assert!(p.confirmation.is_some());
        assert!(app.views[&target].pending.is_none());
        // Confirmation is shared with the scalar selector; exercise its reset/
        // keep transport path without entering the model chooser's own review.
        app.inference.picker.as_mut().unwrap().field = Field::Thinking;
        app.inference.picker.as_mut().unwrap().selected = choice;
        frame(&app, 110, 40);
        app.inference_input(&key(KeyCode::Enter)).unwrap();
        assert!(app.inference.picker.is_none());
        if choice == 0 {
            assert!(app.views[&target].pending.is_none());
        } else {
            let pending = app.views[&target].pending.as_ref().unwrap();
            let command = serde_json::to_value(pending.original.as_ref().unwrap()).unwrap();
            assert_eq!(command["model"], "another-model");
            assert_eq!(
                command["reasoning_effort"],
                if choice == 1 {
                    serde_json::Value::Null
                } else {
                    serde_json::json!("high")
                }
            );
        }
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        for task in app.route_tasks.values().flatten() {
            task.abort();
        }
    }
}

#[tokio::test]
async fn stale_inference_selection_and_invalid_scalar_values_never_dispatch() {
    let (_fixture, mut app, target) = coverage_support::app();
    for value in ["", "two words", "line\nbreak", "\u{1b}[31m"] {
        let p = live_picker(&mut app, target, Field::Thinking);
        assert!(app.select_inference(p, value).is_err());
    }
    let p = live_picker(&mut app, target, Field::Service);
    assert!(app.select_inference(p, &"x".repeat(257)).is_err());
    let p = live_picker(&mut app, target, Field::Thinking);
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .inference
        .as_mut()
        .unwrap()
        .model = "externally-changed".into();
    assert!(
        app.select_inference(p, "high")
            .unwrap_err()
            .to_string()
            .contains("changed while selecting")
    );
    assert!(app.views[&target].pending.is_none());
    assert!(app.route_tasks.is_empty());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[tokio::test]
async fn inference_catalog_responses_ignore_old_picker_identity_and_accept_envelopes() {
    let (_fixture, mut app, target) = coverage_support::app();
    let p = live_picker(&mut app, target, Field::Thinking);
    let id = p.id;
    app.inference.picker = Some(p);
    app.inference_models(Uuid::new_v4(), None, None, Err("stale result".into()));
    assert!(app.inference.picker.as_ref().unwrap().notice.is_empty());
    app.inference_models(id, None, None, Err("synthetic network diagnostic".into()));
    assert!(
        !app.inference
            .picker
            .as_ref()
            .unwrap()
            .notice
            .contains("synthetic network diagnostic")
    );
    assert!(!app.inference.picker.as_ref().unwrap().loading);
    for payload in [
        serde_json::json!([]),
        serde_json::json!({"value":[]}),
        serde_json::json!({"value":{"inventory":[]}}),
    ] {
        app.inference_models(id, None, None, Ok(payload));
        assert!(app.inference.picker.as_ref().unwrap().models_loaded);
        assert!(
            !app.inference
                .picker
                .as_ref()
                .unwrap()
                .notice
                .contains("Couldn’t load")
        );
        assert_eq!(
            app.inference.picker.as_ref().unwrap().options,
            vec!["inherit"]
        );
    }
}
