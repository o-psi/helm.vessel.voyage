//! Plain first-send behavior over the real local duplex client and private journal.
use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use voyage_protocol::{
    duplex::{ClientFrame, SUBPROTOCOL, ServerFrame},
    vessel::VesselResponse,
};

// This peer supplies public replies only. Production launch/persistence/transport
// code chooses identities and freezes the actual creation and submission bytes.
#[allow(clippy::result_large_err)]
async fn peer(
    root: &std::path::Path,
    outcome: &'static str,
) -> (Client, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::fs::write(root.join("process-http.json"), json!({"endpoint":format!("http://{}",listener.local_addr().unwrap()),"token":"b".repeat(64)}).to_string()).unwrap();
    std::fs::set_permissions(
        root.join("process-http.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let private_root = root.to_owned();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_hdr_async(stream, |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            response.headers_mut().insert("sec-websocket-protocol", SUBPROTOCOL.parse().unwrap());
            Ok(response)
        }).await.unwrap();
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
        let mut commands = Vec::new();
        let incarnation = Uuid::new_v4();
        let mut session = Uuid::nil();
        for step in 0..3 {
            let text = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match ws.next().await.unwrap().unwrap() {
                        tokio_tungstenite::tungstenite::Message::Text(text) => break text,
                        tokio_tungstenite::tungstenite::Message::Ping(bytes) => {
                            ws.send(tokio_tungstenite::tungstenite::Message::Pong(bytes))
                                .await
                                .unwrap();
                        }
                        other => panic!("unexpected owned peer frame: {other:?}"),
                    }
                }
            })
            .await
            .unwrap();
            let ClientFrame::Command {
                request_id,
                request,
            } = serde_json::from_str(&text).unwrap()
            else {
                panic!("unexpected synthetic peer frame")
            };
            let command = serde_json::to_value(request.command).unwrap();
            let result = match step {
                0 => {
                    assert_eq!(command["op"], "start_configured");
                    session = serde_json::from_value(command["session_id"].clone()).unwrap();
                    let saved = journal::reload(session).unwrap().unwrap();
                    assert!(saved.start_attempted && !saved.attempted);
                    assert_eq!(serde_json::to_value(saved.start).unwrap(), command);
                    assert_eq!(saved.text, "plain synthetic first turn");
                    let config_path: PathBuf =
                        serde_json::from_value(command["config_path"].clone()).unwrap();
                    assert!(config_path.starts_with(&private_root));
                    let published: Value =
                        serde_json::from_slice(&std::fs::read(config_path).unwrap()).unwrap();
                    assert_eq!(published["version"], 1);
                    assert_eq!(published["config"]["model"], "synthetic-model");
                    assert_eq!(published["workspace"], private_root.to_str().unwrap());
                    json!({"session_id":session,"incarnation":incarnation,"workspace":private_root,"state":"live"})
                }
                1 => {
                    assert_eq!(command["op"], "snapshot");
                    assert_eq!(command["session_id"], session.to_string());
                    json!({"session_id":session,"incarnation":incarnation,"result":{"session_id":session,"revision":23,"messages":[]}})
                }
                _ => {
                    assert_eq!(command["op"], "submit");
                    assert_eq!(command["prompt"], "plain synthetic first turn");
                    assert_eq!(command["expected_revision"], 23);
                    let saved = journal::reload(session).unwrap().unwrap();
                    assert!(saved.start_attempted && saved.attempted);
                    assert_eq!(saved.turn.to_string(), command["command_id"]);
                    let mut frozen = serde_json::to_value(saved.submit).unwrap();
                    let object = command.as_object().unwrap();
                    frozen
                        .as_object_mut()
                        .unwrap()
                        .insert("session_id".into(), object["session_id"].clone());
                    if let Some(value) = object.get("incarnation") {
                        frozen
                            .as_object_mut()
                            .unwrap()
                            .insert("incarnation".into(), value.clone());
                    }
                    assert_eq!(frozen, command);
                    let receipt_id = if outcome == "wrong_identity" {
                        Uuid::new_v4().to_string()
                    } else {
                        saved.turn.to_string()
                    };
                    let status = if outcome == "wrong_identity" {
                        "accepted"
                    } else {
                        outcome
                    };
                    json!({"session_id":session,"incarnation":incarnation,"result":{"command_id":receipt_id,"status":status}})
                }
            };
            commands.push(command);
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
        }
        commands
    });
    (Client::local(root.to_owned()), task)
}

#[tokio::test]
async fn plain_first_send_publishes_config_and_freezes_each_identity_before_wire_effects() {
    let fixture = crate::process_client::ui::account_test_support::Fixture::new();
    let root = fixture.0.path();
    let (client, task) = peer(root, "accepted").await;
    let config = crate::Config {
        workspace: Some(root.into()),
        model: "synthetic-model".into(),
        ..Default::default()
    };
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        start_plain(&client, config, "plain synthetic first turn".into()),
    )
    .await
    .unwrap()
    .unwrap();
    let commands = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    client.disconnect();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0]["session_id"], result.session_id.to_string());
    assert!(journal::reload(result.session_id).unwrap().is_none());
    let path = root
        .join("helm-first-send-receipts")
        .join(format!("{}.json", result.session_id));
    let retained: Saved = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(retained.finished && retained.attempted);
    assert_eq!(
        retained.receipt.as_ref().unwrap()["command_id"],
        commands[2]["command_id"]
    );
}

