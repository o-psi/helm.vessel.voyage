use super::super::coverage_support;
use super::*;
use serde_json::json;
use uuid::Uuid;
#[test]
fn history_results_are_revision_and_incarnation_fenced_and_retained_separately() {
    let (_fixture, mut app, target) = coverage_support::app();
    let incarnation = app.views[&target].process.incarnation;
    let message = |index, content| {
        serde_json::from_value(json!({"message_index":index,"role":"assistant","content":content}))
            .unwrap()
    };
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .messages = vec![message(2, "current")];
    app.views[&target].transcript.borrow_mut().loading = true;
    app.views[&target].transcript.borrow_mut().attempted = Some(16);
    app.apply_update(Update::History {
        target,
        incarnation: Uuid::new_v4(),
        revision: 17,
        result: Ok(vec![message(1, "stale")]),
    });
    assert!(app.views[&target].transcript.borrow().loading);
    app.apply_update(Update::History {
        target,
        incarnation,
        revision: 16,
        result: Ok(vec![message(1, "old revision")]),
    });
    assert_eq!(
        app.views[&target].snapshot.as_ref().unwrap().messages.len(),
        1
    );
    assert!(!app.views[&target].transcript.borrow().loading);
    app.apply_update(Update::History {
        target,
        incarnation,
        revision: 17,
        result: Ok(vec![message(0, "earlier"), message(2, "duplicate")]),
    });
    let snapshot = app.views[&target].snapshot.as_ref().unwrap();
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].content, "current");
    assert_eq!(
        app.views[&target]
            .transcript
            .borrow()
            .messages
            .iter()
            .map(|m| m.message_index)
            .collect::<Vec<_>>(),
        [0, 2]
    );
    app.views[&target].transcript.borrow_mut().loaded_revision = None;
    assert!(app.views[&target].transcript.borrow().dirty);
    app.apply_update(Update::History {
        target,
        incarnation,
        revision: 17,
        result: Err("synthetic history failure".into()),
    });
    assert_eq!(
        app.views[&target].transcript.borrow().error.as_deref(),
        Some("synthetic history failure")
    );
}
#[test]
fn live_results_require_exact_run_and_byte_window() {
    let (_fixture, mut app, target) = coverage_support::app();
    let incarnation = app.views[&target].process.incarnation;
    let run = Uuid::new_v4();
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .run = Some(
        serde_json::from_value(
            json!({"run_id":run,"state":"running","live_text_offset":10,"partial_text_bytes":15}),
        )
        .unwrap(),
    );
    for (candidate, offset, total) in [(Uuid::new_v4(), 10, 15), (run, 9, 15), (run, 10, 16)] {
        app.apply_update(Update::Live {
            target,
            incarnation,
            run: candidate,
            offset,
            total,
            result: Ok("stale".into()),
        });
        assert!(app.views[&target].transcript.borrow().live.is_none());
    }
    app.apply_update(Update::Live {
        target,
        incarnation,
        run,
        offset: 10,
        total: 15,
        result: Ok("hello".into()),
    });
    assert_eq!(
        app.views[&target].transcript.borrow().live,
        Some((run, 10, 15, "hello".into()))
    );
    app.apply_update(Update::Live {
        target,
        incarnation,
        run,
        offset: 10,
        total: 15,
        result: Err("synthetic live failure".into()),
    });
    assert_eq!(
        app.views[&target].transcript.borrow().error.as_deref(),
        Some("synthetic live failure")
    );
}
#[test]
fn control_panels_reset_scroll_but_reject_restarted_process_results() {
    let (_fixture, mut app, target) = coverage_support::app();
    let incarnation = app.views[&target].process.incarnation;
    app.views.get_mut(&target).unwrap().panel = Some("old".into());
    app.views.get_mut(&target).unwrap().scroll = 9;
    app.apply_update(Update::Control {
        target,
        incarnation: Uuid::new_v4(),
        result: Ok("stale".into()),
    });
    assert_eq!(app.views[&target].panel.as_ref().unwrap(), "old");
    app.apply_update(Update::Control {
        target,
        incarnation,
        result: Ok("new tasks".into()),
    });
    assert_eq!(app.views[&target].panel, Some("new tasks".into()));
    assert_eq!(app.views[&target].scroll, 0);
    app.apply_update(Update::Control {
        target,
        incarnation,
        result: Err("synthetic controls failure".into()),
    });
    assert_eq!(
        app.views[&target].error.as_ref().unwrap(),
        "synthetic controls failure"
    );
    app.views.get_mut(&target).unwrap().panel = None;
    app.apply_update(Update::Control {
        target,
        incarnation,
        result: Ok("unrequested".into()),
    });
    assert_eq!(app.views[&target].panel.as_deref(), Some("unrequested"));
}
#[test]
fn snapshot_failures_and_stale_revisions_leave_latest_observation_intact() {
    let (_fixture, mut app, target) = coverage_support::app();
    let incarnation = app.views[&target].process.incarnation;
    let snapshot = |revision| {
        serde_json::from_value(json!({"session_id":target.session,"revision":revision,"messages":[],"model":"synthetic-model"})).unwrap()
    };
    app.apply_update(Update::Snapshot {
        target,
        incarnation: Uuid::new_v4(),
        result: Box::new(Err("stale".into())),
    });
    assert!(app.views[&target].error.is_none());
    app.apply_update(Update::Snapshot {
        target,
        incarnation,
        result: Box::new(Ok(snapshot(16))),
    });
    assert_eq!(app.views[&target].snapshot.as_ref().unwrap().revision, 17);
    app.apply_update(Update::Snapshot {
        target,
        incarnation,
        result: Box::new(Err("offline".into())),
    });
    assert_eq!(app.views[&target].error.as_deref(), Some("offline"));
    assert_eq!(app.views[&target].snapshot.as_ref().unwrap().revision, 17);
    app.apply_update(Update::Snapshot {
        target,
        incarnation,
        result: Box::new(Ok(snapshot(18))),
    });
    assert!(app.views[&target].error.is_none());
    assert_eq!(app.views[&target].snapshot.as_ref().unwrap().revision, 18);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}
