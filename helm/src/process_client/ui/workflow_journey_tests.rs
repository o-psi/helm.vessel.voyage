//! Complete ordinary workflow entry, private handoff and exact receipt recovery.
use super::super::{
    coverage_support,
    socket_support_tests::{Server, voyage},
};
use super::*;
use crossterm::event::KeyEvent;
use serde_json::json;
use tokio::sync::mpsc;
use voyage_protocol::vessel::VesselCommand;

const PRIVATE: &str = "synthetic-workflow-private-sentinel";
fn definition() -> Value {
    json!({"scope":"repository","digest":"a".repeat(64),"document":{"schema_version":1,"id":"review-code","version":"1","description":"Review canonical output","prompt":"Review {{count}} results, enabled {{enabled}}, with {{token}}","parameters":{"count":{"type":"integer","required":true,"minimum":1,"maximum":3,"default":2},"enabled":{"type":"boolean","default":true},"token":{"type":"string","secret":true,"required":false,"max_length":64}}}})
}
fn connect(app: &mut App, old: Target, server: &Server) -> Target {
    let target = Target {
        route: app.clients.insert(server.client.clone()),
        session: server.target.session,
    };
    let mut view = app.views.remove(&old).unwrap();
    view.process.session_id = target.session;
    view.process.incarnation = server.incarnation;
    view.snapshot.as_mut().unwrap().session_id = target.session;
    view.observed = Some(Instant::now());
    app.views.insert(target, view);
    app.selected = Some(target);
    target
}
fn key(app: &mut App, code: KeyCode) {
    assert!(
        app.workflow_input(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap()
    );
}
async fn sent(server: &mut Server) -> VoyageCommand {
    let command = tokio::time::timeout(Duration::from_secs(3), server.requests.recv())
        .await
        .unwrap()
        .unwrap();
    let VesselCommand::Voyage(request) = command else {
        panic!("only selected-voyage workflow requests")
    };
    assert_eq!(request.session_id, server.target.session);
    assert_eq!(request.incarnation, Some(server.incarnation));
    request.command
}
async fn observed(app: &mut App) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            app.poll_workflows();
            if app.workflows.panel.as_ref().unwrap().request.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn next(receiver: &mut mpsc::Receiver<Update>) -> Update {
    tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}
fn read_to_end(app: &mut App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 40)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    while app.workflows.panel.as_ref().unwrap().scroll.get()
        < app.workflows.panel.as_ref().unwrap().scroll_max.get()
    {
        key(app, KeyCode::PageDown);
    }
    terminal.draw(|frame| draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
async fn inventory(app: &mut App, server: &mut Server) {
    app.open_workflows().unwrap();
    assert!(
        matches!(sent(server).await,VoyageCommand::Controls{section,..} if section=="workflows")
    );
    observed(app).await;
    assert_eq!(app.workflows.panel.as_ref().unwrap().entries.len(), 1);
    key(app, KeyCode::Enter);
    assert!(app.workflows.panel.as_ref().unwrap().phase == Phase::Trust);
}
async fn preview(app: &mut App, server: &mut Server, private: bool) {
    read_to_end(app);
    key(app, KeyCode::Char('t'));
    assert_eq!(app.workflows.panel.as_ref().unwrap().editor.as_str(), "2");
    key(app, KeyCode::Enter);
    assert_eq!(
        app.workflows.panel.as_ref().unwrap().editor.as_str(),
        "true"
    );
    key(app, KeyCode::Enter);
    if private {
        assert!(app.workflow_input(&Event::Paste(PRIVATE.into())).unwrap());
        let screen = read_to_end(app);
        assert!(screen.contains("[hidden]"));
        assert!(!screen.contains(PRIVATE));
    }
    key(app, KeyCode::Enter);
    let command = sent(server).await;
    let VoyageCommand::WorkflowPreview {
        optional_secret_names,
        id,
        scope,
        user_directory,
        inputs,
        trust_digest,
    } = command
    else {
        panic!("preview")
    };
    assert_eq!(id, "review-code");
    assert_eq!(scope.as_deref(), Some("repository"));
    assert!(user_directory.is_none());
    assert_eq!(trust_digest.as_deref(), Some("a".repeat(64).as_str()));
    assert_eq!(
        inputs,
        vec![
            ("count".to_owned(), "2".to_owned()),
            ("enabled".to_owned(), "true".to_owned())
        ]
    );
    assert_eq!(
        optional_secret_names,
        private.then(|| vec!["token".to_owned()])
    );
    observed(app).await;
}
fn response(command: &VesselCommand) -> Result<Value, String> {
    let VesselCommand::Voyage(request) = command else {
        panic!("workflow only")
    };
    Ok(voyage(
        command,
        match &request.command {
            VoyageCommand::Controls { section, .. } => {
                assert_eq!(section, "workflows");
                json!({"section":"workflows","value":[definition()]})
            }
            VoyageCommand::WorkflowPreview {
                optional_secret_names,
                ..
            } => {
                json!({"definition":definition(),"prompt":"Review two canonical results with private input held on the executing host.","secret_names":optional_secret_names.clone().unwrap_or_default()})
            }
            VoyageCommand::WorkflowInputs { .. } => json!({"stored":true}),
            VoyageCommand::WorkflowSubmit { command_id, .. } => {
                json!({"command_id":command_id,"status":"accepted"})
            }
            _ => panic!("no inference, process launch or unreviewed command"),
        },
    ))
}
fn assert_private_files(root: &std::path::Path) {
    for directory in ["helm-command-receipts", "command-receipts"] {
        let path = root.join(directory);
        if !path.exists() {
            continue;
        }
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                let bytes = std::fs::read(entry.path()).unwrap();
                assert!(
                    !bytes
                        .windows(PRIVATE.len())
                        .any(|bytes| bytes == PRIVATE.as_bytes())
                );
            }
        }
    }
}