#[tokio::test]
async fn plain_unknown_rejected_and_foreign_receipts_preserve_the_original_recovery_envelope() {
    for outcome in ["unknown", "rejected", "wrong_identity"] {
        let fixture = crate::process_client::ui::account_test_support::Fixture::new();
        let root = fixture.0.path();
        let (client, task) = peer(root, outcome).await;
        let config = crate::Config {
            workspace: Some(root.into()),
            model: "synthetic-model".into(),
            ..Default::default()
        };
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            start_plain(&client, config, "plain synthetic first turn".into()),
        )
        .await
        .unwrap();
        let commands = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        client.disconnect();
        assert!(
            result.is_err(),
            "uncertain or refused first send cannot become a completed launch"
        );
        assert_eq!(commands.len(), 3, "no creation or submission replay");
        let session: Uuid = serde_json::from_value(commands[0]["session_id"].clone()).unwrap();
        let retained = journal::reload(session).unwrap().unwrap();
        assert!(!retained.finished && retained.start_attempted && retained.attempted);
        assert_eq!(retained.text, "plain synthetic first turn");
        assert_eq!(retained.turn.to_string(), commands[2]["command_id"]);
        assert_eq!(retained.process.as_ref().unwrap().session_id, session);
        assert_eq!(retained.receipt.is_some(), outcome == "rejected");
        assert_eq!(serde_json::to_value(retained.start).unwrap(), commands[0]);
    }
}

fn empty_saved(workspace: &std::path::Path) -> Saved {
    Saved {
        id: Uuid::new_v4(),
        route: "connection:synthetic".into(),
        workspace: workspace.into(),
        config: None,
        account_host: None,
        account_settings: None,
        explicit: Default::default(),
        selection: None,
        confirmation: None,
        text: "retained task".into(),
        markers: None,
        images: vec![],
        start: None,
        start_attempted: false,
        process: None,
        turn: Uuid::new_v4(),
        submit: None,
        attempted: false,
        finished: false,
        receipt: None,
    }
}

#[test]
fn recovered_creation_preserves_the_original_config_and_atomic_account_selection() {
    let fixture = crate::process_client::ui::account_test_support::Fixture::new();
    let account = voyage_protocol::accounts::AccountBinding {
        account_id: Uuid::new_v4(),
        connection_id: Uuid::new_v4(),
        identity_generation: 7,
        connection_revision: 4,
        transport: voyage_protocol::accounts::Transport::OpenaiResponses,
    };
    for kind in ["start", "start_configured", "start_account"] {
        let mut saved = empty_saved(fixture.0.path());
        let command = Uuid::new_v4();
        saved.start = Some(match kind {
            "start" => VesselCommand::Start {
                command_id: command,
                session_id: saved.id,
                workspace: saved.workspace.clone(),
            },
            "start_configured" => VesselCommand::StartConfigured {
                command_id: command,
                session_id: saved.id,
                workspace: saved.workspace.clone(),
                config_path: fixture.0.path().join("launch.json"),
            },
            _ => {
                saved.account_host = Some(Uuid::new_v4());
                saved.account_settings = Some(super::super::inference::Settings {
                    account: Some(account.clone()),
                    model: "original-model".into(),
                    reasoning_effort: Some("high".into()),
                    service_tier: Some("default".into()),
                    ..Default::default()
                });
                VesselCommand::StartAccount {
                    config_path: Some(fixture.0.path().join("launch.json")),
                    command_id: command,
                    session_id: saved.id,
                    workspace: saved.workspace.clone(),
                    account: account.clone(),
                    model: "original-model".into(),
                    reasoning_effort: Some("high".into()),
                    service_tier: Some("default".into()),
                }
            }
        });
        saved.start_attempted = true;
        saved.validate_identity().unwrap();
        let restored: Saved = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        let (identity, resolution) = restored.start_resolution().unwrap();
        assert_eq!(identity, command);
        let mut expected = serde_json::to_value(saved.start.as_ref().unwrap()).unwrap();
        let fields = expected.as_object_mut().unwrap();
        fields.insert(
            "op".into(),
            json!(if kind == "start_account" {
                "resolve_start_account"
            } else {
                "resolve_start"
            }),
        );
        if kind == "start" {
            fields.insert("config_path".into(), Value::Null);
        }
        assert_eq!(serde_json::to_value(resolution).unwrap(), expected);
        assert_eq!(restored.turn, saved.turn);
        assert_eq!(restored.text, "retained task");
        assert!(!restored.attempted);
        if kind == "start_account" {
            for changed in ["host", "account", "model", "thinking", "service"] {
                let mut altered = restored.clone();
                if changed == "host" {
                    altered.account_host = None;
                } else {
                    let settings = altered.account_settings.as_mut().unwrap();
                    match changed {
                        "account" => settings.account.as_mut().unwrap().identity_generation += 1,
                        "model" => settings.model = "changed-model".into(),
                        "thinking" => settings.reasoning_effort = None,
                        _ => settings.service_tier = None,
                    }
                }
                assert!(
                    altered.validate_identity().is_err(),
                    "changed atomic selection cannot replace a frozen creation envelope"
                );
                assert_eq!(
                    serde_json::to_value(altered.start).unwrap(),
                    serde_json::to_value(restored.start.as_ref()).unwrap()
                );
            }
        }
    }
    let mut absent = empty_saved(fixture.0.path());
    assert!(absent.start_resolution().is_err());
    absent.start = Some(VesselCommand::Capabilities);
    assert!(absent.start_resolution().is_err());
}
