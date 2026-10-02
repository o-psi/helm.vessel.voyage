//! Integrated read-only transcript journeys through production draw/input APIs.
//! Synthetic canonical metadata, no executor/provider/native storage claim.
use super::*;
use crate::process_client::ui::{App, coverage_support, state::Target};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use uuid::Uuid;

fn install(app: &mut App, target: Target, messages: Vec<Value>, extra: Value) {
    let mut value = json!({"session_id":target.session,"revision":17,"model":"synthetic-model","name":"Fixture","messages":messages,"total_messages":messages.len(),"decisions":[]});
    for (key, value_extra) in extra.as_object().unwrap() {
        value[key] = value_extra.clone();
    }
    let view = app.views.get_mut(&target).unwrap();
    view.snapshot = Some(serde_json::from_value(value).unwrap());
    view.transcript.borrow_mut().dirty = true;
}
fn message(index: usize, role: &str, content: impl Into<String>) -> Value {
    json!({"message_index":index,"role":role,"content":content.into()})
}
fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| super::draw(frame, app, frame.area()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
fn rows(app: &App, target: Target) -> String {
    app.views[&target]
        .transcript
        .borrow()
        .rows
        .iter()
        .map(|row| row.line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> bool {
    app.transcript_input(&Event::Key(KeyEvent::new(code, modifiers)))
}
fn canonical(app: &App, target: Target) -> Vec<Message> {
    app.views[&target]
        .snapshot
        .as_ref()
        .unwrap()
        .messages
        .clone()
}
fn unchanged(app: &App, target: Target, before: &[Message]) {
    assert!(
        canonical(app, target).as_slice() == before,
        "canonical metadata changed by display interaction"
    );
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(app.route_tasks.values().all(Vec::is_empty));
    assert!(app.views[&target].pending.is_none());
    assert_eq!(app.views[&target].process.session_id, target.session);
    assert_eq!(
        app.views[&target].snapshot.as_ref().unwrap().session_id,
        target.session
    );
}
fn tool_message(index: usize, calls: Vec<Value>) -> Value {
    json!({"message_index":index,"role":"assistant","content":"","tool_calls":calls})
}
fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"id":id,"name":name,"arguments":args})
}
fn result(index: usize, id: &str, content: impl Into<String>, success: bool) -> Value {
    json!({"message_index":index,"role":"tool","content":content.into(),"tool_call_id":id,"tool_success":success})
}

