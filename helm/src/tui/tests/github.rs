use super::routing::navigation_agent;
use super::*;

#[tokio::test]
async fn github_preview_requires_visible_end_and_ignores_pasted_approval() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("retained draft λ");
    app.display_area = ratatui::layout::Rect::new(0, 0, 40, 12);
    let (response, mut answer) = oneshot::channel();
    app.approval = Some(ApprovalRequest {
        id: Uuid::new_v4(),
        action: "github.publish".into(),
        target: "https://github.com/o/r/issues/1".into(),
        reason: format!(
            "{}TAIL_EXACT_CONTENT",
            "界 untrusted \u{1b}[31m\u{202e}\n".repeat(100)
        ),
        response,
    });
    for event in [
        Event::Paste("y\n".into()),
        Event::Key(KeyEvent::from(KeyCode::Char('y'))),
    ] {
        handle_input_event(
            event,
            &mut app,
            &agent,
            &mut store,
            &tx,
            &terminals,
            supervisor.clone(),
            todos.clone(),
        )
        .await
        .unwrap();
        assert!(answer.try_recv().is_err());
    }
    handle_key(
        KeyEvent::from(KeyCode::End),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    let mut screen = Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
    screen.draw(|frame| draw(frame, &app)).unwrap();
    let text = screen
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("TAIL_EXACT_CONTENT"), "{text}");
    app.display_area = ratatui::layout::Rect::new(0, 0, 10, 4);
    handle_key(
        KeyEvent::from(KeyCode::Char('y')),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(answer.try_recv().is_err(), "hidden preview cannot approve");
    app.display_area = ratatui::layout::Rect::new(0, 0, 40, 12);
    handle_key(
        KeyEvent::from(KeyCode::Char('y')),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor,
        todos,
    )
    .await
    .unwrap();
    assert_eq!(answer.await.unwrap(), ApprovalOutcome::Approved);
    assert_eq!(app.composer.text, "retained draft λ");
    assert!(app.session.messages.is_empty());
}

#[tokio::test]
async fn stale_github_approval_and_result_cannot_mutate_new_voyage() {
    let directory = tempfile::tempdir().unwrap();
    let store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    let old = Uuid::new_v4();
    let request = Uuid::new_v4();
    app.github_panel.open = true;
    app.github_panel.session = Some(app.session.id);
    app.github_panel.request = Some(request);
    app.github_panel.display = "current operation".into();
    let (response, answer) = oneshot::channel();
    handle_ui_event(
        UiEvent::GithubApproval {
            session: old,
            request,
            approval: ApprovalRequest {
                id: Uuid::new_v4(),
                action: "github.publish".into(),
                target: "old".into(),
                reason: "private old body".into(),
                response,
            },
        },
        &mut app,
        &store,
        &terminals,
    )
    .await
    .unwrap();
    assert_eq!(answer.await.unwrap(), ApprovalOutcome::Unavailable);
    handle_ui_event(
        UiEvent::GithubResult {
            session: old,
            request,
            result: Err("old result".into()),
        },
        &mut app,
        &store,
        &terminals,
    )
    .await
    .unwrap();
    assert!(app.approval.is_none());
    assert_eq!(app.github_panel.display, "current operation");
    assert!(app.session.github_references.is_empty());
    app.github_panel.close();
    assert!(!app.github_panel.matches(app.session.id, request));
}

#[tokio::test]
async fn github_denial_and_cancel_preserve_draft_and_never_approve() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("keep composer");
    for code in [KeyCode::Char('n'), KeyCode::Esc] {
        let (response, answer) = oneshot::channel();
        app.approval = Some(ApprovalRequest {
            id: Uuid::new_v4(),
            action: "github.publish".into(),
            target: "https://github.com/o/r/issues/1".into(),
            reason: "full body".into(),
            response,
        });
        handle_key(
            KeyEvent::from(code),
            &mut app,
            &agent,
            &mut store,
            &tx,
            &terminals,
            supervisor.clone(),
            todos.clone(),
        )
        .await
        .unwrap();
        assert_eq!(answer.await.unwrap(), ApprovalOutcome::Denied);
    }
    app.github_panel.open = true;
    app.github_panel.request = Some(Uuid::new_v4());
    handle_key(
        KeyEvent::from(KeyCode::Esc),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor,
        todos,
    )
    .await
    .unwrap();
    assert!(!app.github_panel.open);
    assert!(app.github_panel.request.is_none());
    assert_eq!(app.composer.text, "keep composer");
    assert!(app.session.messages.is_empty());
}

