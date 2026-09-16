use super::*;
use crate::process_client::ui::socket_support_tests::{Server, voyage};
use serde_json::json;
use voyage_protocol::vessel::VesselCommand;

#[tokio::test]
async fn paginated_history_rehydrates_unicode_projection_and_closes_revision_fence() {
    let mut server = Server::new(|command| {
        let VesselCommand::Voyage(request) = command else { panic!("not a voyage") };
        let id = request.session_id;
        let result = match request.command {
            VoyageCommand::History { offset: 0, limit: 128, expected_revision: Some(17) } => json!({"session_id":id,"revision":17,"message_offset":0,"messages":[{"message_index":0,"role":"assistant","content":"preview","projection_truncated":true}]}),
            VoyageCommand::History { offset: 1, limit: 128, expected_revision: Some(17) } => json!({"session_id":id,"revision":17,"message_offset":1,"messages":[{"message_index":1,"role":"user","content":"next"}]}),
            VoyageCommand::History { offset: 2, limit: 1, expected_revision: Some(17) } => json!({}),
            VoyageCommand::MessageChunk { index: 0, offset, expected_revision: 17, .. } => {
                let encoded = serde_json::to_string(&json!({"message_index":999,"role":"assistant","content":"complete 界 answer"})).unwrap();
                let split = encoded.find("answer").unwrap();
                let (data, more) = if offset == 0 { (&encoded[..split], true) } else { assert_eq!(offset as usize, split); (&encoded[split..], false) };
                json!({"session_id":id,"revision":17,"index":0,"offset":offset,"data":data,"next_offset":offset + data.len() as u64,"has_more":more})
            }
            _ => panic!("unexpected history request"),
        };
        Ok(voyage(command, result))
    }).await;
    let messages = load(&server.client, server.target, server.incarnation, 17, 0, 2)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "complete 界 answer");
    assert_eq!(messages[0].message_index, 0);
    assert!(!messages[0].projection_truncated);
    assert_eq!(messages[1].content, "next");
    let mut count = 0;
    while server.requests.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(count, 5);
}

#[tokio::test]
async fn malformed_history_and_chunk_cursors_fail_without_returning_partial_messages() {
    for case in 0..9 {
        let server = Server::new(move |command| {
            let VesselCommand::Voyage(request) = command else { unreachable!() };
            let id = request.session_id;
            let result = match request.command {
                VoyageCommand::History { .. } => {
                    let mut page = json!({"session_id":id,"revision":17,"message_offset":0,"messages":[{"message_index":0,"role":"assistant","content":"preview","projection_truncated":case >= 4}]});
                    match case {
                        0 => page["revision"] = json!(18),
                        1 => page["messages"] = json!([]),
                        2 => page["messages"][0]["message_index"] = json!(2),
                        3 => page["message_offset"] = json!(1),
                        _ => {}
                    }
                    page
                }
                VoyageCommand::MessageChunk { .. } => {
                    let mut chunk = json!({"session_id":id,"revision":17,"index":0,"offset":0,"data":"","next_offset":0,"has_more":true});
                    match case {
                        4 => chunk["session_id"] = json!(Uuid::new_v4()),
                        5 => chunk["data"] = json!(null),
                        6 => chunk["next_offset"] = json!(2),
                        7 => {},
                        8 => { chunk["has_more"] = json!(false); },
                        _ => unreachable!()
                    }
                    chunk
                }
                _ => unreachable!()
            };
            Ok(voyage(command, result))
        }).await;
        assert!(
            load(&server.client, server.target, server.incarnation, 17, 0, 1)
                .await
                .is_err(),
            "case {case}"
        );
    }
}

#[tokio::test]
async fn empty_history_still_checks_revision_and_large_windows_never_contact_vessel() {
    let mut server = Server::new(|command| {
        let VesselCommand::Voyage(request) = command else {
            unreachable!()
        };
        assert!(matches!(
            request.command,
            VoyageCommand::History {
                offset: 7,
                limit: 1,
                expected_revision: Some(17)
            }
        ));
        Err("revision changed".into())
    })
    .await;
    assert!(
        load(
            &server.client,
            server.target,
            server.incarnation,
            17,
            0,
            16385
        )
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("too large")
    );
    assert!(server.requests.try_recv().is_err());
    assert!(
        load(&server.client, server.target, server.incarnation, 17, 7, 7)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("revision changed")
    );
    assert!(server.requests.try_recv().is_ok());
}