#[tokio::test]
async fn complete_public_and_private_workflow_journeys_submit_one_frozen_envelope_after_rendered_review()
 {
    for private in [false, true] {
        let mut server = Server::new(response).await;
        let (fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        inventory(&mut app, &mut server).await;
        preview(&mut app, &mut server, private).await;
        assert!(app.workflows.panel.as_ref().unwrap().phase == Phase::Confirm);
        key(&mut app, KeyCode::Char('y'));
        assert!(app.views[&target].pending.is_none());
        assert!(server.requests.try_recv().is_err());
        let screen = read_to_end(&mut app);
        assert!(!screen.contains(PRIVATE));
        key(&mut app, KeyCode::Char('y'));
        assert!(!app.workflows_open());
        let pending = app.views[&target].pending.as_ref().unwrap().clone();
        assert!(pending.preserve_draft);
        assert!(pending.draft.is_empty());
        let inputs_id = if private {
            let VoyageCommand::WorkflowInputs { input_id, values } = sent(&mut server).await else {
                panic!("private handoff precedes submit")
            };
            assert_eq!(values, vec![("token".to_owned(), PRIVATE.to_owned())]);
            Some(input_id)
        } else {
            None
        };
        let command = sent(&mut server).await;
        let VoyageCommand::WorkflowSubmit {
            command_id,
            expected_revision,
            expires_at_ms,
            id,
            scope,
            user_directory,
            inputs,
            trust_digest,
            private_inputs_id,
        } = command
        else {
            panic!("submit")
        };
        assert_eq!(command_id, pending.command_id);
        assert_eq!(expected_revision, 17);
        assert!(expires_at_ms > u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap());
        assert_eq!(id, "review-code");
        assert_eq!(scope.as_deref(), Some("repository"));
        assert!(user_directory.is_none());
        assert_eq!(trust_digest.as_deref(), Some("a".repeat(64).as_str()));
        assert_eq!(private_inputs_id, inputs_id);
        assert_eq!(
            inputs,
            vec![
                ("count".to_owned(), "2".to_owned()),
                ("enabled".to_owned(), "true".to_owned())
            ]
        );
        app.views
            .get_mut(&target)
            .unwrap()
            .draft
            .set_text("new unsent workflow composer".into());
        app.reconcile_pending();
        app.update(next(&mut receiver).await);
        assert!(app.views[&target].pending.is_none());
        assert_eq!(
            app.views[&target].draft.text,
            "new unsent workflow composer"
        );
        assert!(server.requests.try_recv().is_err());
        assert_private_files(fixture.0.path());
        assert!(
            app.views[&target]
                .panel
                .as_ref()
                .is_none_or(|panel| !panel.contains(PRIVATE))
        );
    }
}

#[tokio::test]
async fn refused_private_handoff_or_submit_preserves_public_identity_and_recovers_without_private_replay()
 {
    for refuse_handoff in [true, false] {
        let mut server = Server::new(move |command| {
            let VesselCommand::Voyage(request) = command else {
                panic!("workflow only")
            };
            match &request.command {
                VoyageCommand::WorkflowInputs { .. } if refuse_handoff => {
                    Err(format!("private host diagnostic {PRIVATE}"))
                }
                VoyageCommand::WorkflowSubmit { .. } => {
                    Err(format!("private submit diagnostic {PRIVATE}"))
                }
                VoyageCommand::Resolve {
                    command_id,
                    original: Some(original),
                } => {
                    assert!(matches!(
                        original.as_ref(),
                        VoyageCommand::WorkflowSubmit { .. }
                    ));
                    Ok(voyage(
                        command,
                        json!({"command_id":command_id,"status":"not_admitted"}),
                    ))
                }
                _ => response(command),
            }
        })
        .await;
        let (fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        inventory(&mut app, &mut server).await;
        preview(&mut app, &mut server, true).await;
        read_to_end(&mut app);
        key(&mut app, KeyCode::Char('y'));
        let pending = app.views[&target].pending.as_ref().unwrap().clone();
        assert!(matches!(
            sent(&mut server).await,
            VoyageCommand::WorkflowInputs { .. }
        ));
        if !refuse_handoff {
            assert!(matches!(
                sent(&mut server).await,
                VoyageCommand::WorkflowSubmit { .. }
            ));
        }
        let update = next(&mut receiver).await;
        let Update::Command {
            result: Err(error), ..
        } = &update
        else {
            panic!("failed workflow update")
        };
        assert!(!error.contains(PRIVATE));
        app.update(update);
        assert_eq!(
            app.views[&target].pending.as_ref().unwrap().command_id,
            pending.command_id
        );
        assert!(!app.status.contains(PRIVATE));
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        app.command_checks
            .insert((target, pending.command_id), Some(Instant::now()));
        app.reconcile_pending();
        app.reconcile_pending();
        let VoyageCommand::Resolve {
            command_id,
            original,
        } = sent(&mut server).await
        else {
            panic!("resolve, no replay")
        };
        assert_eq!(command_id, pending.command_id);
        assert_eq!(
            serde_json::to_value(original).unwrap(),
            serde_json::to_value(pending.original).unwrap()
        );
        app.update(next(&mut receiver).await);
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(server.requests.try_recv().is_err());
        assert_private_files(fixture.0.path());
    }
}

#[tokio::test]
async fn changed_or_unsupported_host_preview_cannot_advance_to_submission() {
    for case in 0..7 {
        let mut server=Server::new(move|command|{
            let VesselCommand::Voyage(request)=command else {panic!("workflow only")};
            if matches!(request.command,VoyageCommand::WorkflowPreview{..}) {
                let mut value=json!({"definition":definition(),"prompt":"canonical preview","secret_names":["token"]});
                match case{0=>value["definition"]["digest"]=json!("b".repeat(64)),1=>value["definition"]["scope"]=json!("user"),2=>value["definition"]["document"]["id"]=json!("changed"),3=>value["secret_names"]=json!([]),4=>{value.as_object_mut().unwrap().remove("secret_names");},5=>value["prompt"]=Value::Null,_=>value["prompt"]=json!("x".repeat(128*1024+1))};
                Ok(voyage(command,value))
            }else{response(command)}
        }).await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        inventory(&mut app, &mut server).await;
        preview(&mut app, &mut server, true).await;
        let panel = app.workflows.panel.as_ref().unwrap();
        assert!(panel.phase == Phase::Preview, "case {case}");
        assert!(!panel.notice.is_empty());
        assert!(!panel.notice.contains(PRIVATE));
        read_to_end(&mut app);
        key(&mut app, KeyCode::Char('y'));
        assert!(app.views[&target].pending.is_none());
        assert!(server.requests.try_recv().is_err());
        key(&mut app, KeyCode::Esc);
    }
}

#[tokio::test]
async fn workflow_submit_preflight_refuses_new_revision_active_run_recovery_and_storage_before_handoff()
 {
    for case in 0..6 {
        let mut server = Server::new(response).await;
        let (fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        inventory(&mut app, &mut server).await;
        preview(&mut app, &mut server, true).await;
        read_to_end(&mut app);
        match case {
            0 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .revision += 1
            }
            1 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .run = Some(
                    serde_json::from_value(json!({"run_id":Uuid::new_v4(),"state":"running"}))
                        .unwrap(),
                )
            }
            2 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .recovery_pending = true
            }
            3 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .pending_cleanup_run = Some(Uuid::new_v4())
            }
            4 => {
                std::fs::write(
                    fixture.0.path().join("helm-command-receipts"),
                    b"unrelated sentinel",
                )
                .unwrap();
            }
            _ => {
                app.workflows
                    .panel
                    .as_mut()
                    .unwrap()
                    .private
                    .insert("token".into(), Zeroizing::new("x".repeat(65537)));
            }
        }
        key(&mut app, KeyCode::Char('y'));
        assert!(app.workflows_open());
        assert!(app.views[&target].pending.is_none());
        assert!(app.command_checks.is_empty());
        assert!(server.requests.try_recv().is_err());
        assert!(!app.workflows.panel.as_ref().unwrap().notice.is_empty());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        key(&mut app, KeyCode::Esc);
    }
}

