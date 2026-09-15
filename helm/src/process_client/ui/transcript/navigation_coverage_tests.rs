use super::super::super::coverage_support;
use super::*;
use serde_json::json;
#[test]
fn sender_navigation_installs_complete_read_only_history_and_focuses_tool() {
    let (_fixture, mut app, origin) = coverage_support::app();
    let request = Uuid::new_v4();
    app.coordination_request = Some(request);
    let target = Target {
        route: origin.route,
        session: Uuid::new_v4(),
    };
    let process=serde_json::from_value(json!({"session_id":target.session,"incarnation":Uuid::new_v4(),"workspace":"/synthetic","state":"suspended","name":"Sender"})).unwrap();
    let snapshot=serde_json::from_value(json!({"session_id":target.session,"revision":42,"model":"synthetic","messages":[],"total_messages":1})).unwrap();
    let message = serde_json::from_value(
        json!({"message_index":0,"role":"assistant","content":"Sender explanation"}),
    )
    .unwrap();
    app.coordination_arrived(
        origin,
        request,
        Ok(Box::new(Located {
            target,
            process,
            snapshot,
            messages: vec![message],
            call: "synthetic-call".into(),
            group: 0,
        })),
    );
    assert!(app.coordination_request.is_none());
    assert_eq!(app.selected, Some(target));
    assert_eq!(app.status, "Opened the sending tool call");
    let view = &app.views[&target];
    assert_eq!(view.snapshot.as_ref().unwrap().revision, 42);
    let state = view.transcript.borrow();
    assert_eq!(state.loaded_revision, Some(42));
    assert_eq!(state.messages[0].content, "Sender explanation");
    assert!(state.tool_expanded.contains("synthetic-call"));
    assert!(
        matches!(state.anchor.as_ref().map(|a| &a.key), Some(super::super::Key::Tool(call)) if call == "synthetic-call")
    );
    assert_eq!(state.expanded.get(&0), Some(&true));
    assert!(state.dirty);
    assert!(!state.loading);
    assert!(view.panel.is_none());
    assert!(!view.terminals.open);
    assert!(view.pending.is_none());
    assert_eq!(app.views[&origin].draft.text, "preserved draft");
}
#[test]
fn stale_navigation_and_user_selection_changes_do_not_steal_focus() {
    let (_fixture, mut app, origin) = coverage_support::app();
    let request = Uuid::new_v4();
    app.coordination_request = Some(request);
    app.coordination_arrived(origin, Uuid::new_v4(), Err("stale failure".into()));
    assert_eq!(app.coordination_request, Some(request));
    assert!(!app.status.contains("stale"));
    app.coordination_arrived(origin, request, Err("synthetic navigation failure".into()));
    assert_eq!(app.status, "synthetic navigation failure");
    assert_eq!(app.selected, Some(origin));
    app.coordination_request = Some(request);
    app.selected = None;
    app.coordination_arrived(origin, request, Err("must not replace status".into()));
    assert!(app.coordination_request.is_none());
    assert_eq!(app.status, "synthetic navigation failure");
}
