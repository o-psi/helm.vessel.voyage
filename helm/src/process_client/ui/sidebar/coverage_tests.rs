use super::super::coverage_support;
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn draw(app: &App, width: u16, height: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
    term.draw(|f| app.draw_actions(f, Rect::new(0, 0, width, height)))
        .unwrap();
    term.backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
#[test]
fn action_list_and_editors_render_scope_and_confirmation_language() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.open_actions(target);
    let text = draw(&app, 110, 45);
    for expected in [
        "Rename",
        "Archive",
        "Branch",
        "Access",
        "Details",
        "Compact older messages",
        "Clear conversation",
        "Delete permanently",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    assert!(app.sidebar.visible.get().is_some());
    assert!(!app.sidebar.menu_hits.borrow().is_empty());
    for (action, expected) in [
        (Action::Details, "Workspace: /synthetic-workspace"),
        (Action::Rename, "Enter a new name"),
        (Action::Branch, "Reopen branch review"),
        (Action::Clear, "Type CLEAR to confirm"),
        (Action::Delete, "Type DELETE to confirm"),
        (Action::Compact, "Reduce older working context"),
    ] {
        let menu = app.sidebar.menu.as_mut().unwrap();
        menu.editor = Some(action);
        menu.scroll.set(0);
        menu.follow_selection.set(true);
        let text = draw(&app, 110, 45);
        assert!(text.contains(expected), "{text}");
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
    for action in [
        Action::Access,
        Action::ReadOnly,
        Action::Approval,
        Action::Unrestricted,
    ] {
        app.sidebar.menu.as_mut().unwrap().editor = Some(action);
        let text = draw(&app, 110, 45);
        assert!(text.contains("access"), "{text}");
        assert!(app.views[&target].pending.is_none());
    }
    let text = draw(&app, 18, 8);
    assert!(text.contains("Enlarge"));
    assert!(app.sidebar.visible.get().is_none());
}
#[test]
fn action_availability_rechecks_incarnation_run_cleanup_and_lifecycle() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.open_actions(target);
    assert!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Rename)
            .is_none()
    );
    let original = app.views[&target].process.incarnation;
    app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4();
    assert_eq!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Details),
        Some("Voyage restarted; reopen Actions")
    );
    app.views.get_mut(&target).unwrap().process.incarnation = original;
    app.views.get_mut(&target).unwrap().error = Some("synthetic offline".into());
    assert_eq!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Rename),
        Some("Voyage is unavailable")
    );
    assert!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Details)
            .is_none()
    );
    app.views.get_mut(&target).unwrap().error = None;
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .lifecycle = json!({"deleted":true});
    assert_eq!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Clear),
        Some("History has been deleted")
    );
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .lifecycle = json!({});
    let run = Uuid::new_v4();
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .run = Some(serde_json::from_value(json!({"run_id":run,"state":"running"})).unwrap());
    app.open_actions(target);
    assert!(
        app.sidebar
            .menu
            .as_ref()
            .unwrap()
            .actions
            .contains(&Action::Cancel)
    );
    assert!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Cancel)
            .is_none()
    );
    assert!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Access)
            .is_none()
    );
    assert_eq!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Rename),
        Some("Wait for the current run to finish")
    );
    app.sidebar.menu.as_mut().unwrap().run = None;
    assert_eq!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Cancel),
        Some("The selected run has ended or changed")
    );
    app.views.get_mut(&target).unwrap().snapshot = None;
    assert_eq!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Rename),
        Some("Waiting for voyage state")
    );
    app.views.remove(&target);
    assert_eq!(
        app.action_reason(app.sidebar.menu.as_ref().unwrap(), Action::Details),
        Some("Voyage is no longer available")
    );
}
#[test]
fn menu_errors_sanitize_content_and_compaction_does_not_dispatch() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.review_compact(target, Some("12")).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().text.text, "KEEP 12");
    assert_eq!(app.sidebar.menu.as_ref().unwrap().revision, Some(17));
    app.sidebar.menu.as_mut().unwrap().error = "synthetic error\u{1b}[31m".into();
    let text = draw(&app, 110, 45);
    assert!(text.contains("synthetic error"));
    assert!(!text.contains('\u{1b}'));
    assert!(app.views[&target].pending.is_none());
    for invalid in ["0", "100001", "banana"] {
        assert!(app.review_compact(target, Some(invalid)).is_err());
    }
    app.sidebar_select(target);
    assert!(app.sidebar.focus == Focus::Voyages);
    app.sidebar_composer();
    assert!(app.sidebar.focus == Focus::Composer);
}