#[test]
fn unicode_reading_anchor_survives_actual_scroll_resize_growth_and_explicit_follow_latest() {
    let (_fixture, mut app, target) = coverage_support::app();
    let history = (0..24)
        .map(|i| {
            message(
                i,
                if i % 2 == 0 { "user" } else { "assistant" },
                format!(
                    "Canonical {i}: 日本語 café 👩‍💻 {}",
                    "long reading words ".repeat(8)
                ),
            )
        })
        .collect();
    install(&mut app, target, history, json!({}));
    draw(&app, 90, 16);
    assert!(key(&mut app, KeyCode::PageUp, KeyModifiers::NONE));
    draw(&app, 90, 16);
    let anchor = app.views[&target]
        .transcript
        .borrow()
        .anchor
        .clone()
        .unwrap();
    for width in [46, 110, 27, 90] {
        draw(&app, width, 16);
        let state = app.views[&target].transcript.borrow();
        let kept = state.anchor.as_ref().unwrap();
        assert_eq!(kept.key, anchor.key);
        assert_eq!(kept.offset, anchor.offset);
    }
    let before = canonical(&app, target);
    {
        let view = app.views.get_mut(&target).unwrap();
        let snapshot = view.snapshot.as_mut().unwrap();
        snapshot.messages.push(
            serde_json::from_value(message(24, "assistant", "Fresh canonical output")).unwrap(),
        );
        snapshot.total_messages = 25;
        view.transcript.borrow_mut().observe_growth(true);
        view.transcript.borrow_mut().dirty = true;
    }
    assert!(draw(&app, 90, 16).contains("New output"));
    assert!(app.views[&target].transcript.borrow().anchor.is_some());
    assert!(key(&mut app, KeyCode::End, KeyModifiers::CONTROL));
    assert!(draw(&app, 90, 16).contains("Fresh canonical output"));
    let state = app.views[&target].transcript.borrow();
    assert!(state.anchor.is_none());
    assert!(!state.new_output);
    drop(state);
    assert!(&canonical(&app, target)[..24] == before.as_slice());
    assert!(app.route_tasks.values().all(Vec::is_empty));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[test]
fn actual_search_is_display_only_and_does_not_find_hidden_tool_details_until_disclosed() {
    let (_fixture, mut app, target) = coverage_support::app();
    install(
        &mut app,
        target,
        vec![
            message(0, "user", "Visible question"),
            tool_message(
                1,
                vec![call("one", "read_file", json!({"path":"file.txt"}))],
            ),
            result(2, "one", "hidden-needle private fixture output", true),
            message(3, "assistant", "Public final answer"),
        ],
        json!({}),
    );
    draw(&app, 100, 24);
    let before = canonical(&app, target);
    assert!(key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL));
    assert!(app.transcript_input(&Event::Paste("hidden-needle".into())));
    assert!(key(&mut app, KeyCode::Enter, KeyModifiers::NONE));
    assert!(draw(&app, 100, 24).contains("Loaded display:"));
    assert!(app.views[&target].transcript.borrow().search_error);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    {
        let mut state = app.views[&target].transcript.borrow_mut();
        state.anchor = Some(Anchor {
            key: Key::Tool("one".into()),
            offset: 0,
        });
    }
    assert!(key(&mut app, KeyCode::Char(' '), KeyModifiers::CONTROL));
    draw(&app, 100, 24);
    key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    app.transcript_input(&Event::Paste("hidden-needle".into()));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    draw(&app, 100, 24);
    let state = app.views[&target].transcript.borrow();
    assert!(!state.search_error);
    assert_eq!(state.anchor.as_ref().unwrap().key, Key::Tool("one".into()));
    drop(state);
    unchanged(&app, target, &before);
}

#[test]
fn grouped_activity_mouse_and_keyboard_reveal_results_without_private_program_arguments() {
    let (_fixture, mut app, target) = coverage_support::app();
    let calls = vec![
        call(
            "process-private",
            "process",
            json!({"action":"write","name":"owned program","data":"NEVER-DISPLAY-INPUT","env":{"TOKEN":"NEVER-DISPLAY-ENV"}}),
        ),
        call("read", "read_file", json!({"path":"read.txt"})),
        call(
            "edit",
            "apply_patch",
            json!({"path":"edit.txt","patch":"--- a\n+++ b\n-old\n+new"}),
        ),
        call(
            "search",
            "search_files",
            json!({"query":"café","glob":"*.rs"}),
        ),
        call("list", "list_directory", json!({"recursive":true})),
    ];
    install(
        &mut app,
        target,
        vec![
            message(0, "user", "Inspect fixture work"),
            tool_message(1, calls),
            result(2, "process-private", "recorded program output", true),
            result(3, "read", "retained read output", true),
            result(4, "edit", "edit result", true),
            result(5, "search", "search result", true),
            result(6, "list", "list result", true),
        ],
        json!({}),
    );
    draw(&app, 100, 35);
    let before = canonical(&app, target);
    assert!(rows(&app, target).contains("older actions"));
    let hit = app.views[&target]
        .transcript
        .borrow()
        .hits
        .iter()
        .find(|(_, key)| matches!(key, Key::ActivityHeader(_)))
        .cloned()
        .unwrap();
    assert!(app.transcript_input(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.0.x,
        row: hit.0.y,
        modifiers: KeyModifiers::NONE
    })));
    draw(&app, 100, 35);
    assert!(rows(&app, target).contains("owned program"));
    {
        let mut state = app.views[&target].transcript.borrow_mut();
        state.anchor = Some(Anchor {
            key: Key::Tool("process-private".into()),
            offset: 0,
        });
    }
    key(&mut app, KeyCode::Char(' '), KeyModifiers::CONTROL);
    draw(&app, 100, 35);
    let text = rows(&app, target);
    assert!(text.contains("[withheld]"));
    assert!(text.contains("recorded program output"));
    assert!(!text.contains("NEVER-DISPLAY-INPUT"));
    assert!(!text.contains("NEVER-DISPLAY-ENV"));
    assert!(key(
        &mut app,
        KeyCode::Up,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT
    ));
    draw(&app, 52, 18);
    unchanged(&app, target, &before);
}