#[tokio::test]
async fn live_output_uses_byte_offsets_and_rejects_changed_or_nonadvancing_chunks() {
    for case in 0..6 {
        let run = Uuid::new_v4();
        let server = Server::new(move |command| {
            let VesselCommand::Voyage(request) = command else { unreachable!() };
            let VoyageCommand::RunOutput { run_id, offset, limit } = request.command else { unreachable!() };
            assert_eq!(run_id, run);
            assert_eq!(limit as u64, 9 - offset);
            let data = if offset == 3 { "界" } else { "abc" };
            let mut chunk = json!({"session_id":request.session_id,"run_id":run,"offset":offset,"data":data,"next_offset":offset+3});
            match case {
                1 => chunk["run_id"] = json!(Uuid::new_v4()),
                2 => chunk["data"] = json!(null),
                3 => chunk["data"] = json!(""),
                4 => chunk["next_offset"] = json!(99),
                5 => chunk["data"] = json!("too much output"),
                _ => {}
            }
            Ok(voyage(command, chunk))
        }).await;
        let result = live(
            &server.client,
            server.target,
            server.incarnation,
            (run, 3, 9),
        )
        .await;
        if case == 0 {
            assert_eq!(result.unwrap(), "界abc");
        } else {
            assert!(result.is_err(), "case {case}");
        }
        assert!(
            live(
                &server.client,
                server.target,
                server.incarnation,
                (run, 9, 3)
            )
            .await
            .is_err()
        );
        assert!(
            live(
                &server.client,
                server.target,
                server.incarnation,
                (run, 0, 1024 * 1024 + 1)
            )
            .await
            .is_err()
        );
        assert_eq!(
            live(
                &server.client,
                server.target,
                server.incarnation,
                (run, 3, 3)
            )
            .await
            .unwrap(),
            ""
        );
    }
}

#[tokio::test]
async fn app_history_hydration_schedules_once_applies_result_and_ignores_absent_selection() {
    use crate::process_client::ui::{coverage_support, observe::Update};
    let (_fixture, mut app, old) = coverage_support::app();
    let server=Server::new(|command| {
        let VesselCommand::Voyage(request)=command else { unreachable!() };
        Ok(voyage(command,match request.command {
            VoyageCommand::History { offset:0,limit:128,.. }=>json!({"session_id":request.session_id,"revision":17,"message_offset":0,"messages":[{"message_index":0,"role":"assistant","content":"whole history"}]}),
            VoyageCommand::History { offset:1,limit:1,.. }=>json!({}),
            _=>unreachable!(),
        }))
    }).await;
    let route = app.clients.insert(server.client.clone());
    let target = Target {
        route,
        session: server.target.session,
    };
    let mut view = app.views.remove(&old).unwrap();
    view.process.session_id = target.session;
    view.process.incarnation = server.incarnation;
    let snapshot = view.snapshot.as_mut().unwrap();
    snapshot.session_id = target.session;
    snapshot.message_offset = 1;
    snapshot.total_messages = 1;
    view.transcript.borrow_mut().requested_from = Some(0);
    app.views.insert(target, view);
    app.selected = Some(target);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    app.sender = sender;
    app.refresh_transcript();
    assert!(app.views[&target].transcript.borrow().loading);
    app.refresh_transcript();
    let update = tokio::time::timeout(std::time::Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(update, Update::History { .. }));
    app.update(update);
    let state = app.views[&target].transcript.borrow();
    assert!(!state.loading);
    assert_eq!(state.loaded_revision, Some(17));
    assert_eq!(state.messages[0].content, "whole history");
    drop(state);
    app.refresh_transcript();
    assert!(!app.views[&target].transcript.borrow().loading);
    assert!(receiver.try_recv().is_err());
    app.selected = None;
    app.refresh_transcript();
    app.selected = Some(old);
    app.refresh_transcript();
    app.selected = Some(target);
    app.views.get_mut(&target).unwrap().snapshot = None;
    app.refresh_transcript();
}