#[tokio::test]
async fn workflow_private_context_change_requires_acknowledgement_and_discards_review_state() {
    for case in 0..4 {
        let mut server = Server::new(response).await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        inventory(&mut app, &mut server).await;
        preview(&mut app, &mut server, true).await;
        match case {
            0 => app.selected = None,
            1 => app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4(),
            2 => app.clients.mark_unavailable(target.route),
            _ => {
                app.workflows.panel.as_mut().unwrap().deadline =
                    Instant::now() - Duration::from_secs(1)
            }
        }
        app.poll_workflows();
        let panel = app.workflows.panel.as_ref().unwrap();
        assert!(panel.phase == Phase::Invalidated);
        assert!(panel.private.is_empty());
        assert!(panel.editor.is_empty());
        assert!(panel.public.is_empty());
        assert!(panel.preview.is_empty());
        app.workflow_input(&Event::Paste(PRIVATE.into())).unwrap();
        key(&mut app, KeyCode::Char('y'));
        assert!(app.workflows_open());
        assert!(app.views[&target].pending.is_none());
        assert!(server.requests.try_recv().is_err());
        key(&mut app, KeyCode::Esc);
        assert!(!app.workflows_open());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
}

#[tokio::test]
async fn unsupported_inventory_or_private_host_error_stays_readonly_and_never_displays_diagnostics()
{
    for case in 0..5 {
        let mut server = Server::new(move |command| {
            let VesselCommand::Voyage(request) = command else {
                panic!("workflow only")
            };
            assert!(matches!(request.command, VoyageCommand::Controls { .. }));
            let mut entry = definition();
            let value = match case {
                0 => return Err(format!("private inventory diagnostic {PRIVATE}")),
                1 => json!({"section":"workflows","value":vec![definition();257]}),
                2 => {
                    entry["digest"] = json!("invalid");
                    json!({"section":"workflows","value":[entry]})
                }
                3 => {
                    entry["document"]["schema_version"] = json!(2);
                    json!({"section":"workflows","value":[entry]})
                }
                _ => json!({"section":"workflows","value":[{"document":"malformed"}]}),
            };
            Ok(voyage(command, value))
        })
        .await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        app.open_workflows().unwrap();
        sent(&mut server).await;
        observed(&mut app).await;
        let panel = app.workflows.panel.as_ref().unwrap();
        assert!(panel.phase == Phase::Inventory);
        assert!(panel.entries.is_empty());
        assert!(!panel.notice.is_empty());
        assert!(!panel.notice.contains(PRIVATE));
        read_to_end(&mut app);
        key(&mut app, KeyCode::Enter);
        assert!(app.views[&target].pending.is_none());
        assert!(server.requests.try_recv().is_err());
        key(&mut app, KeyCode::Esc);
    }
}
