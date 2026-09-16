//! Synthetic full-App journeys: no host service, provider, or real workspace.
use super::*;
use crossterm::event::{KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use uuid::Uuid;

fn key(app: &mut App, code: KeyCode) {
    app.input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap();
}
fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[tokio::test]
async fn catalogue_search_help_and_settings_never_edit_the_composer() {
    let (_fixture, mut app, target) = coverage_support::app();
    for (width, height) in [(40, 18), (80, 24), (160, 48)] {
        app.input(Event::Resize(width, height)).unwrap();
        key(&mut app, KeyCode::F(8));
        assert!(app.explore.is_some());
        for ch in "settings".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        let screen = draw(&app, width, height);
        assert!(screen.contains("Actions"));
        for code in [
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::PageDown,
            KeyCode::PageUp,
        ] {
            key(&mut app, code);
            draw(&app, width, height);
        }
        key(&mut app, KeyCode::Enter);
        assert!(app.help && app.discovery.settings);
        for code in [
            KeyCode::Down,
            KeyCode::PageDown,
            KeyCode::Up,
            KeyCode::PageUp,
        ] {
            key(&mut app, code);
            assert!(!draw(&app, width, height).is_empty());
        }
        key(&mut app, KeyCode::Esc);
        app.discovery_open("help").unwrap();
        assert!(app.help && !app.discovery.settings);
        draw(&app, width, height);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
    assert!(app.command_checks.is_empty());
}

#[tokio::test]
async fn no_match_catalogue_consumes_paste_release_and_modified_keys() {
    let (_fixture, mut app, target) = coverage_support::app();
    draw(&app, 100, 32);
    app.discovery_open("actions").unwrap();
    for ch in "zzzz-no-such-action".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    assert!(draw(&app, 100, 32).contains("No matching actions"));
    app.input(Event::Paste("not composer text".into())).unwrap();
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        app.input(Event::Key(KeyEvent::new(KeyCode::Char('x'), modifiers)))
            .unwrap();
    }
    let mut release = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    app.input(Event::Key(release)).unwrap();
    assert!(app.explore.is_some());
    key(&mut app, KeyCode::Enter);
    assert!(app.explore.is_some());
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[tokio::test]
async fn discovery_reasons_cover_absent_stale_and_recovering_selection() {
    let (_fixture, mut app, target) = coverage_support::app();
    for name in ["stop", "cancel", "receipt", "approve", "deny", "answer"] {
        assert!(app.discovery_reason(name).is_some(), "{name}");
        app.discovery_open(name).unwrap();
        assert!(!app.status.is_empty());
    }
    app.selected = None;
    for (name, _, _) in discovery::COMMANDS {
        // The catalogue is descriptive, not an authority to execute anything.
        let reason = app.discovery_reason(name);
        if !matches!(
            *name,
            "help" | "actions" | "settings" | "new" | "vessels" | "voyages" | "archived"
        ) {
            assert!(reason.is_some(), "{name}");
        }
        assert!(!discovery::scope(name).is_empty());
        assert!(!discovery::shortcut(name).is_empty());
    }
    app.selected = Some(target);
    let view = app.views.remove(&target).unwrap();
    assert!(
        app.discovery_reason("inspect")
            .unwrap()
            .contains("unavailable")
    );
    app.views.insert(target, view);
    app.views.get_mut(&target).unwrap().snapshot = None;
    assert!(
        app.discovery_reason("inspect")
            .unwrap()
            .contains("snapshot")
    );
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[tokio::test]
async fn voyage_picker_filters_without_leaking_input_to_live_draft() {
    let (_fixture, mut app, target) = coverage_support::app();
    draw(&app, 100, 32);
    app.open_voyage_picker();
    assert!(app.voyage_picker.is_some());
    app.input(Event::Paste("Synthetic".into())).unwrap();
    for code in [
        KeyCode::End,
        KeyCode::Home,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Backspace,
    ] {
        key(&mut app, code);
        draw(&app, 100, 32);
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.voyage_picker.is_none());
    assert_eq!(app.selected, Some(target));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    app.open_voyage_picker();
    app.input(Event::Paste("unknown voyage".into())).unwrap();
    key(&mut app, KeyCode::Enter);
    assert!(app.voyage_picker.is_some());
    key(&mut app, KeyCode::Esc);
    assert!(app.voyage_picker.is_none());
}

#[tokio::test]
async fn conversation_render_state_matrix_and_overview_navigation() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.views.get_mut(&target).unwrap().snapshot.as_mut().unwrap().messages = serde_json::from_value(json!([
        {"message_index":0,"role":"system","content":"Synthetic policy"},
        {"message_index":1,"role":"user","content":"Question with `code` and 日本語"},
        {"message_index":2,"role":"assistant","content":"# Answer\n\n- first\n- second\n\n```rust\nfn main() {}\n```"},
        {"message_index":3,"role":"tool","content":"exit: 0\nstdout:\ncompleted\nstderr:\n"}
    ])).unwrap();
    for state in ["live", "unavailable", "cleanup_unconfirmed"] {
        app.views.get_mut(&target).unwrap().process.state =
            serde_json::from_value(json!(state)).unwrap();
        for error in [None, Some("synthetic disconnection".to_owned())] {
            app.views.get_mut(&target).unwrap().error = error;
            for unavailable in [false, true] {
                app.views.get_mut(&target).unwrap().connection_unavailable = unavailable;
                for (w, h) in [(1, 1), (39, 17), (40, 18), (80, 24), (150, 45)] {
                    assert!(!draw(&app, w, h).is_empty());
                }
            }
        }
    }
    app.views.get_mut(&target).unwrap().panel = Some("Synthetic overview\n".repeat(100));
    draw(&app, 100, 30);
    for code in [
        KeyCode::Down,
        KeyCode::PageDown,
        KeyCode::PageUp,
        KeyCode::Up,
    ] {
        key(&mut app, code);
        draw(&app, 100, 30);
    }
    key(&mut app, KeyCode::Esc);
    assert!(app.views[&target].panel.is_none());
    assert_eq!(app.views[&target].scroll, 0);
}

#[tokio::test]
async fn stop_review_is_modal_and_revalidates_revision_before_dispatch() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert!(app.review_stop(target).is_err());
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .run =
        Some(serde_json::from_value(json!({"run_id":Uuid::new_v4(),"state":"running"})).unwrap());
    app.review_stop(target).unwrap();
    draw(&app, 100, 32);
    app.input(Event::Paste("must not leak".into())).unwrap();
    for code in [
        KeyCode::PageDown,
        KeyCode::PageUp,
        KeyCode::Down,
        KeyCode::Up,
    ] {
        key(&mut app, code);
    }
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .revision += 1;
    key(&mut app, KeyCode::Enter);
    assert!(app.stop_review.is_some());
    assert!(app.views[&target].pending.is_none());
    assert!(draw(&app, 100, 32).contains("Voyage changed"));
    key(&mut app, KeyCode::Esc);
    assert!(app.stop_review.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[tokio::test]
async fn input_mouse_focus_release_and_composer_limit_are_local() {
    let (_fixture, mut app, target) = coverage_support::app();
    draw(&app, 100, 32);
    app.input(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 50,
        row: 12,
        modifiers: KeyModifiers::NONE,
    }))
    .unwrap();
    assert!(app.sidebar.pointer.is_some());
    app.input(Event::FocusLost).unwrap();
    assert!(app.sidebar.pointer.is_none());
    app.input(Event::FocusGained).unwrap();
    let mut release = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    app.input(Event::Key(release)).unwrap();
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    key(&mut app, KeyCode::Char('界'));
    assert!(app.views[&target].draft.text.ends_with('界'));
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    app.views.get_mut(&target).unwrap().draft = Default::default();
    app.views
        .get_mut(&target)
        .unwrap()
        .draft
        .insert_str(&"x".repeat(65536));
    assert!(
        app.input(Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE
        )))
        .is_err()
    );
    assert!(
        app.input(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )))
        .is_err()
    );
    assert_eq!(app.views[&target].draft.text.len(), 65536);
    assert!(app.views[&target].pending.is_none());
}