#[test]
fn typed_command_outcome_and_vessel_admission_are_not_misrepresented_as_completed_work() {
    use voyage_protocol::tool_result::{CommandOutcome, IncompleteReason, ToolOutcome};
    let (_fixture, mut app, target) = coverage_support::app();
    let outcome = ToolOutcome {
        command: Some(CommandOutcome::Exited { code: 7 }),
        incomplete: Some(IncompleteReason::OutputLimit),
        elapsed_ms: Some(65000),
        ..Default::default()
    };
    let mut command_result = result(1, "shell", "partial command output", true);
    command_result["tool_outcome"] = serde_json::to_value(outcome).unwrap();
    install(
        &mut app,
        target,
        vec![
            tool_message(
                0,
                vec![
                    call("shell", "shell", json!({"command":"owned fixture command"})),
                    call("vessel-refused", "vessel", json!({"action":"submit"})),
                    call("vessel-unknown", "vessel", json!({"action":"create"})),
                ],
            ),
            command_result,
            result(
                2,
                "vessel-refused",
                r#"{"submit":{"status":"refused"}}"#,
                true,
            ),
            result(
                3,
                "vessel-unknown",
                r#"{"start":{"status":"outcome_unknown"}}"#,
                true,
            ),
        ],
        json!({}),
    );
    draw(&app, 100, 35);
    let before = canonical(&app, target);
    let text = rows(&app, target);
    assert!(text.contains("exit 7"));
    assert!(text.contains("output incomplete"));
    assert!(text.contains("1m 05s"));
    assert!(text.contains("Refused"));
    assert!(text.contains("Unconfirmed"));
    assert!(!text.contains("Done · Create voyages"));
    unchanged(&app, target, &before);
}

