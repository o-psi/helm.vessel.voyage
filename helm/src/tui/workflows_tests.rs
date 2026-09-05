use super::*;
use crate::workflow::{Definition, Scope};

fn definition(scope: Scope) -> Definition {
    Definition {
        scope,
        digest: "a".repeat(64),
        document: crate::workflow::parse(
            br#"
schema_version=1
id="review"
version="1"
description="Review a topic"
prompt="Topic {{topic}}; count {{count}}; flag {{flag}}; optional {{note}}"
[parameters.topic]
type="string"
required=true
max_length=64
[parameters.count]
type="integer"
default=2
minimum=1
maximum=4
[parameters.flag]
type="boolean"
default=false
choices=[false,true]
[parameters.note]
type="string"
"#,
        )
        .unwrap(),
    }
}

#[test]
fn workflow_form_preserves_typed_defaults_and_literal_data() {
    let mut form = Form::new(definition(Scope::User)).unwrap();
    assert!(form.render_preview().is_err());
    form.set_input("topic", "$(touch marker)\nUnicode 🦀")
        .unwrap();
    form.render_preview().unwrap();
    let prepared = form.prepare().unwrap();
    assert_eq!(prepared.invocation.inputs["count"], 2);
    assert_eq!(prepared.invocation.inputs["flag"], false);
    assert!(prepared.invocation.inputs["note"].is_null());
    assert!(prepared.prompt.contains(r#""$(touch marker)\nUnicode 🦀""#));
    assert_eq!(prepared.invocation.digest, "a".repeat(64));
}

#[test]
fn workflow_form_requires_exact_repository_trust_and_rejects_changed_definition() {
    let original = definition(Scope::Repository);
    let mut form = Form::new(original.clone()).unwrap();
    form.set_input("topic", "review").unwrap();
    form.render_preview().unwrap();
    assert!(form.prepare().is_err());
    form.trust_current_digest();
    assert!(form.prepare().is_ok());
    let mut changed = original.clone();
    changed.digest = "b".repeat(64);
    assert!(form.verify_definition(&changed).is_err());
    assert!(form.verify_definition(&original).is_ok());
    assert_eq!(form.prepare().unwrap().invocation.inputs["topic"], "review");
}

#[test]
fn workflow_invalid_values_do_not_discard_other_fields_or_preview_as_valid() {
    let mut form = Form::new(definition(Scope::User)).unwrap();
    form.set_input("topic", "kept").unwrap();
    for (name, value) in [
        ("count", "0"),
        ("count", "5"),
        ("count", "1.5"),
        ("flag", "yes"),
    ] {
        form.set_input(name, value).unwrap();
        assert!(form.render_preview().is_err());
        form.unset_input(name).unwrap();
    }
    form.render_preview().unwrap();
    assert_eq!(form.prepare().unwrap().invocation.inputs["topic"], "kept");
    assert!(form.set_input("topic", &"a".repeat(8193)).is_err());
    assert_eq!(form.prepare().unwrap().invocation.inputs["topic"], "kept");
}

#[test]
fn workflow_secret_declarations_cannot_open_an_input_form() {
    let mut d = definition(Scope::User);
    d.document.parameters.get_mut("topic").unwrap().secret = true;
    assert!(Form::new(d).is_err());
}

#[test]
fn workflow_preparation_never_applies_advisory_authority_metadata() {
    let mut d = definition(Scope::User);
    d.document.recommended.access = Some("unrestricted".into());
    d.document.recommended.provider = Some("codex-compatibility".into());
    d.document.recommended.model = Some("other-model".into());
    let mut form = Form::new(d).unwrap();
    form.set_input("topic", "review").unwrap();
    form.render_preview().unwrap();
    let prepared = form.prepare().unwrap();
    assert_eq!(prepared.invocation.id, "review");
    assert!(!prepared.prompt.contains("unrestricted"));
    assert!(!prepared.no_save);
}

#[test]
fn cancelled_or_replaced_discovery_cannot_open_a_form_or_invoke() {
    let request = Uuid::new_v4();
    let mut panel = Panel {
        mode: Some(Mode::Loading(None)),
        pending: Some(request),
        ..Default::default()
    };
    panel.close();
    panel.discovered(request, Ok(vec![definition(Scope::User)]));
    assert!(!panel.is_open());
    assert!(panel.ready.is_none());
    panel.mode = Some(Mode::Loading(None));
    panel.pending = Some(Uuid::new_v4());
    panel.discovered(request, Ok(vec![definition(Scope::User)]));
    assert!(matches!(panel.mode, Some(Mode::Loading(_))));
    assert!(panel.definitions.is_empty());
}

#[test]
fn workflow_final_recheck_rejects_changed_removed_or_failed_discovery_without_discarding_inputs() {
    for changed in [true, false] {
        let mut form = Form::new(definition(Scope::Repository)).unwrap();
        form.set_input("topic", "kept").unwrap();
        form.render_preview().unwrap();
        form.trust_current_digest();
        let request = Uuid::new_v4();
        let mut panel = Panel {
            mode: Some(Mode::Form(Box::new(form))),
            pending: Some(request),
            ..Default::default()
        };
        let mut newer = definition(Scope::Repository);
        newer.digest = "b".repeat(64);
        panel.discovered(request, Ok(if changed { vec![newer] } else { vec![] }));
        assert!(panel.ready.is_none());
        let Some(Mode::Form(form)) = &panel.mode else {
            panic!("form lost");
        };
        assert!(!form.trusted);
        assert_eq!(form.fields["topic"].as_ref().unwrap().text, "kept");
        assert!(!panel.notice.is_empty());
    }
    let request = Uuid::new_v4();
    let mut panel = Panel {
        mode: Some(Mode::Form(Box::new(
            Form::new(definition(Scope::User)).unwrap(),
        ))),
        pending: Some(request),
        ..Default::default()
    };
    panel.discovered(request, Err("read failed\u{1b}[2J".into()));
    assert!(panel.ready.is_none());
    assert!(!panel.notice.contains('\u{1b}'));
    assert!(matches!(panel.mode, Some(Mode::Form(_))));
}

#[test]
fn explicit_scope_keeps_same_name_definitions_distinct_and_secret_form_is_unavailable() {
    for scope in [Scope::User, Scope::Repository] {
        let request = Uuid::new_v4();
        let mut panel = Panel {
            mode: Some(Mode::Loading(Some(("review".into(), Some(scope))))),
            pending: Some(request),
            ..Default::default()
        };
        panel.discovered(
            request,
            Ok(vec![definition(Scope::User), definition(Scope::Repository)]),
        );
        let Some(Mode::Form(form)) = &panel.mode else {
            panic!("form missing");
        };
        assert_eq!(form.definition.scope, scope);
    }
    let request = Uuid::new_v4();
    let mut d = definition(Scope::User);
    d.document.parameters.get_mut("topic").unwrap().secret = true;
    let mut panel = Panel {
        mode: Some(Mode::Loading(Some(("review".into(), None)))),
        pending: Some(request),
        ..Default::default()
    };
    panel.discovered(request, Ok(vec![d]));
    assert!(matches!(panel.mode, Some(Mode::Picker)));
    assert!(panel.notice.contains("Secret"));
    assert!(panel.ready.is_none());
}

#[test]
fn input_editing_is_unicode_safe_bounded_and_preview_does_not_accept_paste() {
    let mut form = Form::new(definition(Scope::User)).unwrap();
    form.selected = 3; // BTreeMap: count, flag, note, topic.
    form.insert("雪🦀").unwrap();
    form.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();
    form.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
        .unwrap();
    form.insert("界").unwrap();
    assert_eq!(form.fields["topic"].as_ref().unwrap().text, "界🦀");
    form.render_preview().unwrap();
    form.insert("not appended").unwrap();
    assert_eq!(form.prepare().unwrap().invocation.inputs["topic"], "界🦀");
    assert!(matches!(
        form.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap(),
        Action::None
    ));
    form.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(form.fields["topic"].is_none());
    assert!(form.render_preview().is_err());
}

#[test]
fn workflow_render_handles_narrow_unicode_control_and_all_field_selection() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut d = definition(Scope::Repository);
    d.document.description = "Review 雪\u{1b}[2J\u{7} safely".into();
    for size in [(32, 10), (48, 14), (100, 30)] {
        let mut form = Form::new(d.clone()).unwrap();
        form.set_input("topic", "literal 雪\u{1b}[31m").unwrap();
        form.render_preview().unwrap();
        let panel = Panel {
            mode: Some(Mode::Form(Box::new(form))),
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
        terminal
            .draw(|frame| panel.draw(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        for cell in &buffer.content {
            assert!(!cell.symbol().contains('\u{1b}'));
            assert!(!cell.symbol().contains('\u{7}'));
        }
        assert!(buffer.content.iter().any(|cell| cell.symbol() == "W"));
    }
}

#[test]
fn workflow_trust_and_run_require_plain_documented_keys() {
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SHIFT,
    ] {
        let mut form = Form::new(definition(Scope::Repository)).unwrap();
        form.set_input("topic", "review").unwrap();
        form.render_preview().unwrap();
        assert!(matches!(
            form.key(KeyEvent::new(KeyCode::Char('t'), modifiers))
                .unwrap(),
            Action::None
        ));
        assert!(
            form.prepare().is_err(),
            "modifier shortcut must not grant repository trust"
        );
        form.trust_current_digest();
        assert!(
            matches!(
                form.key(KeyEvent::new(KeyCode::Char('r'), modifiers))
                    .unwrap(),
                Action::None
            ),
            "modifier shortcut must not run workflow"
        );
    }
}

#[test]
fn workflow_navigation_preserves_unset_defaults_and_optional_null() {
    for name in ["count", "flag", "note"] {
        for code in [KeyCode::Left, KeyCode::Right, KeyCode::Home, KeyCode::End] {
            let mut form = Form::new(definition(Scope::User)).unwrap();
            form.set_input("topic", "review").unwrap();
            form.selected = form.fields.keys().position(|key| key == name).unwrap();
            form.key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
            assert!(
                form.fields[name].is_none(),
                "navigation changed unset {name}"
            );
            form.render_preview().unwrap();
            let inputs = form.prepare().unwrap().invocation.inputs;
            assert_eq!(inputs["count"], 2);
            assert_eq!(inputs["flag"], false);
            assert!(inputs["note"].is_null());
        }
    }
}

#[test]
fn workflow_selected_field_and_unicode_cursor_remain_visible_at_minimum_size() {
    use ratatui::{Terminal, backend::TestBackend};
    for index in 0..4 {
        let mut form = Form::new(definition(Scope::User)).unwrap();
        form.selected = index;
        let name = form.selected_name().unwrap();
        form.set_input(&name, "雪\n🦀\nvisible-end").unwrap();
        let panel = Panel {
            mode: Some(Mode::Form(Box::new(form))),
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(32, 10)).unwrap();
        terminal
            .draw(|frame| panel.draw(frame, frame.area()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains(&name), "selected field missing: {text}");
        assert!(
            text.contains("visible-end"),
            "editing cursor scrolled out of view: {text}"
        );
    }
}