fn observation(app: &App, target: state::Target) -> operator::Observation {
    operator::Observation {
        target,
        incarnation: app.views[&target].process.incarnation,
        revision: 17,
        run_id: None,
    }
}
fn panel_draw(mut render: impl FnMut(&mut ratatui::Frame<'_>, ratatui::layout::Rect)) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            render(frame, area);
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
fn event(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn inspection_scope_requests_and_changed_file_navigation_stay_remote() {
    let (_fixture, app, target) = coverage_support::app();
    let observation = observation(&app, target);
    let tools = json!({"value":{"inventory":[
        {"name":"shell","input_schema":{}},
        {"name":"list_directory","input_schema":{}},
        {"name":"read_file","input_schema":{}}
    ]}});
    for scope in [
        inspection::Scope::Status,
        inspection::Scope::Unstaged,
        inspection::Scope::Staged,
        inspection::Scope::Untracked,
        inspection::Scope::Directory,
        inspection::Scope::File,
    ] {
        let request = scope
            .request(observation, &tools, "folder/a'b.txt")
            .unwrap();
        assert_eq!(request.observation, observation);
        assert!(!scope.label().is_empty());
        assert!(request.command_text().unwrap().starts_with("/tool "));
        for path in [
            "../escape",
            "/absolute",
            "C:drive",
            "bad\\path",
            "bad\npath",
        ] {
            assert!(scope.request(observation, &tools, path).is_err());
        }
        assert!(scope.request(observation, &json!([]), ".").is_err());
    }
    let mut panel = inspection::Panel::new(observation, "Synthetic executing host".into(), tools);
    assert!(matches!(
        panel.input(&event(KeyCode::Enter)),
        inspection::Outcome::Submit(_)
    ));
    panel.result("exit: 0\nstdout:\n M src/main.rs\n?? new.txt\n M \"space file.rs\"\nmalformed\n\nstderr:\n");
    assert!(panel_draw(|f, a| panel.render(f, a)).contains("src/main.rs"));
    for code in [KeyCode::Tab, KeyCode::Down, KeyCode::Up] {
        panel.input(&event(code));
    }
    match panel.input(&event(KeyCode::Enter)) {
        inspection::Outcome::Submit(request) => {
            assert_eq!(request.name, "read_file");
            assert_eq!(request.arguments["path"], "src/main.rs");
        }
        _ => panic!("selected changed file must create a remote read request"),
    }
    panel.result(&"result line\n".repeat(100));
    for code in [
        KeyCode::PageDown,
        KeyCode::PageUp,
        KeyCode::Char('c'),
        KeyCode::Char('a'),
        KeyCode::Char('p'),
        KeyCode::Backspace,
        KeyCode::Char('x'),
        KeyCode::Enter,
    ] {
        panel.input(&event(code));
        panel_draw(|f, a| panel.render(f, a));
    }
    panel.set_error("synthetic refused request".into());
    assert!(panel_draw(|f, a| panel.render(f, a)).contains("synthetic refused request"));
    panel.input(&Event::Resize(40, 18));
    assert!(matches!(
        panel.input(&event(KeyCode::Esc)),
        inspection::Outcome::Close
    ));
}

#[test]
fn operator_forms_review_typed_scalars_and_pin_observation_without_dispatch() {
    let (_fixture, app, target) = coverage_support::app();
    for (schema, input, expected) in [
        (
            json!({"type":"string","minLength":1}),
            "hello",
            json!("hello"),
        ),
        (
            json!({"type":"integer","minimum":1,"maximum":10}),
            "7",
            json!(7),
        ),
        (json!({"type":"number"}), "2.5", json!(2.5)),
        (
            json!({"type":"array","items":{"type":"string"}}),
            "first\nsecond",
            json!(["first", "second"]),
        ),
    ] {
        let captured = observation(&app, target);
        let tools = json!([{"name":"synthetic_tool","input_schema":{"type":"object","properties":{"value":schema},"required":["value"]}}]);
        let mut panel =
            operator::Panel::new(captured, &tools, &json!([]), &json!([]), None).unwrap();
        assert!(panel_draw(|f, a| panel.render(f, a)).contains("synthetic_tool"));
        panel.input(&event(KeyCode::Enter));
        panel.input(&Event::Paste(input.into()));
        panel.input(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )));
        // A review must be painted before confirmation can produce a request.
        assert!(matches!(
            panel.input(&event(KeyCode::Enter)),
            operator::Outcome::Stay
        ));
        panel_draw(|f, a| panel.render(f, a));
        match panel.input(&event(KeyCode::Enter)) {
            operator::Outcome::Submit(request) => {
                assert_eq!(request.observation, captured);
                assert_eq!(request.arguments["value"], expected);
                assert_eq!(request.name, "synthetic_tool");
            }
            _ => panic!("valid reviewed scalar did not produce a request: {input}"),
        }
    }
}

#[test]
fn operator_validation_search_cancel_resize_and_error_journey() {
    let (_fixture, app, target) = coverage_support::app();
    let tools = json!([{"name":"synthetic_tool","input_schema":{"type":"object","properties":{"number":{"type":"integer","minimum":1,"maximum":3}},"required":["number"]}}]);
    let mut panel = operator::Panel::new(
        observation(&app, target),
        &tools,
        &json!([]),
        &json!([]),
        None,
    )
    .unwrap();
    panel.input(&Event::Paste("no match".into()));
    panel_draw(|f, a| panel.render(f, a));
    for _ in 0..8 {
        panel.input(&event(KeyCode::Backspace));
    }
    panel.input(&event(KeyCode::Enter));
    for value in ["not a number", "0", "4"] {
        panel.input(&event(KeyCode::Delete));
        panel.input(&Event::Paste(value.into()));
        assert!(matches!(
            panel.input(&Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::CONTROL
            ))),
            operator::Outcome::Stay
        ));
        panel_draw(|f, a| panel.render(f, a));
    }
    panel.set_error("synthetic transport refusal".into());
    assert!(panel_draw(|f, a| panel.render(f, a)).contains("synthetic transport refusal"));
    panel.input(&Event::Resize(80, 24));
    for code in [
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::PageDown,
        KeyCode::PageUp,
        KeyCode::Esc,
    ] {
        panel.input(&event(code));
    }
    assert!(matches!(
        panel.input(&event(KeyCode::Esc)),
        operator::Outcome::Close
    ));
}