#[tokio::test]
async fn github_reference_save_failure_keeps_original_session_and_private_draft() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("not-a-directory");
    std::fs::write(&destination, b"fixture").unwrap();
    let store = SessionStore::new(destination);
    let agent = navigation_agent(&directory);
    let terminals = FakeTerminals::new();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("unsent composer");
    app.github_panel.authority = Some(agent);
    let session = app.session.id;
    let request = Uuid::new_v4();
    app.github_panel.open = true;
    app.github_panel.session = Some(session);
    app.github_panel.request = Some(request);
    let reference = crate::github::operator::Reference {
        object: crate::github::repository::Object::parse("https://github.com/o/r/issues/1")
            .unwrap(),
        head: None,
        fetched_at: Utc::now(),
    };
    handle_ui_event(
        UiEvent::GithubResult {
            session,
            request,
            result: Ok(crate::github::operator::CommandResult {
                display: "Observed reference".into(),
                reference: Some(reference),
                feedback: None,
            }),
        },
        &mut app,
        &store,
        &terminals,
    )
    .await
    .unwrap();
    assert!(app.github_panel.display.contains("not saved"));
    assert!(app.session.github_references.is_empty());
    assert_eq!(app.session.revision, 0);
    assert_eq!(app.composer.text, "unsent composer");
    assert!(app.session.messages.is_empty());
}

#[tokio::test]
async fn github_panel_blocks_model_dispatch_and_session_navigation_until_closed() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("preserved draft");
    app.github_panel.open = true;
    let (tx, _rx) = mpsc::unbounded_channel();
    assert!(!start_run(&mut app, &agent, &mut store, &tx, "must not dispatch".into(), None).await.unwrap());
    assert!(crate::tui::commands::request_navigation(&mut app, &store, "other".into()).await.is_err());
    assert!(!app.is_running());
    assert!(app.session.messages.is_empty());
    assert_eq!(app.composer.text, "preserved draft");
    assert!(app.exit.is_none());
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PublicationCase { Published, Denied, Escaped, Cancelled, StalePolicy, Disconnected }

