use super::super::coverage_support;
use super::*;

fn setup() -> (super::super::account_test_support::Fixture, App, Uuid) {
    let (fixture, mut app, target) = coverage_support::app();
    std::fs::create_dir_all(&app.clients[target.route].directory).unwrap();
    let mut config = crate::Config::default();
    config.workspace = Some(fixture.0.path().into());
    app.new_chat_config = Some(config);
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    let id = app.active_draft.unwrap();
    (fixture, app, id)
}

fn key(code: KeyCode) -> Event {
    Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
}

fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw_new_draft(frame, frame.area()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[tokio::test]
async fn empty_draft_is_reused_without_starting_a_process() {
    let (fixture, mut app, id) = setup();
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    assert_eq!(app.active_draft, Some(id));
    assert_eq!(app.new_drafts.len(), 1);
    assert_eq!(app.new_drafts[&id].navigation_title(), "New voyage");
    assert_eq!(app.new_drafts[&id].navigation_workspace(), fixture.0.path());
    assert!(app.new_drafts[&id].saved.start.is_none());
}

#[tokio::test]
async fn authored_draft_is_not_reused_and_titles_use_first_line() {
    let (fixture, mut app, id) = setup();
    app.new_draft_composer_mut(id)
        .unwrap()
        .insert_str("first line\nsecond line");
    assert_eq!(app.new_drafts[&id].navigation_title(), "first line");
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    assert_ne!(app.active_draft, Some(id));
    assert_eq!(app.new_drafts.len(), 2);
    assert_eq!(app.new_drafts[&id].composer.text, "first line\nsecond line");
}

#[tokio::test]
async fn draft_keyboard_edits_and_saves_without_sending() {
    let (_fixture, mut app, id) = setup();
    for event in [
        key(KeyCode::Char('a')),
        key(KeyCode::Char('b')),
        key(KeyCode::Left),
        key(KeyCode::Char('c')),
    ] {
        assert!(app.new_draft_input(&event).unwrap());
    }
    assert_eq!(app.new_drafts[&id].composer.text, "acb");
    assert_eq!(app.new_drafts[&id].saved.text, "acb");
    app.new_draft_input(&key(KeyCode::Backspace)).unwrap();
    app.new_draft_input(&key(KeyCode::Delete)).unwrap();
    assert_eq!(app.new_drafts[&id].saved.text, "a");
    assert!(app.new_drafts[&id].saved.start.is_none());
}

#[tokio::test]
async fn help_and_release_keys_leave_draft_untouched() {
    let (_fixture, mut app, id) = setup();
    let mut released = crossterm::event::KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    released.kind = crossterm::event::KeyEventKind::Release;
    assert!(app.new_draft_input(&Event::Key(released)).unwrap());
    app.new_draft_input(&key(KeyCode::F(1))).unwrap();
    assert!(app.status.contains("Draft commands:"));
    assert!(app.new_drafts[&id].composer.text.is_empty());
    app.draft_command(id, "/help").unwrap();
    assert!(app.status.contains("/discard"));
}

#[tokio::test]
async fn escape_returns_to_existing_voyage_and_inactive_input_is_ignored() {
    let (_fixture, mut app, id) = setup();
    assert_eq!(app.active_draft, Some(id));
    assert!(app.selected.is_none());
    app.new_draft_input(&key(KeyCode::Esc)).unwrap();
    assert!(app.active_draft.is_none());
    assert!(app.selected.is_some());
    assert!(!app.new_draft_input(&key(KeyCode::Char('x'))).unwrap());
}

#[tokio::test]
async fn busy_draft_rejects_commands_and_absorbs_edits() {
    let (_fixture, mut app, id) = setup();
    app.new_drafts.get_mut(&id).unwrap().busy = true;
    assert!(app.new_draft_input(&key(KeyCode::Char('x'))).unwrap());
    assert!(app.new_drafts[&id].composer.text.is_empty());
    assert!(app.draft_command(id, "/discard").is_err());
    assert!(app.draft_command(id, "/access approval").is_err());
    assert!(app.new_drafts.contains_key(&id));
}

#[tokio::test]
async fn access_commands_validate_and_persist_modes_without_starting() {
    let (_fixture, mut app, id) = setup();
    for (command, mode) in [
        ("read-only", crate::config::AccessMode::ReadOnly),
        ("approval", crate::config::AccessMode::Approval),
        ("unrestricted", crate::config::AccessMode::Unrestricted),
    ] {
        app.draft_command(id, &format!("/access {command}"))
            .unwrap();
        assert_eq!(
            app.new_drafts[&id].saved.config.as_ref().unwrap().access,
            Some(mode)
        );
    }
    assert!(app.draft_command(id, "/access bogus").is_err());
    assert!(app.new_drafts[&id].saved.start.is_none());
}

#[tokio::test]
async fn unsupported_commands_preserve_text_and_discard_removes_only_draft() {
    let (_fixture, mut app, id) = setup();
    app.new_draft_composer_mut(id)
        .unwrap()
        .insert_str("keep me");
    assert!(app.draft_command(id, "/unknown").is_err());
    assert_eq!(app.new_drafts[&id].composer.text, "keep me");
    app.draft_command(id, "/discard").unwrap();
    assert!(!app.new_drafts.contains_key(&id));
    assert!(app.active_draft.is_none());
    assert_eq!(app.views.len(), 1);
    assert_eq!(app.status, "Local draft discarded.");
}

#[tokio::test]
async fn draft_render_handles_empty_authored_and_busy_states() {
    let (_fixture, mut app, id) = setup();
    let empty = screen(&app, 100, 35);
    assert!(empty.contains("New voyage"));
    app.new_draft_composer_mut(id)
        .unwrap()
        .insert_str("rendered synthetic message");
    assert!(screen(&app, 100, 35).contains("rendered synthetic message"));
    app.new_drafts.get_mut(&id).unwrap().busy = true;
    assert!(!screen(&app, 100, 35).is_empty());
    for (width, height) in [(1, 1), (12, 5), (40, 12)] {
        let _ = screen(&app, width, height);
    }
    app.active_draft = None;
    assert!(screen(&app, 40, 12).trim().is_empty());
}

#[tokio::test]
async fn saved_draft_identity_matrix_refuses_partial_or_cross_bound_launch_state() {
    let (_fixture, app, id) = setup();
    let original = app.new_drafts[&id].saved.clone();
    original.validate_identity().unwrap();
    for case in [
        "nil",
        "start",
        "attempted_start",
        "process",
        "turn",
        "attempted_turn",
        "receipt",
    ] {
        let mut s = original.clone();
        match case {
            "nil" => s.id = Uuid::nil(),
            "start" => {
                s.start = Some(VesselCommand::Start {
                    command_id: Uuid::new_v4(),
                    session_id: Uuid::new_v4(),
                    workspace: s.workspace.clone(),
                })
            }
            "attempted_start" => s.start_attempted = true,
            "process" => {
                s.process = Some(ProcessInfo {
                    session_id: Uuid::new_v4(),
                    incarnation: Uuid::new_v4(),
                    workspace: s.workspace.clone(),
                    state: voyage_protocol::process::ProcessState::Live,
                    name: None,
                    catalogue: None,
                    archive: None,
                    deletion: None,
                })
            }
            "turn" => {
                s.submit = Some(VoyageCommand::Submit {
                    coordination: None,
                    command_id: Uuid::new_v4(),
                    expected_revision: 0,
                    expires_at_ms: 1,
                    prompt: "fixture".into(),
                })
            }
            "attempted_turn" => s.attempted = true,
            "receipt" => {
                s.receipt = Some(serde_json::json!({"command_id":s.turn,"status":"accepted"}))
            }
            _ => unreachable!(),
        };
        assert!(s.validate_identity().is_err(), "{case}");
    }
}
#[tokio::test]
async fn access_change_is_durable_without_erasing_text_and_refuses_inflight_or_remote_drafts() {
    let (_fixture, mut app, id) = setup();
    app.new_draft_composer_mut(id)
        .unwrap()
        .insert_str("preserve authored task");
    app.set_draft_access(id, crate::config::AccessMode::ReadOnly)
        .unwrap();
    assert_eq!(app.draft_access(id).unwrap(), "read-only");
    assert_eq!(app.new_drafts[&id].composer.text, "preserve authored task");
    assert_eq!(
        app.new_drafts[&id].saved.explicit.access,
        Some(crate::config::AccessMode::ReadOnly)
    );
    app.new_drafts.get_mut(&id).unwrap().busy = true;
    assert!(
        app.set_draft_access(id, crate::config::AccessMode::Unrestricted)
            .is_err()
    );
    app.new_drafts.get_mut(&id).unwrap().busy = false;
    app.new_drafts.get_mut(&id).unwrap().saved.config = None;
    assert!(app.draft_access(id).is_err());
    assert!(
        app.set_draft_access(id, crate::config::AccessMode::Unrestricted)
            .is_err()
    );
}

#[tokio::test]
async fn first_send_handoff_keeps_exact_text_on_rejection_and_observation_only_on_unknown() {
    for status in ["accepted", "rejected"] {
        let (_fixture, mut app, id) = setup();
        let mut saved = app.new_drafts[&id].saved.clone();
        saved.text = "authored first turn".into();
        saved.start = Some(VesselCommand::Start {
            command_id: Uuid::new_v4(),
            session_id: id,
            workspace: saved.workspace.clone(),
        });
        saved.start_attempted = true;
        saved.process = Some(ProcessInfo {
            session_id: id,
            incarnation: Uuid::new_v4(),
            workspace: saved.workspace.clone(),
            state: voyage_protocol::process::ProcessState::Live,
            name: None,
            catalogue: None,
            archive: None,
            deletion: None,
        });
        saved.submit = Some(VoyageCommand::Submit {
            coordination: None,
            command_id: saved.turn,
            expected_revision: 0,
            expires_at_ms: 1,
            prompt: saved.text.clone(),
        });
        saved.attempted = true;
        let turn = saved.turn;
        app.first_send_update(saved.clone(), Ok(None));
        assert!(app.new_drafts.contains_key(&id));
        assert!(app.status.contains("not confirmed"));
        app.first_send_update(
            saved,
            Ok(Some(
                serde_json::json!({"command_id":turn,"status":status,"reason":"fixture"}),
            )),
        );
        assert!(!app.new_drafts.contains_key(&id));
        let target = app.selected.unwrap();
        assert_eq!(target.session, id);
        if status == "rejected" {
            assert_eq!(app.views[&target].draft.text, "authored first turn");
        }
        assert!(app.active_draft.is_none());
    }
}
#[tokio::test]
async fn cancelled_connection_reloads_durable_draft_and_never_replays_first_send() {
    let (_fixture, mut app, id) = setup();
    let route = app.new_drafts[&id].route;
    {
        let draft = app.new_drafts.get_mut(&id).unwrap();
        draft.saved.text = "durable text".into();
        storage::save(&draft.saved).unwrap();
        draft.composer.set_text("not saved".into());
        draft.busy = true;
    }
    app.cancel_new_drafts(route.id).unwrap();
    assert_eq!(app.new_drafts[&id].composer.text, "durable text");
    assert!(!app.new_drafts[&id].busy);
    assert!(app.new_drafts[&id].saved.start.is_none());
    app.recover_new_drafts().unwrap();
    assert_eq!(app.new_drafts.len(), 1);
    app.reactivate_drafts(route).unwrap();
    assert_eq!(app.new_drafts[&id].composer.text, "durable text");
}

#[tokio::test]
async fn draft_recovery_matches_exact_route_and_preserves_legacy_bytes() {
    let (fixture, mut app, id) = setup();
    let client = app.clients[app.new_drafts[&id].route].clone();
    let mut saved = app.new_drafts[&id].saved.clone();
    saved.text = "recover exact authored text".into();
    saved.route = serde_json::to_string(&(
        client.directory.clone(),
        Option::<String>::None,
        client.access_file.clone(),
    ))
    .unwrap();
    storage::save(&saved).unwrap();
    let original = std::fs::read(
        fixture
            .0
            .path()
            .join("helm-new-drafts")
            .join(format!("{id}.json")),
    )
    .unwrap();
    app.new_drafts.remove(&id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let recovered = loop {
        let value = storage::recover(std::iter::once(&client)).unwrap();
        if value.contains_key(&id) {
            break value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "draft lock never released"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    };
    assert_eq!(recovered[&id].composer.text, "recover exact authored text");
    assert_eq!(recovered[&id].saved.route, storage::route(&client).unwrap());
    assert_eq!(
        std::fs::read(
            fixture
                .0
                .path()
                .join("helm-new-drafts")
                .join(format!("{id}.legacy"))
        )
        .unwrap(),
        original
    );
    drop(recovered);
    let wrong = Client::local(fixture.0.path().join("different"));
    assert!(
        storage::recover(std::iter::once(&wrong))
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn finished_draft_records_are_retired_without_reintroducing_a_send() {
    let (fixture, mut app, id) = setup();
    let mut saved = app.new_drafts[&id].saved.clone();
    saved.finished = true;
    storage::save(&saved).unwrap();
    assert!(storage::reload(id).unwrap().is_none());
    assert!(
        !fixture
            .0
            .path()
            .join("helm-new-drafts")
            .join(format!("{id}.json"))
            .exists()
    );
    app.new_drafts.remove(&id);
    assert!(storage::recover(app.clients.iter()).unwrap().is_empty());
}
