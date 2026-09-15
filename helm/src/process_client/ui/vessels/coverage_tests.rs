//! Synthetic private-setup journeys; never connect or submit credentials.
use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};

fn key(manager: &mut Manager, code: KeyCode) -> Vec<Action> {
    let (sender, _) = mpsc::channel(8);
    manager.input(
        &TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        &sender,
    )
}
fn draw(manager: &mut Manager, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| manager.draw(f, f.area())).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn local_actions_are_observation_only_and_do_not_create_connections() {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = Manager::open(dir.path().into()).unwrap();
    assert!(key(&mut manager, KeyCode::Char('c')).is_empty());
    for (code, expected) in [('c', 0), ('d', 1), ('u', 2), ('w', 3), ('v', 4), ('l', 5)] {
        manager.open_panel();
        let actions = key(&mut manager, KeyCode::Char(code));
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            (&actions[0], expected),
            (Action::ConnectLocal, 0)
                | (Action::DisconnectLocal, 1)
                | (Action::RetryUnavailable, 2)
                | (Action::New(None), 3)
                | (Action::FilterLocal, 4)
                | (Action::Filter(None), 5)
        ));
        assert!(manager.records().is_empty());
    }
}

#[test]
fn pairing_fields_mask_input_strip_controls_and_clear_on_mode_switch() {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = Manager::open(dir.path().into()).unwrap();
    manager.open_panel();
    key(&mut manager, KeyCode::Char('a'));
    assert!(matches!(
        manager.panel.page,
        Page::Form { import: false, .. }
    ));
    let (sender, _) = mpsc::channel(8);
    manager.input(
        &TerminalEvent::Paste("https://example.invalid\n\t".into()),
        &sender,
    );
    assert_eq!(&*manager.panel.fields[0], "https://example.invalid");
    key(&mut manager, KeyCode::Tab);
    manager.input(
        &TerminalEvent::Paste("synthetic-invitation".into()),
        &sender,
    );
    let output = draw(&mut manager, 110, 40);
    assert!(output.contains("HTTPS endpoint"));
    assert!(output.contains("masked"));
    assert!(!output.contains("synthetic-invitation"));
    assert!(output.contains("https://example.invalid"));
    key(&mut manager, KeyCode::Tab);
    manager.input(&TerminalEvent::Paste("Friendly alias".into()), &sender);
    assert_eq!(&*manager.panel.fields[2], "Friendly alias");
    key(&mut manager, KeyCode::Tab);
    assert!(!manager.panel.focus.is_empty());
    let auto = manager.panel.autoconnect;
    key(&mut manager, KeyCode::Char(' '));
    assert_ne!(auto, manager.panel.autoconnect);
    manager.focus_control(Button::Import);
    key(&mut manager, KeyCode::Enter);
    assert!(matches!(
        manager.panel.page,
        Page::Form { import: true, .. }
    ));
    assert!(manager.panel.fields.iter().all(|s| s.is_empty()));
    assert!(draw(&mut manager, 110, 40).contains("Access-file path"));
    key(&mut manager, KeyCode::Esc);
    assert!(matches!(manager.panel.page, Page::List));
    assert!(manager.is_open());
    key(&mut manager, KeyCode::Esc);
    assert!(!manager.is_open());
}

#[test]
fn setup_editing_limits_utf8_and_ignores_release_modified_and_busy_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = Manager::open(dir.path().into()).unwrap();
    manager.open_panel();
    key(&mut manager, KeyCode::Char('i'));
    let (sender, _) = mpsc::channel(8);
    manager.input(&TerminalEvent::Paste("界".repeat(4000)), &sender);
    assert_eq!(manager.panel.fields[0].len(), 8190);
    key(&mut manager, KeyCode::Backspace);
    assert_eq!(manager.panel.fields[0].len(), 8187);
    manager.input(
        &TerminalEvent::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        &sender,
    );
    assert!(manager.panel.fields[0].is_empty());
    let mut release = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    release.kind = crossterm::event::KeyEventKind::Release;
    manager.input(&TerminalEvent::Key(release), &sender);
    manager.input(
        &TerminalEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
        &sender,
    );
    assert!(manager.panel.fields[0].is_empty());
    manager.panel.busy = Some((Uuid::new_v4(), None));
    manager.input(&TerminalEvent::Paste("ignored".into()), &sender);
    assert!(manager.panel.fields[0].is_empty());
    assert!(manager.panel.focus.current() == Some(&Button::Back));
    key(&mut manager, KeyCode::Esc);
    assert!(!manager.is_open());
}