#[test]
fn pending_delivery_disappears_only_when_exact_canonical_content_and_parts_are_saved() {
    let (_fixture, mut app, target) = coverage_support::app();
    install(
        &mut app,
        target,
        vec![message(0, "user", "same earlier text")],
        json!({}),
    );
    {
        let mut state = app.views[&target].transcript.borrow_mut();
        state.delivery = Some(Delivery {
            text: "same earlier text".into(),
            parts: vec![voyage_protocol::content::ContentPart::Text {
                text: "same earlier text".into(),
            }],
            before: 1,
            label: "Delivery unconfirmed".into(),
        });
    }
    draw(&app, 85, 24);
    assert!(
        app.views[&target]
            .transcript
            .borrow()
            .rows
            .iter()
            .any(|row| matches!(row.key, Key::Pending))
    );
    let old = canonical(&app, target);
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .messages
        .push(serde_json::from_value(message(1, "user", "different received text")).unwrap());
    app.views
        .get_mut(&target)
        .unwrap()
        .transcript
        .borrow_mut()
        .dirty = true;
    draw(&app, 85, 24);
    assert!(
        app.views[&target]
            .transcript
            .borrow()
            .rows
            .iter()
            .any(|row| matches!(row.key, Key::Pending))
    );
    {
        let view = app.views.get_mut(&target).unwrap();
        view.snapshot.as_mut().unwrap().messages[1].content = "same earlier text".into();
        view.transcript.borrow_mut().dirty = true;
    }
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .total_messages = 2;
    draw(&app, 85, 24);
    assert!(
        app.views[&target]
            .transcript
            .borrow()
            .rows
            .iter()
            .any(|row| matches!(row.key, Key::Pending)),
        "same text with different parts is not exact delivery"
    );
    let parts = app.views[&target]
        .transcript
        .borrow()
        .delivery
        .as_ref()
        .unwrap()
        .parts
        .clone();
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .messages[1]
        .parts = parts;
    app.views[&target].transcript.borrow_mut().dirty = true;
    draw(&app, 85, 24);
    assert!(
        !app.views[&target]
            .transcript
            .borrow()
            .rows
            .iter()
            .any(|row| matches!(row.key, Key::Pending))
    );
    assert!(canonical(&app, target)[0] == old[0]);
    assert!(app.route_tasks.values().all(Vec::is_empty));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[test]
fn live_suffix_becomes_saved_text_without_duplicate_and_cleanup_uncertainty_preserves_the_draft() {
    let (_fixture, mut app, target) = coverage_support::app();
    let run = Uuid::new_v4();
    install(
        &mut app,
        target,
        vec![message(0, "user", "Question")],
        json!({"run":{"run_id":run,"state":"running","live_text":"Provisional continuation","live_text_offset":0,"partial_text_bytes":24}}),
    );
    draw(&app, 100, 24);
    assert_eq!(
        rows(&app, target)
            .matches("Provisional continuation")
            .count(),
        1
    );
    {
        let view = app.views.get_mut(&target).unwrap();
        let snap = view.snapshot.as_mut().unwrap();
        snap.messages.push(
            serde_json::from_value(message(1, "assistant", "Provisional continuation")).unwrap(),
        );
        snap.total_messages = 2;
        snap.run.as_mut().unwrap().state = "completed".into();
        snap.run.as_mut().unwrap().live_text = Some(String::new());
        view.transcript.borrow_mut().dirty = true;
    }
    draw(&app, 100, 24);
    assert_eq!(
        rows(&app, target)
            .matches("Provisional continuation")
            .count(),
        1
    );
    let before = canonical(&app, target);
    for (phase, retryable, expected) in [
        ("running", false, "Finishing cleanup"),
        ("failed", true, "Cleanup needs attention"),
        ("unconfirmed", false, "cannot yet be verified"),
    ] {
        let view = app.views.get_mut(&target).unwrap();
        let snap = view.snapshot.as_mut().unwrap();
        snap.run.as_mut().unwrap().state = "interrupted".into();
        snap.pending_cleanup_run = Some(run);
        snap.cleanup=Some(serde_json::from_value(json!({"run_id":run,"phase":phase,"retryable":retryable,"pending":["owned child","private profile"],"reason":"Observed cleanup remains pending"})).unwrap());
        view.transcript.borrow_mut().dirty = true;
        draw(&app, 100, 24);
        let text = rows(&app, target);
        assert!(text.contains(expected));
        assert!(text.contains("Waiting for: owned child"));
        assert!(!text.contains("Ready to continue."));
        unchanged(&app, target, &before);
    }
}

#[test]
fn interrupted_turn_json_authored_text_and_image_metadata_preserve_canonical_order_and_identity() {
    use voyage_protocol::content::{ContentPart, ImageAttachment, ImageMediaType};
    let (_fixture, mut app, target) = coverage_support::app();
    let run = Uuid::new_v4();
    let interrupted = Uuid::new_v4();
    let mut first = message(0, "user", "fallback text must not duplicate parts");
    first["parts"] = serde_json::to_value(vec![
        ContentPart::Text {
            text: "Before attachment".into(),
        },
        ContentPart::Image {
            attachment: ImageAttachment {
                id: Uuid::new_v4(),
                name: "owned-image.png".into(),
                media_type: ImageMediaType::Png,
                byte_size: 70,
                width: 1,
                height: 2,
                sha256: "a".repeat(64),
            },
        },
        ContentPart::Text {
            text: "After attachment".into(),
        },
    ])
    .unwrap();
    let mut answer = message(
        1,
        "assistant",
        r#"{"status":"completed","command_id":"authored-not-a-receipt"}"#,
    );
    answer["interrupted_attempt"] = json!(interrupted);
    install(
        &mut app,
        target,
        vec![first, answer],
        json!({"turns":[{"run_id":run,"phase":"interrupted","message_start":0,"message_end":2,"failure_summary":"Owned interrupted attempt retained","started_at":"2026-01-01T00:00:00Z","finished_at":"2026-01-01T00:01:02Z"}]}),
    );
    draw(&app, 100, 28);
    let before = canonical(&app, target);
    let text = rows(&app, target);
    assert!(text.find("Before attachment").unwrap() < text.find("owned-image.png").unwrap());
    assert!(text.find("owned-image.png").unwrap() < text.find("After attachment").unwrap());
    assert!(!text.contains("fallback text must not duplicate parts"));
    assert!(text.contains("Interrupted response"));
    assert!(text.contains("authored-not-a-receipt"));
    assert!(text.contains("Work was not confirmed complete"));
    assert!(text.contains("1m 02s"));
    unchanged(&app, target, &before);
}

#[test]
fn searching_hidden_loaded_history_never_changes_canonical_revision_or_queues_a_command() {
    let (_fixture, mut app, target) = coverage_support::app();
    install(
        &mut app,
        target,
        (128..150)
            .map(|index| {
                message(
                    index,
                    if index % 2 == 0 { "user" } else { "assistant" },
                    format!("Loaded canonical row {index} 日本語"),
                )
            })
            .collect(),
        json!({"message_offset":128,"total_messages":150,"history_truncated":true}),
    );
    draw(&app, 88, 12);
    let before = canonical(&app, target);
    key(&mut app, KeyCode::Home, KeyModifiers::CONTROL);
    draw(&app, 88, 12);
    assert_eq!(
        app.views[&target].transcript.borrow().requested_from,
        Some(0)
    );
    assert!(app.views[&target].transcript.borrow().anchor.is_some());
    key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    app.transcript_input(&Event::Paste("row 130".into()));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    draw(&app, 88, 12);
    assert!(!app.views[&target].transcript.borrow().search_error);
    assert_eq!(app.views[&target].snapshot.as_ref().unwrap().revision, 17);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut app, KeyCode::Up, KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Up, KeyModifiers::CONTROL);
    draw(&app, 88, 12);
    assert_eq!(
        app.views[&target]
            .transcript
            .borrow()
            .anchor
            .as_ref()
            .unwrap()
            .key,
        Key::MessageHeading(128)
    );
    unchanged(&app, target, &before);
}

