use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use voyage_protocol::{
    duplex::{ClientFrame, SUBPROTOCOL, ServerFrame},
    vessel::VesselResponse,
};
// The handshake callback error type is fixed by tungstenite.
#[allow(clippy::result_large_err)]
async fn peer(
    root: &std::path::Path,
    replies: Vec<Value>,
) -> (
    crate::process_client::transport::Client,
    tokio::task::JoinHandle<Vec<Value>>,
) {
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
    (
        crate::process_client::transport::Client::local(root.into()),
        task,
    )
}

#[tokio::test(flavor = "current_thread")]
async fn status_journey_verifies_current_release_and_retains_exact_operation() {
    let fixture = super::super::account_test_support::Fixture::new();
    let operation = Uuid::new_v4();
    let vessel = Uuid::new_v4();
    let release = "b".repeat(64);
    let caps = json!({"vessel_id":vessel,"remote_updates":true,"features":["verified_user_updates"],"running_release":release});
    let (client, task) = peer(
        fixture.0.path(),
        vec![
            caps.clone(),
            json!({"operation_id":operation,"phase":"complete","release_id":release}),
            caps,
        ],
    )
    .await;
    let result = perform(&client, vec!["status".into(), operation.to_string()])
        .await
        .unwrap();
    assert!(result.contains("complete"));
    assert_eq!(restored(client.id()).unwrap(), Some([vessel, operation]));
    let seen = task.await.unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[1]["operation_id"], operation.to_string());
    assert_eq!(seen[1]["op"], "update_status");
}
#[tokio::test(flavor = "current_thread")]
async fn missing_updater_refuses_before_any_effect() {
    let fixture = super::super::account_test_support::Fixture::new();
    let (client, task) = peer(fixture.0.path(), vec![json!({"remote_updates":false})]).await;
    assert!(
        perform(&client, vec!["prepare".into(), "nightly".into()])
            .await
            .unwrap_err()
            .to_string()
            .contains("bootstrap")
    );
    assert_eq!(task.await.unwrap().len(), 1);
    assert!(restored(client.id()).unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn historical_completed_receipt_does_not_establish_current_activation() {
    let fixture = super::super::account_test_support::Fixture::new();
    let operation = Uuid::new_v4();
    let vessel = Uuid::new_v4();
    let caps = json!({"vessel_id":vessel,"remote_updates":true,"features":["verified_user_updates"],"running_release":"a".repeat(64)});
    let (client, task) = peer(
        fixture.0.path(),
        vec![
            caps.clone(),
            json!({"operation_id":operation,"phase":"complete","release_id":"b".repeat(64)}),
            caps,
        ],
    )
    .await;
    assert!(
        perform(&client, vec!["status".into(), operation.to_string()])
            .await
            .unwrap_err()
            .to_string()
            .contains("historical")
    );
    assert_eq!(task.await.unwrap().len(), 3);
    assert_eq!(restored(client.id()).unwrap(), Some([vessel, operation]));
}
#[tokio::test(flavor = "current_thread")]
async fn stale_review_refuses_apply_before_mutation() {
    let fixture = super::super::account_test_support::Fixture::new();
    let operation = Uuid::new_v4();
    let vessel = Uuid::new_v4();
    let release = "b".repeat(64);
    let caps = json!({"vessel_id":vessel,"remote_updates":true,"features":["verified_user_updates"],"running_release":"a".repeat(64)});
    let receipt = json!({"operation_id":operation,"phase":"ready","release_id":release,"current_release":"a".repeat(64),"expires_at":0});
    let (client, task) = peer(fixture.0.path(), vec![caps, receipt]).await;
    assert!(
        perform(
            &client,
            vec!["approve".into(), operation.to_string(), release]
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("expired")
    );
    let seen = task.await.unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1]["op"], "update_status");
    assert!(restored(client.id()).unwrap().is_none());
}
