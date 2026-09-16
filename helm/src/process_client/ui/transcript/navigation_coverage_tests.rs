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

#[cfg(unix)]
#[tokio::test]
async fn sender_lookup_reads_exact_identity_snapshot_and_history_over_public_socket() {
    use crate::process_client::ui::socket_support_tests::{Server, voyage};
    let source = source();
    let expected = source.clone();
    let incarnation = Uuid::new_v4();
    let mut server = Server::new(move |command| {
        let value = match command {
            VesselCommand::Capabilities => json!({"vessel_id":expected.vessel_id}),
            VesselCommand::Inspect { session_id } => {
                assert_eq!(*session_id, expected.session_id);
                json!({"session_id":expected.session_id,"incarnation":incarnation,"workspace":"/synthetic-workspace","state":"live"})
            }
            VesselCommand::Voyage(request) => {
                let value = match request.command {
                    VoyageCommand::Snapshot => json!({"session_id":expected.session_id,"model":"synthetic-model","revision":17,"message_offset":0,"total_messages":1,"messages":[]}),
                    VoyageCommand::History { offset:0, .. } => json!({"session_id":expected.session_id,"revision":17,"message_offset":0,"messages":[{"message_index":0,"role":"assistant","content":"sent","tool_calls":[{"id":expected.tool_call_id,"name":"vessel","arguments":{"action":"submit","command_id":expected.command_id}}]}]}),
                    VoyageCommand::History { offset:1, limit:1, .. } => json!({}),
                    _ => panic!("navigation must only read snapshot and history"),
                };
                voyage(command, value)
            }
            _ => panic!("navigation must not mutate"),
        };
        Ok(value)
    }).await;
    let located = locate(
        vec![(server.target.route, server.client.clone())],
        source.clone(),
        server.target,
    )
    .await
    .unwrap();
    assert_eq!(located.target.session, source.session_id);
    assert_eq!(located.process.incarnation, incarnation);
    assert_eq!(located.call, source.tool_call_id);
    assert_eq!(located.group, 0);
    assert_eq!(located.messages.len(), 1);
    let mut count = 0;
    while server.requests.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(count, 5);
}

#[cfg(unix)]
#[tokio::test]
async fn sender_lookup_rejects_unavailable_ambiguous_and_nonmatching_provenance() {
    use crate::process_client::ui::socket_support_tests::{Server, voyage};
    for case in 0..5 {
        let source = source();
        let expected = source.clone();
        let server = Server::new(move |command| {
            Ok(match command {
                VesselCommand::Capabilities => json!({"vessel_id":if case == 0 { Uuid::new_v4() } else { expected.vessel_id }}),
                VesselCommand::Inspect { .. } => json!({"session_id":expected.session_id,"incarnation":Uuid::new_v4(),"workspace":"/synthetic-workspace","state":"live"}),
                VesselCommand::Voyage(request) => voyage(command, match request.command {
                    VoyageCommand::Snapshot => json!({"session_id":expected.session_id,"model":"synthetic-model","revision":17,"message_offset":0,"total_messages":if case == 1 {16385} else {1},"messages":[]}),
                    VoyageCommand::History { offset:0, .. } => json!({"session_id":expected.session_id,"revision":17,"message_offset":0,"messages":[{"message_index":0,"role":"assistant","content":"sent","tool_calls":[{"id":if case == 2 {"wrong"} else {&expected.tool_call_id},"name":if case == 3 {"shell"} else {"vessel"},"arguments":{"action":"submit","command_id":Uuid::new_v4()}}]}]}),
                    VoyageCommand::History { .. } => json!({}),
                    _ => unreachable!(),
                }),
                _ => unreachable!(),
            })
        }).await;
        let result = locate(
            vec![(server.target.route, server.client.clone())],
            source,
            server.target,
        )
        .await;
        let error = match result {
            Ok(_) => panic!("case {case} unexpectedly located"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains(match case {
                0 => "not connected",
                1 => "reading limit reached",
                2 => "not in the sender",
                3 => "not in the sender",
                _ => "not in the sender",
            }),
            "case {case}: {error}"
        );
    }
    let source = source();
    let vessel = source.vessel_id;
    let server = Server::new(move |_| Ok(json!({"vessel_id":vessel}))).await;
    let mut origin = server.target;
    origin.route.generation += 1;
    let result = locate(
        vec![
            (server.target.route, server.client.clone()),
            (server.target.route, server.client.clone()),
        ],
        source,
        origin,
    )
    .await;
    assert!(match result {
        Err(error) => error.to_string().contains("Multiple connections"),
        Ok(_) => false,
    });
}

fn source() -> CoordinationSource {
    CoordinationSource {
        vessel_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        session_name: "Sender".into(),
        command_id: Uuid::new_v4(),
        tool_call_id: "send-call".into(),
    }
}
