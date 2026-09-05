use super::*;
use flow::{Answer, validate};

fn select(w: &mut Wizard, step: Step, choice: usize) {
    w.answers.insert(step, Answer::Choice(choice));
}

#[test]
fn all_role_and_connectivity_routes_are_navigable() {
    for role in 0..5 {
        for mask in 1..16 {
            for vessel in 0..4 {
                for reach in 0..4 {
                    for tunnel in 0..5 {
                        let mut w = Wizard::new();
                        select(&mut w, Step::Role, role);
                        w.answers.insert(
                            Step::Roles,
                            Answer::Roles(std::array::from_fn(|i| mask & (1 << i) != 0)),
                        );
                        select(&mut w, Step::Vessel, vessel);
                        select(&mut w, Step::Reach, reach);
                        select(&mut w, Step::Internet, 2);
                        select(&mut w, Step::Tunnel, tunnel);
                        let route = w.route();
                        assert_eq!(route.first(), Some(&Step::Role));
                        assert_eq!(route.last(), Some(&Step::Review));
                        for step in &route[..route.len() - 1] {
                            w.enter(*step);
                            assert!(!w.next());
                            assert!(w.error.is_none(), "{step:?}: {:?}", w.error);
                        }
                        assert_eq!(w.step(), Step::Review);
                        w.next();
                        assert_eq!(w.step(), Step::Done);
                        assert!(w.actions().last().unwrap().starts_with("NOT RUN:"));
                    }
                }
            }
        }
    }
}

#[test]
fn controller_only_and_vessel_only_skip_model_setup() {
    for role in [1, 3] {
        let mut w = Wizard::new();
        select(&mut w, Step::Role, role);
        assert!(!w.route().contains(&Step::Provider));
        assert!(!w.route().contains(&Step::Folder));
    }
    let mut w = Wizard::new();
    select(&mut w, Step::Role, 1);
    select(&mut w, Step::Controller, 1);
    assert!(w.route().contains(&Step::Provider));
}

#[test]
fn provider_branches_ask_only_relevant_questions() {
    for provider in 0..5 {
        for api in 0..3 {
            let mut w = Wizard::new();
            select(&mut w, Step::Provider, provider);
            select(&mut w, Step::Api, api);
            let route = w.route();
            assert_eq!(route.contains(&Step::Api), provider == 2);
            assert_eq!(
                route.contains(&Step::Endpoint),
                provider == 3 || (provider == 2 && api == 2)
            );
            assert_eq!(route.contains(&Step::Model), matches!(provider, 1..=3));
        }
    }
}

#[test]
fn back_preserves_answers_but_removed_branches_never_leak_into_review() {
    let mut w = Wizard::new();
    select(&mut w, Step::Role, 2);
    w.enter(Step::Name);
    w.input = "雪 remote".into();
    w.next();
    w.back();
    assert_eq!(w.input, "雪 remote");
    w.enter(Step::Role);
    w.cursor = 0;
    w.next();
    assert!(!w.summary().join("\n").contains("雪 remote"));
    assert!(!w.actions().join("\n").contains("enroll this Helm"));
    w.enter(Step::Role);
    w.cursor = 2;
    w.next();
    w.enter(Step::Name);
    assert_eq!(w.input, "雪 remote");
}

#[test]
fn text_and_url_validation_rejects_secrets_controls_and_invalid_addresses() {
    for value in ["", " ", "name\nnext", "\x1b]52;secret", &"x".repeat(513)] {
        assert!(validate(Step::Name, value).is_err());
    }
    for value in [
        "example.com",
        "ftp://host",
        "https://user:secret@host",
        "https://host?key=secret",
        "https://host/#secret",
        "https://ho st",
        "http://public.example",
    ] {
        assert!(validate(Step::Address, value).is_err(), "{value}");
    }
    for value in [
        "https://vessel.example",
        "http://127.0.0.1:9480",
        "http://[::1]:9480",
    ] {
        assert!(validate(Step::Address, value).is_ok(), "{value}");
    }
    assert!(validate(Step::Folder, "../relative").is_err());
    for value in ["/srv/work 雪", "C:\\work", "/tmp/$(touch nope)"] {
        assert!(validate(Step::Folder, value).is_ok());
    }
}

#[test]
fn empty_roles_and_invalid_text_block_advance() {
    let mut w = Wizard::new();
    select(&mut w, Step::Role, 4);
    w.enter(Step::Roles);
    w.roles = [false; 4];
    w.next();
    assert!(w.error.is_some());
    assert_eq!(w.step(), Step::Roles);
    w.enter(Step::Address);
    w.input = "bad".into();
    w.next();
    assert!(w.error.is_some());
    assert_eq!(w.step(), Step::Address);
}

#[test]
fn paste_is_atomic_and_keyboard_cancellation_is_distinct_from_finish() {
    let mut w = Wizard::new();
    w.enter(Step::Name);
    w.input = "雪".into();
    paste(&mut w, "\x1b[2J");
    assert_eq!(w.input, "雪");
    paste(&mut w, &"x".repeat(513));
    assert_eq!(w.input, "雪");
    handle_key(
        &mut w,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    );
    assert!(w.input.is_empty());
    assert!(matches!(
        handle_key(
            &mut w,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ),
        Action::Cancel
    ));
    w.enter(Step::Review);
    w.cursor = 2;
    assert!(matches!(
        handle_key(&mut w, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Cancel
    ));
}

#[test]
fn rendering_handles_every_step_and_tiny_resized_unicode_views() {
    let mut w = Wizard::new();
    select(&mut w, Step::Role, 4);
    w.answers.insert(Step::Roles, Answer::Roles([true; 4]));
    select(&mut w, Step::Reach, 2);
    select(&mut w, Step::Internet, 2);
    select(&mut w, Step::Tunnel, 1);
    select(&mut w, Step::Provider, 2);
    select(&mut w, Step::Api, 2);
    let mut route = w.route();
    route.extend([Step::Done, Step::Address]);
    for (width, height) in [(1, 1), (30, 10), (48, 20), (80, 24), (120, 40)] {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        for step in &route {
            w.enter(*step);
            w.input = "雪🙂e\u{301}".repeat(70);
            terminal.draw(|f| draw(f, &w)).unwrap();
            let buffer = terminal.backend().buffer();
            let screen = buffer
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            if width >= 48 && height >= 20 {
                assert!(screen.contains("SETUP PREVIEW"));
            }
        }
    }
}

#[test]
fn deferred_relay_and_sharing_never_claim_success() {
    let mut w = Wizard::new();
    select(&mut w, Step::Role, 4);
    w.answers
        .insert(Step::Roles, Answer::Roles([false, false, true, true]));
    select(&mut w, Step::Reach, 2);
    select(&mut w, Step::Internet, 2);
    select(&mut w, Step::Tunnel, 3);
    select(&mut w, Step::Sharing, 1);
    select(&mut w, Step::Approval, 2);
    let actions = w.actions().join("\n");
    assert!(actions.contains("relay does not exist"));
    assert!(actions.contains("existing sessions remain private"));
    assert!(actions.contains("approval-required work stays blocked or denied"));
    assert!(!w.route().contains(&Step::HostAddress));
}
