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

#[tokio::test]
async fn receipt_matrix_distinguishes_unknown_mismatched_rejected_and_admitted() {
    for (status, matches_id, retained) in [
        ("unknown", true, true),
        ("completed", false, true),
        ("", true, true),
        ("rejected", true, false),
        ("not_admitted", true, false),
        ("completed", true, false),
        ("transferred", true, false),
    ] {
        let (_fixture, mut app, target) = coverage_support::app();
        let command_id = Uuid::new_v4();
        let incarnation = app.views[&target].process.incarnation;
        app.views.get_mut(&target).unwrap().pending = Some(super::super::state::Pending {
            account_host: None,
            command_id,
            incarnation,
            draft: "preserved draft".into(),
            preserve_draft: false,
            original: None,
            receipt_only: false,
        });
        app.update(Update::Command {
            target, command_id, refused: false,
            result: Ok(json!({"command_id":if matches_id {command_id} else {Uuid::new_v4()},"status":status,"original_status":"rejected"})),
        });
        assert_eq!(app.views[&target].pending.is_some(), retained, "{status}");
        let admitted = status == "completed" && matches_id;
        assert_eq!(
            app.views[&target].draft.text,
            if admitted { "" } else { "preserved draft" },
            "{status}"
        );
        assert!(!app.status.is_empty());
        // Updates schedule a bounded check first. Reconciliation, not receipt
        // handling, prunes entries whose pending command has been resolved.
        assert!(app.command_checks[&(target, command_id)].is_some());
        app.reconcile_pending();
        assert_eq!(
            app.command_checks.contains_key(&(target, command_id)),
            retained
        );
        for task in app.route_tasks.values().flatten() {
            task.abort();
        }
    }
}

#[test]
fn rejected_receipt_restores_empty_draft_but_never_overwrites_new_text() {
    for draft in ["", "/receipt", "new unsent text"] {
        let (_fixture, mut app, target) = coverage_support::app();
        let command_id = Uuid::new_v4();
        let incarnation = app.views[&target].process.incarnation;
        let view = app.views.get_mut(&target).unwrap();
        view.draft.set_text(draft.into());
        view.pending = Some(super::super::state::Pending {
            account_host: None,
            command_id,
            incarnation,
            draft: "original prompt".into(),
            preserve_draft: false,
            original: None,
            receipt_only: false,
        });
        app.update(Update::Command {
            target,
            command_id,
            refused: false,
            result: Ok(json!({"command_id":command_id,"status":"rejected"})),
        });
        assert_eq!(
            app.views[&target].draft.text,
            if draft == "new unsent text" {
                draft
            } else {
                "original prompt"
            }
        );
        assert!(app.views[&target].pending.is_none());
    }
}