#[tokio::test]
async fn command_validation_retains_unsent_text_without_admission() {
    let (_fixture, mut app, target) = coverage_support::app();
    for command in [
        "",
        "   ",
        "/approve not-a-uuid",
        "/deny not-a-uuid",
        "/answer not-a-uuid response",
        "/clear wrong-id",
        "/delete wrong-id",
        "/definitely-unknown-command",
    ] {
        assert!(
            app.command_for(target, command.into(), false).is_err(),
            "{command}"
        );
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
    assert!(app.command_for(target, "x".repeat(65537), false).is_err());
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .recovery_pending = true;
    assert!(
        app.command_for(target, "normal prompt".into(), false)
            .unwrap_err()
            .to_string()
            .contains("recovery")
    );
    app.views.get_mut(&target).unwrap().snapshot = None;
    assert!(
        app.command_for(target, "normal prompt".into(), false)
            .is_err()
    );
    app.selected = None;
    assert!(app.send().is_err());
    assert!(app.command_checks.is_empty());
}

#[tokio::test]
async fn submissions_pin_revision_identity_and_retain_original_envelopes() {
    for (state, expected) in [(None, "submit"), (Some("running"), "steer")] {
        let (_fixture, mut app, target) = coverage_support::app();
        let run_id = Uuid::new_v4();
        if let Some(state) = state {
            app.views
                .get_mut(&target)
                .unwrap()
                .snapshot
                .as_mut()
                .unwrap()
                .run =
                Some(serde_json::from_value(json!({"run_id":run_id,"state":state})).unwrap());
        }
        let prompt = "  synthetic prompt\nwith Unicode 界  ";
        app.command_for(target, prompt.into(), false).unwrap();
        let pending = app.views[&target].pending.as_ref().unwrap();
        assert_eq!(pending.draft, prompt);
        assert!(!pending.preserve_draft);
        assert!(!pending.receipt_only);
        assert_eq!(pending.incarnation, app.views[&target].process.incarnation);
        let original = serde_json::to_value(pending.original.as_ref().unwrap()).unwrap();
        assert_eq!(original["op"], expected);
        assert_eq!(original["expected_revision"], 17);
        assert_eq!(original["command_id"], pending.command_id.to_string());
        assert_eq!(original["prompt"], prompt);
        if expected == "steer" {
            assert_eq!(original["run_id"], run_id.to_string());
            assert!(app.status.contains("Steering pending"));
        } else {
            assert!(app.status.contains("Submission pending"));
        }
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        for size in [(60, 20), (140, 45)] {
            assert!(!draw(&app, size.0, size.1).is_empty());
        }
        for task in app.route_tasks.values().flatten() {
            task.abort();
        }
    }
}

#[tokio::test]
async fn administrative_commands_capture_exact_confirmations_and_tool_run_scope() {
    for running in [false, true] {
        for operation in ["clear", "delete", "tool", "rename"] {
            let (_fixture, mut app, target) = coverage_support::app();
            let run_id = Uuid::new_v4();
            if running {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .run = Some(
                    serde_json::from_value(json!({"run_id":run_id,"state":"running"})).unwrap(),
                );
            }
            let text = match operation {
                "clear" | "delete" => format!("/{operation} {}", target.session),
                "tool" => "/tool synthetic_tool {\"count\":3,\"text\":\"界\"}".into(),
                _ => "/rename Synthetic renamed voyage".into(),
            };
            app.command_for(target, text.clone(), true).unwrap();
            let pending = app.views[&target].pending.as_ref().unwrap();
            assert_eq!(pending.draft, text);
            assert!(pending.preserve_draft);
            let envelope = serde_json::to_value(pending.original.as_ref().unwrap()).unwrap();
            assert_eq!(envelope["expected_revision"], 17);
            match operation {
                "clear" | "delete" => {
                    assert_eq!(envelope["confirm_session_id"], target.session.to_string())
                }
                "tool" => {
                    assert_eq!(envelope["name"], "synthetic_tool");
                    assert_eq!(envelope["arguments"], json!({"count":3,"text":"界"}));
                    assert_eq!(
                        envelope["op"],
                        if running {
                            "execute_tool"
                        } else {
                            "operator_tool"
                        }
                    );
                    if running {
                        assert_eq!(envelope["run_id"], run_id.to_string());
                    }
                }
                _ => assert_eq!(envelope["name"], "Synthetic renamed voyage"),
            }
            for task in app.route_tasks.values().flatten() {
                task.abort();
            }
        }
    }
}

#[tokio::test]
async fn decision_commands_validate_kind_incarnation_and_answer_before_dispatch() {
    for (operation, kind, response) in [
        ("approve", "approval", json!("approved")),
        ("deny", "approval", json!("denied")),
        (
            "answer",
            "question",
            json!({"status":"custom","answer":"a synthetic answer"}),
        ),
    ] {
        let (_fixture, mut app, target) = coverage_support::app();
        let decision_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let incarnation = app.views[&target].process.incarnation;
        let decision = json!({"decision_id":decision_id,"run_id":run_id,
            "incarnation":incarnation,"expires_at_ms":u64::MAX,"request":{"kind":kind}});
        app.views
            .get_mut(&target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .decisions
            .push(serde_json::from_value(decision).unwrap());
        let suffix = if operation == "answer" {
            " a synthetic answer"
        } else {
            ""
        };
        app.command_for(target, format!("/{operation} {decision_id}{suffix}"), true)
            .unwrap();
        let pending = app.views[&target].pending.as_ref().unwrap();
        let command = serde_json::to_value(pending.original.as_ref().unwrap()).unwrap();
        assert_eq!(command["op"], "respond");
        assert_eq!(command["decision_id"], decision_id.to_string());
        assert_eq!(command["run_id"], run_id.to_string());
        assert_eq!(command["response"], response);
        for task in app.route_tasks.values().flatten() {
            task.abort();
        }
    }
    for (operation, kind, stale) in [
        ("approve", "question", false),
        ("deny", "question", false),
        ("answer", "approval", false),
        ("answer", "question", false),
        ("approve", "approval", true),
    ] {
        let (_fixture, mut app, target) = coverage_support::app();
        let id = Uuid::new_v4();
        let incarnation = if stale {
            Uuid::new_v4()
        } else {
            app.views[&target].process.incarnation
        };
        app.views
            .get_mut(&target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .decisions
            .push(
                serde_json::from_value(json!({"decision_id":id,"run_id":Uuid::new_v4(),
                "incarnation":incarnation,"expires_at_ms":u64::MAX,"request":{"kind":kind}}))
                .unwrap(),
            );
        assert!(
            app.command_for(target, format!("/{operation} {id}"), false)
                .is_err()
        );
        assert!(app.views[&target].pending.is_none());
        assert!(app.route_tasks.is_empty());
    }
}

#[tokio::test]
async fn tool_validation_and_cleanup_gates_never_create_pending_commands() {
    let (_fixture, mut app, target) = coverage_support::app();
    for command in [
        "/tool name",
        "/tool name {broken",
        "/clear",
        "/delete",
        "/stop",
        "/cancel",
        "/clear 00000000-0000-0000-0000-000000000000",
        "/delete 00000000-0000-0000-0000-000000000000",
    ] {
        assert!(
            app.command_for(target, command.into(), false).is_err(),
            "{command}"
        );
        assert!(app.views[&target].pending.is_none());
    }
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .run = Some(
        serde_json::from_value(json!({"run_id":Uuid::new_v4(),"state":"cancel_requested"}))
            .unwrap(),
    );
    let error = app
        .command_for(target, "next prompt".into(), false)
        .unwrap_err();
    assert!(error.to_string().contains("Stop is pending"));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[tokio::test]
async fn explore_keyboard_boundaries_empty_results_and_unicode_queries() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert!(
        !app.explore_input(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap()
    );
    app.explore_input(&KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE))
        .unwrap();
    assert!(app.explore.is_some());
    for _ in 0..200 {
        app.explore_input(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(app.explore, Some(discovery::matches("").len() - 1));
    for _ in 0..200 {
        app.explore_input(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(app.explore, Some(0));
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        app.explore_input(&KeyEvent::new(KeyCode::Char('x'), modifiers))
            .unwrap();
    }
    assert!(app.discovery.query.is_empty());
    for c in "zzzz界".chars() {
        app.explore_input(&KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .unwrap();
    }
    assert!(discovery::matches(&app.discovery.query).is_empty());
    app.explore_input(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.explore.is_some());
    assert!(draw(&app, 120, 40).contains("No matching actions"));
    app.explore_input(&KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.discovery.detail_scroll, 1);
    app.explore_input(&KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.discovery.detail_scroll, 0);
    app.explore_input(&KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.discovery.query, "zzzz");
    app.discovery.query = "x".repeat(256);
    app.explore_input(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.discovery.query.len(), 256);
    app.explore_input(&KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE))
        .unwrap();
    assert!(app.explore.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[tokio::test]
async fn help_overlay_consumes_private_input_but_allows_resize_and_explicit_quit() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert!(!app.discovery_input(&Event::Paste("not consumed".into())));
    assert!(app.discovery_input(&event(KeyCode::F(1))));
    assert!(app.help);
    assert!(app.discovery_input(&Event::Paste("must never enter composer".into())));
    assert!(!app.discovery_input(&Event::Resize(100, 30)));
    for code in [KeyCode::Down, KeyCode::PageDown] {
        assert!(app.discovery_input(&event(code)));
    }
    assert_eq!(app.help_scroll, 2);
    for code in [KeyCode::Up, KeyCode::PageUp, KeyCode::PageUp] {
        app.discovery_input(&event(code));
    }
    assert_eq!(app.help_scroll, 0);
    assert!(app.discovery_input(&Event::Key(KeyEvent::new_with_kind(
        KeyCode::Esc,
        KeyModifiers::NONE,
        KeyEventKind::Release
    ))));
    assert!(app.help);
    assert!(!draw(&app, 100, 40).is_empty());
    app.discovery_input(&Event::Key(KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::CONTROL,
    )));
    assert!(app.quit);
    app.discovery_input(&event(KeyCode::Esc));
    assert!(!app.help);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[test]
fn control_mutations_preserve_revisions_expiry_and_reject_relative_configuration() {
    let (_fixture, app, target) = coverage_support::app();
    let view = &app.views[&target];
    let id = Uuid::new_v4();
    for (text, operation, field, expected) in [
        ("/compact 0", "compact", "retain", json!(0)),
        ("/compact 4294967295", "compact", "retain", json!(u32::MAX)),
        (
            "/configure /synthetic/config with spaces.toml",
            "configure",
            "config_path",
            json!("/synthetic/config with spaces.toml"),
        ),
        ("/archive", "archive", "archived", json!(true)),
        ("/restore", "archive", "archived", json!(false)),
    ] {
        let command = controls::mutation(text, view, id, 91, 12345)
            .unwrap()
            .unwrap();
        let command = serde_json::to_value(command).unwrap();
        assert_eq!(command["op"], operation);
        assert_eq!(command[field], expected);
        assert_eq!(command["command_id"], id.to_string());
        assert_eq!(command["expected_revision"], 91);
        assert_eq!(command["expires_at_ms"], 12345);
        if operation == "compact" {
            assert_eq!(command["preserve_canonical"], true);
        }
    }
    for text in [
        "/compact -1",
        "/compact 4294967296",
        "/compact many",
        "/configure relative.toml",
        "/configure ",
        "/tool missing",
        "/tool x nope",
    ] {
        assert!(controls::mutation(text, view, id, 1, 2).is_err(), "{text}");
    }
    for text in [
        "hello",
        "/unknown",
        "/compact",
        "/configure",
        "/archive extra",
    ] {
        assert!(controls::mutation(text, view, id, 1, 2).unwrap().is_none());
    }
}

#[tokio::test]
async fn workflow_inventory_overlay_quarantines_events_and_closes_explicitly() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert!(!app.workflow_input(&event(KeyCode::Enter)).unwrap());
    app.open_workflows().unwrap();
    assert!(app.workflows_open());
    for event in [
        Event::Paste("private input must not leak".into()),
        Event::Resize(90, 30),
        event(KeyCode::Enter),
        event(KeyCode::Tab),
        event(KeyCode::PageDown),
        Event::FocusGained,
    ] {
        assert!(app.workflow_input(&event).unwrap());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
    for (w, h) in [(40, 16), (100, 35), (160, 50)] {
        let output = draw(&app, w, h);
        assert!(!output.contains("private input must not leak"));
        assert!(!output.is_empty());
    }
    app.workflow_input(&Event::FocusLost).unwrap();
    assert!(app.workflows_open());
    app.workflow_input(&Event::Paste("quarantined after blur".into()))
        .unwrap();
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    app.workflow_input(&event(KeyCode::Esc)).unwrap();
    assert!(!app.workflows_open());
    assert!(app.status.contains("private inputs discarded"));
    assert!(app.views[&target].pending.is_none());
    for task in app.route_tasks.values().flatten() {
        task.abort();
    }
}

#[tokio::test]
async fn workflow_poll_invalidation_tracks_selected_identity_and_snapshot_requirements() {
    for change in ["selection", "incarnation", "missing"] {
        let (_fixture, mut app, target) = coverage_support::app();
        app.open_workflows().unwrap();
        match change {
            "selection" => app.selected = None,
            "incarnation" => {
                app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4()
            }
            _ => {
                app.views.remove(&target);
            }
        }
        app.poll_workflows();
        assert!(app.workflows_open());
        assert!(app.status.contains("invalidated"));
        let status = app.status.clone();
        app.poll_workflows();
        assert_eq!(app.status, status);
        assert!(app.workflow_input(&event(KeyCode::Enter)).unwrap());
        assert!(app.workflows_open());
        app.workflow_input(&event(KeyCode::Esc)).unwrap();
        assert!(!app.workflows_open());
        for task in app.route_tasks.values().flatten() {
            task.abort();
        }
    }
    let (_fixture, mut app, target) = coverage_support::app();
    app.selected = None;
    assert!(
        app.open_workflows()
            .unwrap_err()
            .to_string()
            .contains("Select")
    );
    app.selected = Some(target);
    app.views.get_mut(&target).unwrap().snapshot = None;
    assert!(
        app.open_workflows()
            .unwrap_err()
            .to_string()
            .contains("snapshot")
    );
    assert!(!app.workflows_open());
    assert!(app.route_tasks.is_empty());
}

#[tokio::test]
async fn workflow_quit_shortcuts_clear_panel_without_submitting() {
    for letter in ['c', 'q'] {
        let (_fixture, mut app, target) = coverage_support::app();
        app.open_workflows().unwrap();
        app.workflow_input(&Event::Key(KeyEvent::new(
            KeyCode::Char(letter),
            KeyModifiers::CONTROL,
        )))
        .unwrap();
        assert!(app.quit);
        assert!(!app.workflows_open());
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        for task in app.route_tasks.values().flatten() {
            task.abort();
        }
    }
}

#[test]
fn operator_optional_enum_boolean_and_array_forms_submit_typed_values() {
    let (_fixture, app, target) = coverage_support::app();
    for (schema, keys, expected) in [
        (json!({"type":"boolean"}), vec![KeyCode::Enter], json!(true)),
        (
            json!({"type":"string","enum":["alpha","beta"]}),
            vec![KeyCode::Right, KeyCode::Enter],
            json!("beta"),
        ),
        (
            json!({"type":"array","items":{"type":"string","enum":["alpha","beta"]}}),
            vec![KeyCode::Enter, KeyCode::Right, KeyCode::Enter],
            json!(["alpha", "beta"]),
        ),
    ] {
        let tools = json!([{"name":"typed","input_schema":{"type":"object","properties":{"value":schema}}}]);
        let mut panel = operator::Panel::new(
            observation(&app, target),
            &tools,
            &json!([]),
            &json!([]),
            None,
        )
        .unwrap();
        panel.input(&event(KeyCode::Enter));
        for code in keys {
            panel.input(&event(code));
        }
        panel.input(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )));
        panel_draw(|f, a| panel.render(f, a));
        let operator::Outcome::Submit(request) = panel.input(&event(KeyCode::Enter)) else {
            panic!("typed review did not submit")
        };
        assert_eq!(request.arguments, json!({"value":expected}));
        assert_eq!(request.observation, observation(&app, target));
    }
}

#[test]
fn operator_editing_review_backtracking_and_optional_deletion_are_lossless() {
    let (_fixture, app, target) = coverage_support::app();
    let tools = json!([{"name":"edit","input_schema":{"type":"object","properties":{"text":{"type":"string"}}}}]);
    let mut panel = operator::Panel::new(
        observation(&app, target),
        &tools,
        &json!([]),
        &json!([]),
        None,
    )
    .unwrap();
    panel.input(&event(KeyCode::Enter));
    panel.input(&Event::Paste("discard me".into()));
    panel.input(&event(KeyCode::Delete));
    panel.input(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::CONTROL,
    )));
    panel_draw(|f, a| panel.render(f, a));
    let operator::Outcome::Submit(request) = panel.input(&event(KeyCode::Enter)) else {
        panic!("optional omission")
    };
    assert_eq!(request.arguments, json!({}));
    panel.input(&event(KeyCode::Esc));
    panel.input(&Event::Paste("a界".into()));
    panel.input(&event(KeyCode::Backspace));
    panel.input(&event(KeyCode::Char('b')));
    panel.input(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::ALT,
    )));
    panel.input(&event(KeyCode::Char('c')));
    panel.input(&event(KeyCode::Home));
    panel.input(&event(KeyCode::End));
    panel.input(&event(KeyCode::Tab));
    panel.input(&event(KeyCode::BackTab));
    panel.input(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::CONTROL,
    )));
    assert!(matches!(
        panel.input(&event(KeyCode::Enter)),
        operator::Outcome::Stay
    ));
    panel_draw(|f, a| panel.render(f, a));
    let operator::Outcome::Submit(request) = panel.input(&event(KeyCode::Enter)) else {
        panic!("edited review")
    };
    assert_eq!(request.arguments, json!({"text":"ab\nc"}));
    panel.input(&event(KeyCode::Esc));
    panel.input(&event(KeyCode::Esc));
    assert!(matches!(
        panel.input(&event(KeyCode::Esc)),
        operator::Outcome::Close
    ));
}

