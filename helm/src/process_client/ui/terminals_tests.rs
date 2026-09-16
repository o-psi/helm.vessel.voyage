use super::*;
use crossterm::event::{KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn entry(title: &str, state: serde_json::Value) -> Entry {
    Entry {
        id: Uuid::new_v4(),
        title: title.into(),
        state,
    }
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn render(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| draw(frame, app, frame.area()))
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
fn entry_labels_roles_and_inventory_lifecycle_are_explicit() {
    for (state, label, role) in [
        (json!("running"), "Running", Role::Running),
        (json!({"exited":{"code":0}}), "Finished", Role::Completed),
        (
            json!({"exited":{"code":7}}),
            "Stopped with an error",
            Role::Failed,
        ),
        (json!({"exited":{"code":null}}), "Stopped", Role::Muted),
        (json!({}), "Status unavailable", Role::Muted),
    ] {
        let e = entry("fixture", state);
        assert_eq!(e.state(), label);
        assert_eq!(e.role().style(), role.style());
    }
    let mut browser = Browser::default();
    let run = Uuid::new_v4();
    let a = entry("one", json!("running"));
    let b = entry("two", json!("running"));
    browser.update(
        Ok(Inventory {
            run_id: Some(run),
            entries: vec![a.clone(), b.clone()],
        }),
        Instant::now(),
    );
    assert_eq!(browser.selected, Some(a.id));
    assert!(browser.summary().contains("2 programs running"));
    browser.selected = Some(b.id);
    browser.displayed.set(Some((run, run, b.id)));
    browser.update(
        Ok(Inventory {
            run_id: Some(run),
            entries: vec![b.clone()],
        }),
        Instant::now(),
    );
    assert_eq!(browser.selected, Some(b.id));
    assert!(browser.displayed.get().is_some());
    browser.update(
        Ok(Inventory {
            run_id: None,
            entries: vec![b],
        }),
        Instant::now(),
    );
    assert_eq!(browser.displayed.get(), None);
    browser.clear_displayed();
    browser.update(Err("bad\x1b".into()), Instant::now());
    assert_eq!(browser.summary(), "Program status unavailable");
    assert!(!browser.error.as_ref().unwrap().contains('\x1b'));
    browser.update(
        Ok(Inventory {
            run_id: None,
            entries: vec![entry("done", json!({"exited":{"code":0}}))],
        }),
        Instant::now(),
    );
    assert_eq!(browser.summary(), "1 finished program");
    browser.update(
        Ok(Inventory {
            run_id: None,
            entries: vec![],
        }),
        Instant::now(),
    );
    assert_eq!(browser.summary(), "");
}

#[tokio::test]
async fn offline_panel_renders_navigation_errors_and_stale_inventory_without_attaching() {
    let (_fixture, mut app, target) = super::super::coverage_support::app();
    app.clients.mark_unavailable(target.route);
    let a = entry("compile fixture", json!("running"));
    let b = entry("finished fixture", json!({"exited":{"code":0}}));
    app.views.get_mut(&target).unwrap().terminals.open = true;
    assert!(!render(&app, 100, 25).is_empty());
    {
        let browser = &mut app.views.get_mut(&target).unwrap().terminals;
        browser.open = true;
        browser.update(
            Ok(Inventory {
                run_id: Some(Uuid::new_v4()),
                entries: vec![a.clone(), b.clone()],
            }),
            Instant::now(),
        );
    }
    assert!(render(&app, 100, 25).contains("compile fixture"));
    // The synthetic connection is offline, so even a freshly painted running entry cannot attach.
    assert!(app.request_terminal(target, a.id).is_err());
    app.terminal_input(&key(KeyCode::Down)).unwrap();
    assert_eq!(app.views[&target].terminals.selected, Some(b.id));
    assert_eq!(app.views[&target].terminals.displayed.get(), None);
    assert!(app.terminal_input(&key(KeyCode::Enter)).is_err());
    assert!(app.terminal_request.is_none());
    app.terminal_input(&key(KeyCode::Up)).unwrap();
    assert_eq!(app.views[&target].terminals.selected, Some(a.id));
    app.terminal_input(&Event::Resize(40, 10)).unwrap();
    let mut release = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    app.terminal_input(&Event::Key(release)).unwrap();
    assert_eq!(app.views[&target].terminals.selected, Some(a.id));
    app.views.get_mut(&target).unwrap().terminals.observed =
        Some(Instant::now() - Duration::from_secs(6));
    assert!(render(&app, 100, 25).contains("Reconnecting"));
    app.views
        .get_mut(&target)
        .unwrap()
        .terminals
        .update(Err("unavailable fixture".into()), Instant::now());
    assert!(render(&app, 100, 25).contains("Reconnecting"));
    for size in [(1, 1), (20, 5), (80, 16)] {
        let _ = render(&app, size.0, size.1);
    }
    app.terminal_input(&key(KeyCode::Esc)).unwrap();
    assert!(!app.views[&target].terminals.open);
    app.selected = None;
    app.terminal_input(&key(KeyCode::Enter)).unwrap();
    assert!(render(&app, 100, 25).trim().is_empty());
}