async fn publication_case(case: PublicationCase) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = reqwest::Url::parse(&format!("http://{}/",listener.local_addr().unwrap())).unwrap();
    let posts = Arc::new(AtomicUsize::new(0));
    let server_posts = posts.clone();
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let body = format!("{}FINAL_EXACT_BODY", "selected feedback line\n".repeat(80));
    let expected_body = body.clone();
    let server: tokio::task::JoinHandle<()> = tokio::spawn(async move {
        loop {
            let (mut socket,_) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let (end,length) = loop {
                let mut buffer = [0;4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count>0); request.extend_from_slice(&buffer[..count]);
                assert!(request.len()<128*1024);
                if let Some(end) = request.windows(4).position(|part|part==b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length = headers.lines().find_map(|line|line.strip_prefix("content-length: ")).map(|value|value.parse::<usize>().unwrap()).unwrap_or(0);
                    assert!(headers.contains("authorization: bearer fixture-github-token"));
                    break (end+4,length);
                }
            };
            while request.len()<end+length {
                let mut buffer = [0;4096];
                let count=socket.read(&mut buffer).await.unwrap();assert!(count>0);request.extend_from_slice(&buffer[..count]);
            }
            let first = String::from_utf8_lossy(&request[..end]).lines().next().unwrap().to_owned();
            server_requests.fetch_add(1,Ordering::SeqCst);
            let published = first.starts_with("POST ");
            let value = if published {
                assert!(first.starts_with("POST /repos/o/r/issues/1/comments "));
                let sent:serde_json::Value = serde_json::from_slice(&request[end..end+length]).unwrap();
                assert_eq!(sent,serde_json::json!({"body":expected_body}));
                server_posts.fetch_add(1,Ordering::SeqCst);
                if case == PublicationCase::Disconnected { drop(socket); continue; }
                serde_json::json!({"id":9,"html_url":"https://github.com/o/r/issues/1#issuecomment-9","user":{"id":7},"body":expected_body})
            } else if first.starts_with("GET /user ") {
                serde_json::json!({"id":7,"login":"fixture"})
            } else {
                assert!(first.starts_with("GET /repos/o/r/issues/1 "),"{first}");
                serde_json::json!({"number":1,"html_url":"https://github.com/o/r/issues/1","state":"open"})
            };
            let bytes=serde_json::to_vec(&value).unwrap();
            let status=if published {"201 Created"} else {"200 OK"};
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).as_bytes()).await.unwrap();
            socket.write_all(&bytes).await.unwrap();

        }
    });
    let agent=navigation_agent(&directory);
    let mut store=SessionStore::new(directory.path().join("sessions"));
    let mut app=App::new(Session::new(directory.path().into(),"fixture".into()),vec![]);
    let request=Uuid::new_v4();let session=app.session.id;
    let (tx,mut rx)=mpsc::unbounded_channel();
    app.github_panel.open=true;app.github_panel.session=Some(session);app.github_panel.request=Some(request);
    app.display_area=ratatui::layout::Rect::new(0,0,40,12);
    app.composer.insert_str("keep unsent draft");
    let profiles = if case == PublicationCase::StalePolicy {
        use crate::policy_profile::{Builtin, Overrides, selection::{Selection,SelectionRequest}, store::{Action,ProfileChange,ProfileStore}};
        let directory=directory.path().join("profiles");
        let profiles=ProfileStore::open(&directory).unwrap();
        let snapshot=profiles.change(&ProfileChange {operation_id:Uuid::new_v4(),name:"github-fixture".into(),expected_revision:0,action:Action::Create {rules:Builtin::Autonomous.document().rules}}).unwrap().snapshot;
        let mut config=crate::config::Config {access:Some(crate::config::AccessMode::Unrestricted),..Default::default()};
        let selection=SelectionRequest {directory,name:snapshot.name.clone(),revision:snapshot.revision,digest:snapshot.digest().unwrap(),explicit:Overrides::default()};
        let preview=Selection::preview(&config,app.session.workspace.as_path(),&selection).unwrap();
        config.policy_profile=Some(Selection::bind(&config,app.session.workspace.as_path(),selection,Some(&preview.transition_digest)).unwrap());
        Some((profiles,Arc::new(crate::policy::Policy::new(&config,app.session.workspace.clone()).unwrap())))
    } else {None};
    let context=crate::tools::ToolContext {
        completion:None,
        policy:profiles.as_ref().map(|(_,policy)|policy.clone()).unwrap_or_else(||Arc::new(crate::policy::Policy::new(&crate::config::Config {access:Some(crate::config::AccessMode::Unrestricted),..Default::default()},directory.path().into()).unwrap())),
        approver:crate::tui::github::scoped_approver(tx.clone(),session,request),
        timeout:Duration::from_secs(5),max_output_bytes:64*1024,
        environment:[("HELM_GITHUB_TOKEN".into(),"fixture-github-token".into())].into_iter().collect(),
        cancellation:tokio_util::sync::CancellationToken::new(),execution_id:Uuid::new_v4(),
        interaction:crate::tools::InteractionMode::Attended,redactor:Arc::new(crate::tools::Redactor::default()),
    };
    let store_path=directory.path().join("github-store");
    let service=crate::github::service::Service::fixture(context.clone(),Some(session),origin.clone(),store_path.clone()).unwrap();
    let owner=service.owner().clone();
    let operation=service.prepare(crate::github::publication::Draft {object:crate::github::repository::Object::parse("https://github.com/o/r/issues/1").unwrap(),action:crate::github::publication::Action::Comment {body}}).await.unwrap();
    let operation_id=operation.id;
    let digest=operation.digest.clone();
    let (finished_tx,finished_rx)=oneshot::channel();
    let worker=tokio::spawn(async move {let result=service.publish(operation.id,&operation.digest).await;let _=finished_tx.send(result);});
    app.github_panel.own_test_job(worker,context.cancellation.clone());
    let event=tokio::time::timeout(Duration::from_secs(5),rx.recv()).await.unwrap().unwrap();
    let terminals=FakeTerminals::new();
    handle_ui_event(event,&mut app,&store,&terminals).await.unwrap();
    assert!(app.approval.as_ref().unwrap().reason.contains("FINAL_EXACT_BODY"));
    let supervisor=Arc::new(FakeSupervisor::new(vec![]));let todos=todo_store(&directory);
    handle_input_event(Event::Paste("y\n".into()),&mut app,&agent,&mut store,&tx,&terminals,supervisor.clone(),todos.clone()).await.unwrap();
    assert_eq!(posts.load(Ordering::SeqCst),0);
    handle_key(KeyEvent::from(KeyCode::Char('y')),&mut app,&agent,&mut store,&tx,&terminals,supervisor.clone(),todos.clone()).await.unwrap();
    assert_eq!(posts.load(Ordering::SeqCst),0);
    if let Some((profiles,policy))=&profiles {
        use crate::policy_profile::store::{Action,ProfileChange};
        profiles.change(&ProfileChange {operation_id:Uuid::new_v4(),name:"github-fixture".into(),expected_revision:1,action:Action::Delete {}}).unwrap();
        assert!(policy.check_current().is_err());
    }
    let keys=match case {
        PublicationCase::Denied=>vec![KeyCode::Char('n')],
        PublicationCase::Escaped=>vec![KeyCode::Esc],
        PublicationCase::Cancelled=>vec![KeyCode::Esc,KeyCode::Esc],
        _=>vec![KeyCode::End,KeyCode::Char('y')],
    };
    for code in keys {
        handle_key(KeyEvent::from(code),&mut app,&agent,&mut store,&tx,&terminals,supervisor.clone(),todos.clone()).await.unwrap();
    }
    let completed=tokio::time::timeout(Duration::from_secs(5),finished_rx).await.unwrap();
    if case==PublicationCase::Cancelled {
        assert!(completed.is_err(),"closing the panel must abort its pending task");
        assert!(context.cancellation.is_cancelled());
        assert!(!app.github_panel.open);
    }
    let result=completed.ok();
    let persisted=crate::github::store::Store::open(store_path.clone()).unwrap().inspect(operation_id,&owner).unwrap();
    if case==PublicationCase::Published {
        let published=result.unwrap().unwrap();
        assert_eq!(published.state,crate::github::store::State::Published);
        assert_eq!(posts.load(Ordering::SeqCst),1);
        assert_eq!(published.receipt.unwrap().id,9);
        assert_eq!(persisted.state,crate::github::store::State::Published);
    } else {
        assert!(result.as_ref().is_none_or(|result|result.is_err()),"failure scenario must not claim publication");
        assert!(persisted.receipt.is_none());
        assert_eq!(persisted.id,operation_id);
        assert_eq!(persisted.digest,digest);
        let expected=if case==PublicationCase::Disconnected {crate::github::store::State::Sending} else {crate::github::store::State::Prepared};
        assert_eq!(persisted.state,expected);
        assert_eq!(posts.load(Ordering::SeqCst),usize::from(case==PublicationCase::Disconnected));
        if case==PublicationCase::Disconnected {
            let before=requests.load(Ordering::SeqCst);
            let reopened=crate::github::service::Service::fixture(context,Some(session),origin,store_path).unwrap();
            assert!(reopened.publish(operation_id,&digest).await.is_err());
            assert_eq!(requests.load(Ordering::SeqCst),before,"uncertain recovery must not revalidate over HTTP or send again");
            assert_eq!(posts.load(Ordering::SeqCst),1);
            assert_eq!(reopened.inspect(operation_id).await.unwrap().state,crate::github::store::State::Sending);
            assert!(rx.try_recv().is_err(),"uncertain retry must not create another approval");
        }
    }
    assert_eq!(app.composer.text,"keep unsent draft");
    assert!(app.session.messages.is_empty());
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled(),"fixture server must not panic");
}

#[tokio::test]
async fn github_real_publication_waits_for_complete_attended_tui_preview() {
    publication_case(PublicationCase::Published).await;
}
#[tokio::test]
async fn github_real_publication_denial_sends_nothing() {
    publication_case(PublicationCase::Denied).await;
}
#[tokio::test]
async fn github_real_publication_escape_cancels_confirmation_without_sending() {
    publication_case(PublicationCase::Escaped).await;
}
#[tokio::test]
async fn github_real_publication_rejects_policy_changed_while_preview_is_open() {
    publication_case(PublicationCase::StalePolicy).await;
}
#[tokio::test]
async fn github_real_publication_lost_response_remains_uncertain_without_resending() {
    publication_case(PublicationCase::Disconnected).await;
}

#[tokio::test]
async fn github_real_publication_closing_panel_cancels_owned_task_without_sending() {
    publication_case(PublicationCase::Cancelled).await;
}