#[test]
fn access_review_keyboard_requires_visible_confirmation_and_never_changes_mode_on_navigation() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let (_fixture, mut app, target) = coverage_support::app();
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .access = Some("approval".into());
    app.open_access(target, None).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().access_selected, 1);
    let text = draw(&app, 110, 45);
    assert!(text.contains("Current access: Ask first"));
    for code in [KeyCode::Down, KeyCode::Up, KeyCode::BackTab, KeyCode::Tab] {
        let mut menu = app.sidebar.menu.take().unwrap();
        app.access_input(
            &mut menu,
            &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        )
        .unwrap();
        app.sidebar.menu = Some(menu);
    }
    assert_eq!(app.sidebar.menu.as_ref().unwrap().access_selected, 1);
    let mut menu = app.sidebar.menu.take().unwrap();
    app.access_input(
        &mut menu,
        &Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    )
    .unwrap();
    assert!(menu.editor == Some(Action::Approval));
    app.sidebar.menu = Some(menu);
    let text = draw(&app, 110, 45);
    assert!(text.contains("Change access to Ask first?"));
    assert!(text.contains("Pending approvals are denied"));
    let mut menu = app.sidebar.menu.take().unwrap();
    app.access_input(
        &mut menu,
        &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    )
    .unwrap();
    assert!(menu.editor == Some(Action::Access));
    app.access_input(
        &mut menu,
        &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    )
    .unwrap();
    assert!(menu.editor.is_none());
    assert!(app.views[&target].pending.is_none());
    assert_eq!(
        app.views[&target]
            .snapshot
            .as_ref()
            .unwrap()
            .access
            .as_deref(),
        Some("approval")
    );
    assert!(app.open_access(target, Some("invalid")).is_err());
}

fn event(code: crossterm::event::KeyCode) -> crossterm::event::Event {
    crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
        code,
        crossterm::event::KeyModifiers::NONE,
    ))
}

#[test]
fn sidebar_keyboard_focus_never_eats_composer_edits_or_modified_navigation() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let (_fixture, mut app, target) = coverage_support::app();
    app.sidebar.focus = Focus::Composer;
    for code in [KeyCode::Up, KeyCode::Down, KeyCode::Right] {
        assert!(!app.sidebar_input(&event(code)).unwrap());
    }
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SHIFT,
    ] {
        assert!(
            !app.sidebar_input(&Event::Key(KeyEvent::new(KeyCode::Left, modifiers)))
                .unwrap()
        );
    }
    app.views.get_mut(&target).unwrap().draft.cursor = 0;
    assert!(app.sidebar_input(&event(KeyCode::Left)).unwrap());
    assert!(app.sidebar.focus == Focus::Voyages);
    assert!(app.sidebar_input(&event(KeyCode::Right)).unwrap());
    assert!(app.sidebar.focus == Focus::Button);
    assert!(app.sidebar_input(&event(KeyCode::Enter)).unwrap());
    assert!(app.sidebar.menu.is_some());
    assert!(app.sidebar_input(&event(KeyCode::Esc)).unwrap());
    assert!(app.sidebar.menu.is_none());
    for code in [KeyCode::Char('a'), KeyCode::Delete, KeyCode::Backspace] {
        app.sidebar.focus = Focus::Voyages;
        assert!(!app.sidebar_input(&event(code)).unwrap());
        assert!(app.sidebar.focus == Focus::Composer);
    }
    app.sidebar.focus = Focus::Voyages;
    assert!(app.sidebar_input(&event(KeyCode::Enter)).unwrap());
    assert!(app.sidebar.focus == Focus::Composer);
    app.help = true;
    assert!(!app.sidebar_input(&event(KeyCode::F(9))).unwrap());
    app.help = false;
    assert!(app.sidebar_input(&event(KeyCode::F(9))).unwrap());
    assert_eq!(app.sidebar.menu.as_ref().unwrap().target, target);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[test]
fn menu_navigation_scroll_resize_and_editor_cancel_preserve_target_identity() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    let (_fixture, mut app, target) = coverage_support::app();
    app.open_actions(target);
    let count = app.sidebar.menu.as_ref().unwrap().actions.len();
    app.sidebar_input(&event(KeyCode::Up)).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().selected, count - 1);
    app.sidebar_input(&event(KeyCode::Tab)).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().selected, 0);
    app.sidebar_input(&event(KeyCode::BackTab)).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().selected, count - 1);
    app.sidebar_input(&event(KeyCode::Down)).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().selected, 0);
    app.sidebar.scroll_bounds.set((4, 20));
    app.sidebar_input(&event(KeyCode::PageDown)).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().scroll.get(), 7);
    assert!(!app.sidebar.menu.as_ref().unwrap().follow_selection.get());
    app.sidebar_input(&event(KeyCode::PageUp)).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().scroll.get(), 1);
    app.sidebar_input(&Event::Resize(100, 30)).unwrap();
    assert!(app.sidebar.menu.as_ref().unwrap().follow_selection.get());
    let mut release = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    app.sidebar_input(&Event::Key(release)).unwrap();
    assert!(app.sidebar.menu.is_some());
    let menu = app.sidebar.menu.as_mut().unwrap();
    menu.editor = Some(Action::Rename);
    menu.text = Default::default();
    for code in [
        KeyCode::Char('界'),
        KeyCode::Char('a'),
        KeyCode::Left,
        KeyCode::Delete,
        KeyCode::Home,
        KeyCode::Right,
        KeyCode::Backspace,
        KeyCode::Char('b'),
        KeyCode::End,
    ] {
        app.sidebar_input(&event(code)).unwrap();
    }
    assert_eq!(app.sidebar.menu.as_ref().unwrap().text.text, "b");
    app.sidebar_input(&event(KeyCode::Esc)).unwrap();
    assert!(app.sidebar.menu.as_ref().unwrap().editor.is_none());
    // Escape leaves the editor draft intact but disarms submission.
    assert_eq!(app.sidebar.menu.as_ref().unwrap().text.text, "b");
    assert!(app.sidebar.visible.get().is_none());
    app.sidebar_input(&event(KeyCode::Left)).unwrap();
    assert!(app.sidebar.menu.is_none());
    assert_eq!(app.selected, Some(target));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[test]