#[test]
fn live_reasoning_disclosure_finalizes_to_fresh_key_without_promoting_it_to_the_answer() {
    let (_fixture, mut app, target) = coverage_support::app();
    let run = Uuid::new_v4();
    let attempt = Uuid::new_v4();
    let previews = json!([{"attempt_id":attempt,"index":0,"kind":"summary","text":"Synthetic provider disclosure","truncated":false,"finalized":false}]);
    install(
        &mut app,
        target,
        vec![message(0, "user", "Question")],
        json!({"run":{"run_id":run,"state":"running","reasoning_previews":previews}}),
    );
    draw(&app, 100, 25);
    let id = format!("reasoning:{attempt}:0:Summary:false");
    {
        let mut state = app.views[&target].transcript.borrow_mut();
        state.anchor = Some(Anchor {
            key: Key::Tool(id.clone()),
            offset: 0,
        });
    }
    key(&mut app, KeyCode::Char(' '), KeyModifiers::CONTROL);
    draw(&app, 100, 25);
    assert!(rows(&app, target).contains("Synthetic provider disclosure"));
    assert!(rows(&app, target).contains("not the answer"));
    let before = canonical(&app, target);
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .run
        .as_mut()
        .unwrap()
        .reasoning_previews[0]
        .finalized = true;
    app.views[&target].transcript.borrow_mut().dirty = true;
    draw(&app, 100, 25);
    let state = app.views[&target].transcript.borrow();
    assert!(!state.tool_expanded.contains(&id));
    assert_eq!(
        state.anchor.as_ref().unwrap().key,
        Key::Tool(format!("reasoning:{attempt}:0:Summary:true"))
    );
    drop(state);
    assert!(!rows(&app, target).contains("Synthetic provider disclosure"));
    unchanged(&app, target, &before);
}