#[test]
fn operator_invalid_fields_and_registry_search_do_not_execute() {
    let (_fixture, app, target) = coverage_support::app();
    for (schema, input) in [
        (json!({"type":"integer","minimum":2}), "1"),
        (json!({"type":"integer","maximum":2}), "3"),
        (json!({"type":"integer"}), "1.5"),
        (json!({"type":"number"}), "not numeric"),
        (json!({"type":"string","minLength":3}), "x"),
        (json!({"type":"string","maxLength":2}), "long"),
    ] {
        let tools = json!([{"name":"validate","input_schema":{"type":"object","properties":{"value":schema},"required":["value"]}}]);
        let mut panel = operator::Panel::new(
            observation(&app, target),
            &tools,
            &json!([]),
            &json!([]),
            None,
        )
        .unwrap();
        panel.input(&Event::Paste("no-match".into()));
        assert!(matches!(
            panel.input(&event(KeyCode::Enter)),
            operator::Outcome::Stay
        ));
        for _ in 0..8 {
            panel.input(&event(KeyCode::Backspace));
        }
        panel.input(&event(KeyCode::Down));
        panel.input(&event(KeyCode::Up));
        panel.input(&event(KeyCode::Enter));
        panel.input(&Event::Paste(input.into()));
        assert!(matches!(
            panel.input(&Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::CONTROL
            ))),
            operator::Outcome::Stay
        ));
        panel_draw(|f, a| panel.render(f, a));
        assert!(matches!(
            panel.input(&event(KeyCode::Enter)),
            operator::Outcome::Stay
        ));
        panel.input(&Event::Resize(80, 24));
        panel.input(&event(KeyCode::PageDown));
        panel.input(&event(KeyCode::PageUp));
        assert!(!panel_draw(|f, a| panel.render(f, a)).is_empty());
    }
}

