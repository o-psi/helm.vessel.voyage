use super::*;
use crate::process_client::ui::{account_test_support::Fixture, coverage_support};
use crossterm::event::{KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};

fn setup(manage: bool) -> (Fixture, App, Target) {
    let (fixture, mut app, target) = coverage_support::app();
    let profiles = (0..3)
        .map(|index| ExecutionProfile {
            id: Uuid::new_v4(),
            name: format!("Profile {index}"),
            model: format!("model-{index}"),
            account: voyage_protocol::accounts::AccountBinding {
                account_id: Uuid::new_v4(),
                connection_id: Uuid::new_v4(),
                identity_generation: 1,
                connection_revision: 1,
                transport: voyage_protocol::accounts::Transport::OpenaiResponses,
            },
            reasoning_effort: Some("high".into()),
            service_tier: None,
        })
        .collect();
    app.inference.profiles.panel = Some(Panel {
        destination: Destination::Live(target),
        catalogue: Some(ProfileCatalogue {
            revision: 8,
            profiles,
            default_profile_id: None,
            can_manage: manage,
        }),
        selected: 0,
        name: None,
        delete_confirm: false,
        notice: "Ready".into(),
    });
    (fixture, app, target)
}
fn key(app: &mut App, code: KeyCode) {
    assert!(
        app.profiles_input(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap()
    );
}
fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            assert!(app.draw_profiles(frame));
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn profiles_render_and_mouse_navigation_use_the_same_selection_as_keyboard() {
    let (_fixture, mut app, _) = setup(true);
    for (width, height) in [(40, 18), (80, 24), (140, 40)] {
        let text = draw(&app, width, height);
        assert!(text.contains("Profile"));
        let rows = app.inference.profiles.rows.borrow().clone();
        assert!(!rows.is_empty());
        for (rect, index) in rows {
            assert!(
                app.profiles_input(&Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: rect.x,
                    row: rect.y,
                    modifiers: KeyModifiers::NONE,
                }))
                .unwrap()
            );
            assert_eq!(
                app.inference.profiles.panel.as_ref().unwrap().selected,
                index
            );
        }
    }
    for _ in 0..8 {
        key(&mut app, KeyCode::Down);
    }
    assert_eq!(app.inference.profiles.panel.as_ref().unwrap().selected, 2);
    for _ in 0..8 {
        key(&mut app, KeyCode::Up);
    }
    assert_eq!(app.inference.profiles.panel.as_ref().unwrap().selected, 0);
    let mut release = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    assert!(app.profiles_input(&Event::Key(release)).unwrap());
    assert_eq!(app.inference.profiles.panel.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::Esc);
    assert!(app.inference.profiles.panel.is_none());
}

#[test]
fn read_only_profile_catalogue_has_no_mutation_controls_or_effects() {
    let (_fixture, mut app, target) = setup(false);
    let screen = draw(&app, 120, 30);
    assert!(!screen.contains("[Delete]"));
    assert!(!screen.contains("[New]"));
    for code in ['n', 'e', 'd', 'x', 'f'] {
        key(&mut app, KeyCode::Char(code));
        let panel = app.inference.profiles.panel.as_ref().unwrap();
        assert!(panel.name.is_none());
        assert!(!panel.delete_confirm);
        assert!(app.inference.profiles.job.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
}

#[test]
fn profile_name_and_delete_reviews_are_local_until_explicit_confirmation() {
    let (_fixture, mut app, _) = setup(true);
    key(&mut app, KeyCode::Char('n'));
    assert!(draw(&app, 100, 30).contains("name"));
    for ch in "Work界".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    key(&mut app, KeyCode::Backspace);
    assert_eq!(
        app.inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .as_ref()
            .unwrap()
            .1,
        "Work"
    );
    key(&mut app, KeyCode::Esc);
    assert!(
        app.inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .is_none()
    );
    key(&mut app, KeyCode::Char('d'));
    assert!(
        app.inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .as_ref()
            .unwrap()
            .1
            .ends_with("copy")
    );
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('x'));
    assert!(
        app.inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .delete_confirm
    );
    assert!(draw(&app, 100, 30).contains("Delete"));
    key(&mut app, KeyCode::Esc);
    assert!(
        !app.inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .delete_confirm
    );
    assert!(app.inference.profiles.job.is_none());
}

#[test]
fn profile_poll_distinguishes_waiting_interrupted_failed_and_loaded_catalogues() {
    for manage in [false, true] {
        let (_fixture, mut app, _) = setup(manage);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.inference.profiles.job = Some(receiver);
        app.poll_profiles();
        assert!(app.inference.profiles.job.is_some());
        assert!(draw(&app, 100, 30).contains("loading profiles"));
        key(&mut app, KeyCode::Char('n'));
        assert!(
            app.inference
                .profiles
                .panel
                .as_ref()
                .unwrap()
                .name
                .is_none()
        );
        drop(sender);
        app.poll_profiles();
        assert!(app.inference.profiles.job.is_none());
        assert!(
            app.inference
                .profiles
                .panel
                .as_ref()
                .unwrap()
                .notice
                .contains("interrupted")
        );
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.inference.profiles.job = Some(receiver);
        assert!(
            sender
                .send(Err(anyhow::anyhow!("synthetic refusal")))
                .is_ok()
        );
        app.poll_profiles();
        assert!(
            app.inference
                .profiles
                .panel
                .as_ref()
                .unwrap()
                .notice
                .contains("synthetic refusal")
        );
        app.inference.profiles.panel.as_mut().unwrap().selected = 9;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.inference.profiles.job = Some(receiver);
        assert!(
            sender
                .send(Ok((
                    Uuid::new_v4(),
                    ProfileCatalogue {
                        revision: 19,
                        can_manage: manage,
                        ..Default::default()
                    },
                    vec![]
                )))
                .is_ok()
        );
        app.poll_profiles();
        let panel = app.inference.profiles.panel.as_ref().unwrap();
        assert_eq!(panel.selected, 0);
        assert_eq!(panel.catalogue.as_ref().unwrap().revision, 19);
        assert!(panel.notice.contains(if manage {
            "Create a profile"
        } else {
            "No profiles available"
        }));
        assert!(app.inference.profiles.labels.is_empty());
    }
}