#[test]
fn viewport_hits_resize_and_modal_fences_do_not_route_input_into_another_display_or_draft() {
    let (_fixture, mut app, target) = coverage_support::app();
    install(
        &mut app,
        target,
        vec![
            message(0, "user", "Question"),
            tool_message(
                1,
                vec![call("view-only", "read_file", json!({"path":"owned"}))],
            ),
        ],
        json!({}),
    );
    draw(&app, 100, 25);
    let before = canonical(&app, target);
    assert!(!app.views[&target].transcript.borrow().hits.is_empty());
    app.transcript_input(&Event::Resize(50, 14));
    assert!(app.views[&target].transcript.borrow().hits.is_empty());
    draw(&app, 50, 14);
    let mut release = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);
    release.kind = crossterm::event::KeyEventKind::Release;
    assert!(!app.transcript_input(&Event::Key(release)));
    assert!(!key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL));
    app.help = true;
    assert!(!key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL));
    app.help = false;
    app.views.get_mut(&target).unwrap().panel = Some("Owned details panel".into());
    assert!(!key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL));
    app.views.get_mut(&target).unwrap().panel = None;
    app.views.get_mut(&target).unwrap().terminals.open = true;
    assert!(!key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL));
    app.views.get_mut(&target).unwrap().terminals.open = false;
    app.interactions.borrow_mut().focused = true;
    assert!(!key(&mut app, KeyCode::PageUp, KeyModifiers::NONE));
    app.interactions.borrow_mut().focused = false;
    unchanged(&app, target, &before);
}

#[test]
fn archive_deletion_and_unavailable_views_expose_only_the_correct_readonly_recovery_path() {
    use voyage_protocol::process::ProcessState;
    let (_fixture, mut app, target) = coverage_support::app();
    install(
        &mut app,
        target,
        vec![message(128, "assistant", "Saved complete fixture text")],
        json!({"message_offset":128,"total_messages":129,"lifecycle":{"archived":true}}),
    );
    draw(&app, 100, 24);
    let before = canonical(&app, target);
    let text = rows(&app, target);
    assert!(text.contains("Archived"));
    assert!(text.contains("Restore this voyage to load earlier messages"));
    assert!(text.contains("Saved complete fixture text"));
    unchanged(&app, target, &before);
    {
        let view = app.views.get_mut(&target).unwrap();
        view.snapshot.as_mut().unwrap().lifecycle = json!({"deleted":true});
        view.transcript.borrow_mut().dirty = true;
    }
    draw(&app, 100, 24);
    assert!(rows(&app, target).contains("has been deleted"));
    assert!(!rows(&app, target).contains("Saved complete fixture text"));
    unchanged(&app, target, &before);
    let identity = (
        app.views[&target].process.session_id,
        app.views[&target].process.incarnation,
    );
    for (state, error, expected) in [
        (ProcessState::Live, None, "Connecting"),
        (ProcessState::Unavailable, None, "recovering and respawning"),
        (
            ProcessState::Unavailable,
            Some("retained unknown cleanup".into()),
            "Saved conversation needs recovery",
        ),
    ] {
        let view = app.views.get_mut(&target).unwrap();
        view.snapshot = None;
        view.process.state = state;
        view.error = error;
        view.transcript.borrow_mut().dirty = true;
        draw(&app, 100, 24);
        assert!(rows(&app, target).contains(expected));
        assert_eq!(
            (
                app.views[&target].process.session_id,
                app.views[&target].process.incarnation
            ),
            identity
        );
        assert!(app.route_tasks.values().all(Vec::is_empty));
        assert!(app.views[&target].pending.is_none());
    }
}