#[test]
fn operator_branched_registry_filters_management_actions_and_pins_const_action() {
    let (_fixture, app, target) = coverage_support::app();
    let tools = json!({"value":{"source":"builtin_preflight","inventory":[
        {"name":"todo","input_schema":{"type":"object","properties":{"title":{"type":"string"}},
            "oneOf":[{"type":"object","properties":{"action":{"const":"create"},"title":{"minLength":1}},"required":["action","title"]}]}},
        {"name":"unrelated","input_schema":{"type":"object","properties":{}}}
    ]}});
    let mut panel = operator::Panel::new(
        observation(&app, target),
        &tools,
        &json!([]),
        &json!([]),
        Some("todo"),
    )
    .unwrap();
    let text = panel_draw(|f, a| panel.render(f, a));
    assert!(text.contains("todo"));
    assert!(!text.contains("unrelated"));
    panel.input(&event(KeyCode::Enter));
    panel.input(&Event::Paste("Synthetic task".into()));
    panel.input(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::CONTROL,
    )));
    panel_draw(|f, a| panel.render(f, a));
    let operator::Outcome::Submit(request) = panel.input(&event(KeyCode::Enter)) else {
        panic!("branched action")
    };
    assert_eq!(request.name, "todo");
    assert_eq!(
        request.arguments,
        json!({"action":"create","title":"Synthetic task"})
    );
}