#[test]
fn transport_refusal_and_uncertainty_have_different_pending_lifetimes() {
    for refused in [false, true] {
        let (_fixture, mut app, target) = coverage_support::app();
        let command_id = Uuid::new_v4();
        app.views.get_mut(&target).unwrap().pending = Some(super::super::state::Pending {
            account_host: None,
            command_id,
            incarnation: app.views[&target].process.incarnation,
            draft: "preserved draft".into(),
            preserve_draft: true,
            original: None,
            receipt_only: false,
        });
        app.status = "foreground status".into();
        app.selected = None;
        app.update(Update::Command {
            target,
            command_id,
            refused,
            result: Err("synthetic unavailable".into()),
        });
        assert_eq!(app.views[&target].pending.is_some(), !refused);
        assert_eq!(app.status, "foreground status");
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
}

#[test]
fn overview_control_results_are_incarnation_fenced_and_clear_scroll() {
    let (_fixture, mut app, target) = coverage_support::app();
    let incarnation = app.views[&target].process.incarnation;
    app.views.get_mut(&target).unwrap().scroll = 100;
    app.update(Update::Control {
        target,
        incarnation: Uuid::new_v4(),
        result: Ok("stale".into()),
    });
    assert!(app.views[&target].panel.is_none());
    app.update(Update::Control {
        target,
        incarnation,
        result: Ok("canonical overview".into()),
    });
    assert_eq!(
        app.views[&target].panel.as_deref(),
        Some("canonical overview")
    );
    assert_eq!(app.views[&target].scroll, 0);
    app.update(Update::Control {
        target,
        incarnation,
        result: Err("synthetic failure".into()),
    });
    assert_eq!(
        app.views[&target].error.as_deref(),
        Some("synthetic failure")
    );
    assert!(app.status.contains("Couldn't open"));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[test]
fn catalogue_owner_changes_reset_revision_bound_state_but_preserve_pending_drafts() {
    let (_fixture, mut app, target) = coverage_support::app();
    let original = app.views[&target].process.clone();
    app.views.get_mut(&target).unwrap().panel = Some("old owner panel".into());
    app.views[&target].transcript.borrow_mut().loaded_revision = Some(17);
    let mut restarted = original.clone();
    restarted.incarnation = Uuid::new_v4();
    app.update(Update::Catalogue {
        route: target.route,
        processes: vec![restarted.clone()],
    });
    assert_eq!(
        app.views[&target].process.incarnation,
        restarted.incarnation
    );
    assert!(app.views[&target].snapshot.is_none());
    assert_eq!(app.views[&target].panel.as_deref(), Some("old owner panel"));
    assert_eq!(app.views[&target].transcript.borrow().loaded_revision, None);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    app.update(Update::Snapshot {
        target,
        incarnation: original.incarnation,
        result: Box::new(Ok(serde_json::from_value(
            json!({"session_id":target.session,"model":"synthetic-model","revision":999,"messages":[]}),
        )
        .unwrap())),
    });
    assert!(app.views[&target].snapshot.is_none());
    app.update(Update::Catalogue {
        route: target.route,
        processes: vec![],
    });
    assert_eq!(
        app.views[&target].process.incarnation,
        restarted.incarnation
    );
    app.update(Update::Catalogue {
        route: target.route,
        processes: vec![restarted],
    });
    assert!(
        app.views[&target]
            .error
            .as_ref()
            .unwrap()
            .contains("reconnected")
    );
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[test]
fn snapshot_history_shrink_resets_loaded_history_and_mismatched_session_is_explicit() {
    let (_fixture, mut app, target) = coverage_support::app();
    let incarnation = app.views[&target].process.incarnation;
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .total_messages = 100;
    {
        let mut state = app.views[&target].transcript.borrow_mut();
        state.loaded_revision = Some(17);
        state.messages = vec![
            serde_json::from_value(
                json!({"message_index":50,"role":"assistant","content":"removed"}),
            )
            .unwrap(),
        ];
    }
    app.status = format!("{} unavailable: synthetic", app.route_label(target.route));
    app.update(Update::Snapshot {
        target,
        incarnation,
        result: Box::new(Ok(serde_json::from_value(
            json!({"session_id":target.session,"model":"synthetic-model","revision":18,"total_messages":0,"messages":[]}),
        )
        .unwrap())),
    });
    assert!(app.views[&target].transcript.borrow().messages.is_empty());
    assert_eq!(app.views[&target].transcript.borrow().loaded_revision, None);
    assert_eq!(app.status, "Connected. Voyage state refreshed.");
    app.update(Update::Snapshot {
        target,
        incarnation,
        result: Box::new(Ok(serde_json::from_value(
            json!({"session_id":Uuid::new_v4(),"model":"synthetic-model","revision":19,"messages":[]}),
        )
        .unwrap())),
    });
    assert_eq!(app.views[&target].snapshot.as_ref().unwrap().revision, 18);
    assert_eq!(
        app.views[&target].error.as_deref(),
        Some("Snapshot identity mismatch")
    );
}

#[test]
fn route_updates_fence_retired_generations_and_recovery_preserves_draft() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.update(Update::RouteUnavailable {
        route: target.route,
        error: "synthetic timeout".into(),
    });
    assert!(!app.clients.available(target.route));
    assert!(app.views[&target].connection_unavailable);
    assert!(
        app.views[&target]
            .error
            .as_ref()
            .unwrap()
            .contains("unknown")
    );
    let process = app.views[&target].process.clone();
    app.update(Update::Catalogue {
        route: target.route,
        processes: vec![process],
    });
    assert!(app.clients.available(target.route));
    assert!(!app.views[&target].connection_unavailable);
    assert!(app.views[&target].error.is_some());
    let client = app.clients[target.route].clone();
    let replacement = app.clients.insert(client);
    app.status = "new route foreground".into();
    app.update(Update::RouteError {
        route: target.route,
        error: "stale".into(),
    });
    assert_eq!(app.status, "new route foreground");
    assert!(app.clients.current(replacement));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}