#[test]
fn destructive_confirmation_starts_on_back_and_resize_invalidates_hits() {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = Manager::open(dir.path().into()).unwrap();
    manager.open_panel();
    manager.panel.page = Page::Forget(Uuid::new_v4());
    let output = draw(&mut manager, 110, 40);
    assert!(output.contains("Forget this saved connection locally?"));
    assert!(output.contains("does NOT revoke remote access"));
    assert!(manager.panel.focus.current() == Some(&Button::Back));
    assert!(!manager.panel.hits.is_empty());
    let (sender, _) = mpsc::channel(8);
    manager.input(&TerminalEvent::Resize(50, 20), &sender);
    assert!(manager.panel.hits.is_empty());
    key(&mut manager, KeyCode::Enter);
    assert!(matches!(manager.panel.page, Page::List));
    manager.panel.page = Page::Rename(Uuid::new_v4());
    *manager.panel.fields[0] = "synthetic alias".into();
    assert!(draw(&mut manager, 110, 40).contains("authenticated identity is unchanged"));
    key(&mut manager, KeyCode::Esc);
    assert!(manager.panel.fields.iter().all(|s| s.is_empty()));
}

#[test]
fn list_scrolling_and_small_layouts_keep_selection_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = Manager::open(dir.path().into()).unwrap();
    manager.open_panel();
    manager.pending = vec![Uuid::new_v4(), Uuid::new_v4()];
    for _ in 0..10 {
        key(&mut manager, KeyCode::Down);
    }
    assert_eq!(manager.panel.selected, 2);
    for _ in 0..10 {
        key(&mut manager, KeyCode::Up);
    }
    assert_eq!(manager.panel.selected, 0);
    key(&mut manager, KeyCode::PageDown);
    assert_eq!(manager.panel.scroll, 8);
    key(&mut manager, KeyCode::PageUp);
    assert_eq!(manager.panel.scroll, 0);
    let (sender, _) = mpsc::channel(8);
    for kind in [MouseEventKind::ScrollDown, MouseEventKind::ScrollUp] {
        manager.input(
            &TerminalEvent::Mouse(MouseEvent {
                kind,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
            &sender,
        );
    }
    assert_eq!(manager.panel.scroll, 0);
    let output = draw(&mut manager, 110, 40);
    assert!(output.contains("Vessel"));
    for (width, height) in [(1, 1), (12, 5), (40, 12)] {
        let _ = draw(&mut manager, width, height);
        for (rect, _) in &manager.panel.hits {
            assert!(rect.right() <= width && rect.bottom() <= height);
        }
    }
}

#[test]
fn states_are_generation_fenced_and_removal_does_not_forget_access() {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = Manager::open(dir.path().into()).unwrap();
    let id = Uuid::new_v4();
    manager.set_state(id, 3, ConnectionState::Connected);
    manager.set_state(id, 2, ConnectionState::Offline);
    assert_eq!(manager.states[&id].1.label(), "Connected");
    manager.set_state(id, 3, ConnectionState::AccessRevoked);
    assert_eq!(manager.states[&id].1.label(), "Access revoked");
    for (state, label) in [
        (ConnectionState::Connecting, "Connecting"),
        (ConnectionState::Offline, "Unavailable"),
        (ConnectionState::AccessExpired, "Access expired"),
        (
            ConnectionState::AccessUnavailable,
            "Access unavailable (expired or revoked)",
        ),
        (ConnectionState::IdentityChanged, "Identity changed"),
        (ConnectionState::UnsupportedVersion, "Unsupported version"),
    ] {
        assert_eq!(state.label(), label);
    }
    assert_eq!(manager.autoconnect().count(), 0);
    assert!(manager.stop_tasks().is_empty());
}

#[test]
fn saved_connection_list_renders_scope_health_and_provider_readiness() {
    use crate::process_client::connections::{Metadata, Workspace};
    let dir = tempfile::tempdir().unwrap();
    let mut manager = Manager::open(dir.path().into()).unwrap();
    manager.open_panel();
    let id = Uuid::new_v4();
    let workspace = Uuid::new_v4();
    let connection = Connection {
        id,
        alias: "Synthetic remote".into(),
        endpoint: "https://example.invalid".into(),
        vessel_id: Uuid::new_v4(),
        principal_id: Some(Uuid::new_v4()),
        grant_id: Uuid::new_v4(),
        scope: Scope::Workspaces {
            workspace_ids: vec![workspace],
        },
        credential_ref: Uuid::new_v4(),
        autoconnect: true,
        workspace_preference: Some(workspace),
        revision: 3,
        forgotten: false,
        legacy_route: None,
        metadata: Metadata {
            version: Some("fixture-version".into()),
            rights: vec!["inspect".into()],
            expires_at_ms: Some(1),
            workspaces: vec![Workspace {
                id: workspace,
                name: "Synthetic workspace".into(),
                path: "/synthetic".into(),
                provider_ready: Some(false),
            }],
            features: vec!["start_settings".into()],
            grant_revision: Some(2),
        },
    };
    manager.records.push(connection);
    manager.panel.selected = 1;
    manager.set_state(id, 1, ConnectionState::Connected);
    let output = draw(&mut manager, 110, 40);
    assert!(output.contains("Synthetic remote"));
    assert!(output.contains("Connected"));
    assert_eq!(manager.autoconnect().count(), 1);
    assert!(matches!(&key(&mut manager,KeyCode::Char('c'))[0],Action::Activate(c) if c.id==id));
    assert!(
        matches!(key(&mut manager,KeyCode::Char('d'))[0],Action::Disconnect(candidate) if candidate==id)
    );
    manager.records[0].forgotten = true;
    manager.records[0].scope = Scope::Session {
        session_id: Uuid::new_v4(),
    };
    assert_eq!(manager.autoconnect().count(), 0);
    assert!(key(&mut manager, KeyCode::Char('c')).is_empty());
    assert!(
        manager
            .panel
            .notice
            .contains("Restore this original connection first")
    );
    let output = draw(&mut manager, 110, 40);
    assert!(output.contains("Forgotten"));
    assert!(output.contains("Shared conversation"));
}

#[test]
fn setup_failures_are_classified_without_retaining_diagnostics_and_stale_results_ignored() {
    use crate::process_client::access::ConnectionFailure;
    let dir = tempfile::tempdir().unwrap();
    let mut manager = Manager::open(dir.path().into()).unwrap();
    manager.open_panel();
    for (error, expected) in [
        (ConnectionFailure::Unavailable, "Access unavailable"),
        (ConnectionFailure::Expired, "expired"),
        (ConnectionFailure::Revoked, "revoked"),
        (ConnectionFailure::Identity, "identity"),
        (ConnectionFailure::Version, "version"),
        (ConnectionFailure::Offline, "offline"),
        (ConnectionFailure::Redirect, "redirect"),
    ] {
        let operation = Uuid::new_v4();
        manager.panel.busy = Some((operation, None));
        let failure = SetupFailure::from_error(&anyhow::Error::new(error));
        let message = failure.message().to_lowercase();
        assert!(message.contains(&expected.to_lowercase()), "{message}");
        assert!(
            manager
                .apply(Event {
                    operation: Uuid::new_v4(),
                    result: Err(SetupFailure::Timeout)
                })
                .is_empty()
        );
        assert!(manager.panel.busy.is_some());
        manager.apply(Event {
            operation,
            result: Err(failure),
        });
        assert!(manager.panel.busy.is_none());
        assert!(manager.panel.notice.to_lowercase().starts_with(&message));
    }
    let failure = SetupFailure::from_error(&anyhow::anyhow!("synthetic private diagnostic"));
    assert!(!failure.message().contains("synthetic private diagnostic"));
}
