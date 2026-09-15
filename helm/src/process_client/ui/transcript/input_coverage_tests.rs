use super::super::super::coverage_support;
use super::*;
use crossterm::event::KeyEvent;
fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> bool {
    app.transcript_input(&Event::Key(KeyEvent::new(code, modifiers)))
}
#[test]
fn transcript_search_edits_unicode_accepts_cancels_and_clears_without_touching_draft() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert!(key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL));
    assert_eq!(
        app.views[&target].transcript.borrow().search.as_deref(),
        Some("")
    );
    for ch in ['c', 'a', 'f', 'é'] {
        assert!(key(&mut app, KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert_eq!(
        app.views[&target].transcript.borrow().search.as_deref(),
        Some("café")
    );
    key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(
        app.views[&target].transcript.borrow().search.as_deref(),
        Some("caf")
    );
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    {
        let state = app.views[&target].transcript.borrow();
        assert_eq!(state.search.as_deref(), Some("caf"));
        assert_eq!(state.query, "caf");
        assert!(state.search_next);
    }
    key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.views[&target].transcript.borrow().query.is_empty());
    key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.views[&target].transcript.borrow().query.is_empty());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(app.views[&target].pending.is_none());
}
#[test]
fn detail_toggle_and_follow_end_reset_navigation_but_panels_block_transcript_keys() {
    let (_fixture, mut app, target) = coverage_support::app();
    let initial = app.views[&target].transcript.borrow().details;
    key(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert_ne!(app.views[&target].transcript.borrow().details, initial);
    key(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert_eq!(app.views[&target].transcript.borrow().details, initial);
    {
        let mut state = app.views[&target].transcript.borrow_mut();
        state.new_output = true;
        state.search_error = true;
        state.anchor = Some(super::super::Anchor {
            key: super::super::Key::Tool("synthetic-call".into()),
            offset: 2,
        });
    }
    assert!(key(&mut app, KeyCode::End, KeyModifiers::CONTROL));
    {
        let state = app.views[&target].transcript.borrow();
        assert!(state.anchor.is_none());
        assert!(!state.new_output);
        assert!(!state.search_error);
    }
    app.views.get_mut(&target).unwrap().panel = Some("content".into());
    assert!(!key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL));
    assert!(app.views[&target].transcript.borrow().search.is_none());
}