fn destructive_editors_validate_typed_confirmation_and_revision_before_any_dispatch() {
    use crossterm::event::KeyCode;
    let (_fixture, mut app, target) = coverage_support::app();
    for (action, text, expected) in [
        (Action::Rename, "", "Enter a name"),
        (Action::Clear, "clear", "CLEAR"),
        (Action::Compact, "KEEP 0", "1–100000"),
        (Action::Delete, "delete", "DELETE"),
    ] {
        app.open_actions(target);
        let menu = app.sidebar.menu.as_mut().unwrap();
        menu.editor = Some(action);
        menu.revision = Some(17);
        menu.text = Default::default();
        menu.text.insert_str(text);
        draw(&app, 110, 45);
        app.sidebar_input(&event(KeyCode::Enter)).unwrap();
        let menu = app.sidebar.menu.as_ref().unwrap();
        assert!(menu.error.contains(expected), "{}", menu.error);
        assert!(menu.editor == Some(action));
        assert_eq!(menu.text.text, text);
        assert!(app.views[&target].pending.is_none());
    }
    for action in [Action::Clear, Action::Compact, Action::Delete] {
        app.open_actions(target);
        let menu = app.sidebar.menu.as_mut().unwrap();
        menu.editor = Some(action);
        menu.revision = Some(16);
        menu.text.insert_str(match action {
            Action::Clear => "CLEAR",
            Action::Compact => "KEEP 2",
            _ => "DELETE",
        });
        draw(&app, 110, 45);
        app.sidebar_input(&event(KeyCode::Enter)).unwrap();
        assert!(app.sidebar.menu.as_ref().unwrap().error.contains("changed"));
        assert!(app.views[&target].pending.is_none());
    }
}

#[test]
fn sidebar_mouse_hits_require_displayed_menu_identity_and_do_not_reuse_stale_geometry() {
    use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;
    let (_fixture, mut app, target) = coverage_support::app();
    let rect = Rect::new(2, 2, 8, 1);
    let mouse = |kind| {
        Event::Mouse(MouseEvent {
            kind,
            column: 3,
            row: 2,
            modifiers: KeyModifiers::NONE,
        })
    };
    app.sidebar.hits.borrow_mut().push(Hit {
        area: rect,
        button: rect,
        target,
        incarnation: app.views[&target].process.incarnation,
    });
    assert!(
        app.sidebar_input(&mouse(MouseEventKind::Down(MouseButton::Left)))
            .unwrap()
    );
    let menu = app.sidebar.menu.as_ref().unwrap();
    let identity = (menu.target, menu.incarnation, menu.editor);
    app.sidebar.area.set(Rect::new(0, 0, 30, 30));
    app.sidebar.menu_hits.borrow_mut().push((
        rect,
        target,
        app.views[&target].process.incarnation,
        0,
    ));
    app.sidebar.visible.set(None);
    app.sidebar_input(&mouse(MouseEventKind::Moved)).unwrap();
    app.sidebar_input(&mouse(MouseEventKind::Down(MouseButton::Left)))
        .unwrap();
    assert!(app.sidebar.menu.as_ref().unwrap().editor.is_none());
    app.sidebar.visible.set(Some(identity));
    app.sidebar_input(&mouse(MouseEventKind::Moved)).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().selected, 0);
    app.sidebar.scroll_bounds.set((5, 20));
    app.sidebar_input(&mouse(MouseEventKind::ScrollDown))
        .unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().scroll.get(), 8);
    app.sidebar.visible.set(Some(identity));
    app.sidebar_input(&mouse(MouseEventKind::ScrollUp)).unwrap();
    assert_eq!(app.sidebar.menu.as_ref().unwrap().scroll.get(), 2);
    app.sidebar_input(&event(KeyCode::Esc)).unwrap();
}
