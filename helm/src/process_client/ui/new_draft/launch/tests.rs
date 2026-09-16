use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use voyage_protocol::{
    duplex::{ClientFrame, SUBPROTOCOL, ServerFrame},
    vessel::VesselResponse,
};
async fn peer(
    root: &std::path::Path,
    replies: Vec<Value>,
) -> (Client, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    std::fs::write(
        root.join("process-http.json"),
        json!({"endpoint":endpoint,"token":"a".repeat(64)}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(
        root.join("process-http.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws=tokio_tungstenite::accept_hdr_async(stream,|_:&tokio_tungstenite::tungstenite::handshake::server::Request,mut response:tokio_tungstenite::tungstenite::handshake::server::Response|{response.headers_mut().insert("sec-websocket-protocol",SUBPROTOCOL.parse().unwrap());Ok(response)}).await.unwrap();
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&ServerFrame::Hello {
                protocol: 1,
                socket_id: Uuid::new_v4(),
                vessel_id: Uuid::new_v4(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        let mut seen = vec![];
        for result in replies {
            loop {
                let message = ws.next().await.unwrap().unwrap();
                if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                    let ClientFrame::Command {
                        request_id,
                        request,
                    } = serde_json::from_str(&text).unwrap()
                    else {
                        panic!()
                    };
                    seen.push(serde_json::to_value(request.command).unwrap());
                    ws.send(tokio_tungstenite::tungstenite::Message::Text(
                        serde_json::to_string(&ServerFrame::Reply {
                            request_id,
                            response: VesselResponse {
                                protocol: 1,
                                result,
                                error: None,
                                outcome_unknown: false,
                            },
                        })
                        .unwrap()
                        .into(),
                    ))
                    .await
                    .unwrap();
                    break;
                }
            }
        }
        seen
    });
    (Client::local(root.into()), task)
}
fn saved(client: &Client, workspace: &std::path::Path) -> Saved {
    let id = Uuid::new_v4();
    Saved {
        id,
        route: storage::route(client).unwrap(),
        workspace: workspace.into(),
        config: None,
        account_host: None,
        account_settings: None,
        explicit: Default::default(),
        selection: None,
        confirmation: None,
        text: "fixture first send".into(),
        markers: None,
        images: vec![],
        start: Some(VesselCommand::Start {
            command_id: id,
            session_id: id,
            workspace: workspace.into(),
        }),
        start_attempted: false,
        process: None,
        turn: Uuid::new_v4(),
        submit: None,
        attempted: false,
        finished: false,
        receipt: None,
    }
}
fn process(saved: &Saved) -> ProcessInfo {
    ProcessInfo {
        session_id: saved.id,
        incarnation: Uuid::new_v4(),
        workspace: saved.workspace.clone(),
        state: voyage_protocol::process::ProcessState::Live,
        name: None,
        catalogue: None,
        archive: None,
        deletion: None,
    }
}
fn reply(p: &ProcessInfo, value: Value) -> Value {
    json!({"session_id":p.session_id,"incarnation":p.incarnation,"result":value})
}
#[tokio::test]
async fn initial_launch_persists_before_start_and_submit_and_reuses_terminal_receipt() {
    let fixture = crate::process_client::ui::account_test_support::Fixture::new();
    let root = fixture.0.path();
    let dummy = Client::local(root.into());
    let mut state = saved(&dummy, root);
    let p = process(&state);
    let receipt = json!({"command_id":state.turn,"status":"accepted"});
    let (client, task) = peer(
        root,
        vec![
            json!(p),
            reply(
                &p,
                json!({"session_id":p.session_id,"revision":0,"model":"fixture","messages":[]}),
            ),
            reply(&p, receipt.clone()),
        ],
    )
    .await;
    assert_eq!(
        advance(&client, &mut state).await.unwrap(),
        Some(receipt.clone())
    );
    assert!(state.start_attempted && state.attempted && state.submit.is_some());
    assert_eq!(observe(&client, &mut state).await.unwrap(), Some(receipt));
    let seen = task.await.unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0]["op"], "start");
    assert_eq!(seen[2]["op"], "submit");
    client.disconnect();
}
#[tokio::test]
async fn uncertain_submission_resolves_exact_saved_command_without_replay() {
    let fixture = crate::process_client::ui::account_test_support::Fixture::new();
    let root = fixture.0.path();
    let dummy = Client::local(root.into());
    let mut state = saved(&dummy, root);
    let p = process(&state);
    state.process = Some(p.clone());
    state.start_attempted = true;
    state.attempted = true;
    state.submit = Some(VoyageCommand::Submit {
        coordination: None,
        command_id: state.turn,
        expected_revision: 0,
        expires_at_ms: 1,
        prompt: state.text.clone(),
    });
    let receipt = json!({"command_id":state.turn,"status":"accepted"});
    let (client, task) = peer(
        root,
        vec![
            reply(&p, json!({"status":"unknown"})),
            reply(&p, receipt.clone()),
        ],
    )
    .await;
    assert_eq!(observe(&client, &mut state).await.unwrap(), Some(receipt));
    let seen = task.await.unwrap();
    assert_eq!(
        seen.iter()
            .map(|v| v["op"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["receipt", "resolve"]
    );
    client.disconnect();
}
#[tokio::test]
async fn observation_of_unsent_creation_does_not_connect_or_start() {
    let fixture = crate::process_client::ui::account_test_support::Fixture::new();
    let client = Client::local(fixture.0.path().into());
    let mut state = saved(&client, fixture.0.path());
    assert!(observe(&client, &mut state).await.unwrap().is_none());
    assert!(!state.start_attempted && !state.attempted);
    assert!(
        retain_receipt(
            &mut state,
            json!({"command_id":Uuid::new_v4(),"status":"accepted"})
        )
        .is_err()
    );
    let turn = state.turn;
    assert!(retain_receipt(&mut state, json!({"command_id":turn,"status":"unknown"})).is_err());
}
