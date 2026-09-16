use super::super::coverage_support;
use super::*;
use crossterm::event::KeyEvent;
use serde_json::json;

fn entry() -> Entry {
    serde_json::from_value(json!({"scope":"repository","digest":"a".repeat(64),"document":{"schema_version":1,"id":"review-code","version":"1","description":"Review","prompt":"Review {{count}} using {{token}}","parameters":{"count":{"type":"integer","required":true,"minimum":1,"maximum":3,"default":2},"token":{"type":"string","secret":true,"required":true,"max_length":32}}}})).unwrap()
}
fn panel(app: &App, target: Target) -> Panel {
    Panel {
        target,
        incarnation: app.views[&target].process.incarnation,
        host: "synthetic host".into(),
        title: "Test voyage".into(),
        entries: vec![entry()],
        selected: 0,
        phase: Phase::Inventory,
        fields: vec![],
        field: 0,
        public: BTreeMap::new(),
        private: BTreeMap::new(),
        editor: Zeroizing::new(String::new()),
        preview: String::new(),
        revision: 17,
        notice: String::new(),
        scroll: Default::default(),
        scroll_max: Default::default(),
        rendered: Default::default(),
        deadline: Instant::now() + Duration::from_secs(300),
        request: None,
    }
}
fn key(app: &mut App, code: KeyCode) {
    assert!(
        app.workflow_input(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap()
    );
}

#[test]
fn workflow_full_input_journey_validates_then_isolates_private_text_on_focus_loss() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert!(!app.workflows_open());
    assert!(!app.workflow_input(&Event::FocusGained).unwrap());
    app.workflows.panel = Some(panel(&app, target));
    for code in [KeyCode::Down, KeyCode::Up, KeyCode::Enter] {
        key(&mut app, code);
    }
    assert!(app.workflows.panel.as_ref().unwrap().phase == Phase::Trust);
    app.workflows.panel.as_ref().unwrap().rendered.set(true);
    key(&mut app, KeyCode::Char('t'));
    assert!(app.workflows.panel.as_ref().unwrap().phase == Phase::Inputs);
    assert_eq!(app.workflows.panel.as_ref().unwrap().editor.as_str(), "2");
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Char('9'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.workflows.panel.as_ref().unwrap().field, 0);
    assert!(!app.workflows.panel.as_ref().unwrap().notice.is_empty());
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Char('3'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.workflows.panel.as_ref().unwrap().field, 1);
    assert_eq!(
        app.workflows
            .panel
            .as_ref()
            .unwrap()
            .public
            .get("count")
            .map(String::as_str),
        Some("3")
    );
    app.workflow_input(&Event::Paste("private fixture".into()))
        .unwrap();
    assert_eq!(
        app.workflows.panel.as_ref().unwrap().editor.as_str(),
        "private fixture"
    );
    app.workflow_input(&Event::Paste("\0".into())).unwrap();
    assert!(app.workflows.panel.as_ref().unwrap().notice.contains("NUL"));
    app.workflow_input(&Event::FocusLost).unwrap();
    let panel = app.workflows.panel.as_ref().unwrap();
    assert!(panel.phase == Phase::Invalidated);
    assert!(panel.private.is_empty());
    assert!(panel.editor.is_empty());
    app.workflow_input(&Event::Paste("late clipboard".into()))
        .unwrap();
    key(&mut app, KeyCode::Char('x'));
    assert!(app.workflows.panel.as_ref().unwrap().editor.is_empty());
    key(&mut app, KeyCode::Esc);
    assert!(!app.workflows_open());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(app.views[&target].pending.is_none());
}

#[test]
fn workflow_poll_invalidates_stale_targets_but_retains_inventory_on_observation_errors() {
    for case in 0..5 {
        let (_fixture, mut app, target) = coverage_support::app();
        app.workflows.panel = Some(panel(&app, target));
        match case {
            0 => app.selected = None,
            1 => {
                app.workflows.panel.as_mut().unwrap().deadline =
                    Instant::now() - Duration::from_secs(1)
            }
            2 => app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4(),
            3 => {
                let (sender, receiver) = oneshot::channel();
                drop(sender);
                app.workflows.panel.as_mut().unwrap().request = Some(receiver);
            }
            _ => {
                let (sender, receiver) = oneshot::channel();
                sender.send(Ok(json!({"entries":"invalid"}))).unwrap();
                app.workflows.panel.as_mut().unwrap().request = Some(receiver);
            }
        }
        app.poll_workflows();
        let panel = app.workflows.panel.as_ref().unwrap();
        if case < 3 {
            assert!(panel.phase == Phase::Invalidated, "case {case}");
        } else {
            assert!(panel.phase == Phase::Inventory, "case {case}");
            assert!(panel.request.is_none());
            assert_eq!(panel.entries.len(), 1);
            assert_eq!(panel.revision, 17);
            assert_eq!(
                panel.notice,
                if case == 3 {
                    "Workflow observation interrupted; close and reload"
                } else {
                    "Unexpected host inventory response"
                }
            );
        }
        assert!(!panel.notice.is_empty());
        assert!(panel.private.is_empty());
    }
}

#[test]
fn workflow_inventory_response_is_consumed_once_and_waiting_inputs_are_quarantined() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.workflows.panel = Some(panel(&app, target));
    let (sender, receiver) = oneshot::channel();
    app.workflows.panel.as_mut().unwrap().request = Some(receiver);
    key(&mut app, KeyCode::Enter);
    assert!(app.workflows.panel.as_ref().unwrap().phase == Phase::Inventory);
    sender
        .send(Ok(json!({"section":"workflows","value":[]})))
        .unwrap();
    app.poll_workflows();
    let panel = app.workflows.panel.as_mut().unwrap();
    assert!(panel.request.is_none());
    assert!(panel.entries.is_empty());
    assert_eq!(panel.revision, 17);
    assert!(panel.notice.contains("No saved workflows"));
    panel.scroll_max.set(24);
    panel.rendered.set(true);
    key(&mut app, KeyCode::PageDown);
    assert_eq!(app.workflows.panel.as_ref().unwrap().scroll.get(), 8);
    key(&mut app, KeyCode::PageUp);
    assert_eq!(app.workflows.panel.as_ref().unwrap().scroll.get(), 0);
    app.workflow_input(&Event::Resize(80, 24)).unwrap();
    assert!(!app.workflows.panel.as_ref().unwrap().rendered.get());
    assert!(
        app.workflow_input(&Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL
        )))
        .unwrap()
    );
    assert!(app.quit);
    assert!(!app.workflows_open());
}

#[tokio::test]
async fn workflow_open_checks_selected_voyage_and_snapshot_before_reading() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.selected = None;
    assert!(
        app.open_workflows()
            .unwrap_err()
            .to_string()
            .contains("Select an existing voyage")
    );
    app.selected = Some(target);
    let snapshot = app.views.get_mut(&target).unwrap().snapshot.take();
    assert!(
        app.open_workflows()
            .unwrap_err()
            .to_string()
            .contains("Waiting for voyage snapshot")
    );
    app.views.get_mut(&target).unwrap().snapshot = snapshot;
    app.open_workflows().unwrap();
    assert!(app.workflows_open());
    assert!(app.workflows.panel.as_ref().unwrap().request.is_some());
    assert_eq!(app.workflows.panel.as_ref().unwrap().target, target);
    key(&mut app, KeyCode::Esc);
    assert!(app.views[&target].pending.is_none());
}