#[test]
fn notification_settlement_canonical_time_and_attention_gate_full_frames() {
    let (_fixture, mut app, target) = coverage_support::app();
    let now = chrono::Utc::now();
    let run = Uuid::new_v4();
    for state in ["completed", "failed", "cancelled", "interrupted"] {
        let view = app.views.get_mut(&target).unwrap();
        view.snapshot = Some(
            serde_json::from_value(
                json!({"session_id":target.session,"revision":17,"model":"fixture",
            "run":{"run_id":run,"state":state},"messages":[],"decisions":[]}),
            )
            .unwrap(),
        );
        view.settlement = None;
        assert!(view.observe_settlement(now));
        assert!(!view.observe_settlement(now + chrono::Duration::seconds(30)));
        assert!(!view.sidebar_settled(now - chrono::Duration::seconds(1), 0));
        assert!(!view.sidebar_settled(now, 1));
        assert!(view.sidebar_settled(now + chrono::Duration::seconds(1), 1));
        view.error = Some("synthetic attention required".into());
        assert!(!view.sidebar_settled(now, 0));
        view.error = None;
        view.connection_unavailable = true;
        assert!(!view.sidebar_settled(now, 0));
        view.connection_unavailable = false;
        app.presentation_now = now;
        for (w, h) in [(60, 20), (120, 40)] {
            assert!(!draw(&app, w, h).is_empty());
        }
    }
    let view = app.views.get_mut(&target).unwrap();
    view.snapshot.as_mut().unwrap().run.as_mut().unwrap().state = "running".into();
    assert!(view.observe_settlement(now));
    assert!(view.settlement.is_none());
    assert!(!view.sidebar_settled(now, 0));
    view.snapshot = None;
    assert!(!view.observe_settlement(now));
}