#[test]
fn stale_transcript_hit_after_actual_target_switch_cannot_toggle_or_act_on_the_other_voyage() {
    use crate::process_client::ui::state::View;
    let (_fixture, mut app, target) = coverage_support::app();
    install(
        &mut app,
        target,
        vec![tool_message(
            0,
            (0..5)
                .map(|i| {
                    call(
                        &format!("owned-{i}"),
                        "read_file",
                        json!({"path":format!("file-{i}")}),
                    )
                })
                .collect(),
        )],
        json!({}),
    );
    draw(&app, 100, 24);
    let before = canonical(&app, target);
    let stale = app.views[&target]
        .transcript
        .borrow()
        .hits
        .iter()
        .find(|(_, key)| matches!(key, Key::ActivityHeader(_)))
        .cloned()
        .unwrap();
    let next = Target {
        route: target.route,
        session: Uuid::new_v4(),
    };
    let mut process = app.views[&target].process.clone();
    process.session_id = next.session;
    process.incarnation = Uuid::new_v4();
    let mut other = View::new(process);
    other.draft.insert_str("other preserved draft");
    app.views.insert(next, other);
    app.selected = Some(next);
    assert!(!app.transcript_input(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: stale.0.x,
        row: stale.0.y,
        modifiers: KeyModifiers::NONE
    })));
    assert!(app.views[&target].transcript.borrow().expanded.is_empty());
    assert!(app.views[&next].transcript.borrow().expanded.is_empty());
    assert_eq!(app.views[&next].draft.text, "other preserved draft");
    assert!(app.views[&next].pending.is_none());
    draw(&app, 100, 24);
    assert!(rows(&app, next).contains("Connecting"));
    assert!(app.route_tasks.values().all(Vec::is_empty));
    assert!(canonical(&app, target) == before);
}

#[test]
fn actual_artifact_summary_and_partial_saved_tool_details_never_read_or_execute_the_artifact() {
    use voyage_protocol::tool_result::{ArtifactReference, ToolContent, ToolOutput};
    let (_fixture, mut app, target) = coverage_support::app();
    let artifact = Uuid::new_v4();
    let output = ToolOutput {
        content: vec![ToolContent::Image {
            artifact: ArtifactReference {
                id: artifact,
                sha256: "b".repeat(64),
                name: "owned-fixture.png".into(),
                mime_type: "image/png".into(),
                byte_size: 70,
            },
        }],
        ..Default::default()
    };
    let mut saved = result(1, "artifact-call", "bounded saved output", true);
    saved["tool_output"] = serde_json::to_value(output).unwrap();
    saved["projection_truncated"] = json!(true);
    install(
        &mut app,
        target,
        vec![
            tool_message(
                0,
                vec![call(
                    "artifact-call",
                    "shell",
                    json!({"command":"owned fixture artifact description"}),
                )],
            ),
            saved,
        ],
        json!({}),
    );
    draw(&app, 100, 28);
    let before = canonical(&app, target);
    let text = rows(&app, target);
    assert!(text.contains("owned-fixture.png"));
    assert!(text.contains(&artifact.to_string()));
    assert!(text.contains("save with helm connect artifact"));
    assert!(!text.contains("bounded saved output"));
    {
        let mut state = app.views[&target].transcript.borrow_mut();
        state.anchor = Some(Anchor {
            key: Key::Tool("artifact-call".into()),
            offset: 0,
        });
    }
    key(&mut app, KeyCode::Char(' '), KeyModifiers::CONTROL);
    draw(&app, 100, 28);
    assert!(rows(&app, target).contains("bounded saved output"));
    assert!(rows(&app, target).contains("Partial history projection"));
    unchanged(&app, target, &before);
}