#[tokio::test]
async fn slash_navigation_records_history_only_after_success_and_leaves_prompt_unsent() {
    for command in ["/help", "/actions", "/settings"] {
        let (_fixture, mut app, target) = coverage_support::app();
        app.views
            .get_mut(&target)
            .unwrap()
            .draft
            .set_text(command.into());
        app.send().unwrap();
        assert!(app.views[&target].draft.text.is_empty());
        assert!(app.views[&target].pending.is_none());
        assert!(app.route_tasks.is_empty());
        assert!(!draw(&app, 120, 40).is_empty());
        match command {
            "/help" => assert!(app.help && !app.discovery.settings),
            "/settings" => assert!(app.help && app.discovery.settings),
            "/actions" => assert!(app.explore.is_some()),
            _ => assert!(app.views[&target].panel.is_none()),
        }
    }
    let (_fixture, mut app, target) = coverage_support::app();
    for command in ["/unknown-action", "/receipt", "/clear wrong-id"] {
        app.views
            .get_mut(&target)
            .unwrap()
            .draft
            .set_text(command.into());
        assert!(app.send().is_err());
        assert_eq!(app.views[&target].draft.text, command);
        assert!(app.views[&target].pending.is_none());
    }
}

#[tokio::test]
async fn discovery_availability_reasons_track_live_observation_not_only_selection() {
    let (_fixture, mut app, target) = coverage_support::app();
    for name in ["stop", "cancel", "receipt", "approve", "deny", "answer"] {
        assert!(app.discovery_reason(name).is_some(), "{name}");
    }
    app.selected = None;
    for name in ["stop", "receipt", "model", "workflows"] {
        assert!(app.discovery_reason(name).unwrap().contains("Select"));
    }
    for name in [
        "help", "actions", "settings", "new", "vessels", "voyages", "archived",
    ] {
        assert!(app.discovery_reason(name).is_none(), "{name}");
    }
    app.selected = Some(target);
    app.views.get_mut(&target).unwrap().snapshot = None;
    assert!(app.discovery_reason("model").unwrap().contains("snapshot"));
    app.discovery_open("model").unwrap();
    assert!(app.status.contains("snapshot"));
    app.views.remove(&target);
    assert!(
        app.discovery_reason("model")
            .unwrap()
            .contains("unavailable")
    );
    assert!(
        app.discovery_reason("configure")
            .unwrap()
            .contains("explicitly")
    );
}

#[test]
fn operator_request_encoding_rejects_ambiguous_names_and_oversized_public_payloads() {
    let (_fixture, app, target) = coverage_support::app();
    for name in ["two names", "tab\tname", "line\nname"] {
        let request = operator::Request {
            observation: observation(&app, target),
            name: name.into(),
            arguments: json!({}),
        };
        assert!(request.command_text().is_err());
    }
    let request = operator::Request {
        observation: observation(&app, target),
        name: "synthetic".into(),
        arguments: json!({"value":"x".repeat(65536)}),
    };
    assert!(request.command_text().is_err());
    let request = operator::Request {
        observation: observation(&app, target),
        name: "synthetic".into(),
        arguments: json!({"value":"界\nquoted \"text\""}),
    };
    let command = request.command_text().unwrap();
    let arguments = command.strip_prefix("/tool synthetic ").unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(arguments).unwrap(),
        request.arguments
    );
}
