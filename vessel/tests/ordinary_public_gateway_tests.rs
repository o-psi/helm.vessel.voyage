//! Actual current Vessel CLI + production public gateway/SSE journeys. Private
//! ordinary supervisors route only to scripted readonly Runtime IPC peers.
//! These are transport/authority contracts, not executing Voyage/native proof.
#![cfg(target_os = "linux")]

// Keep the actual entrypoint visible to the current-object coverage auditor.
const VESSEL: &str = env!("CARGO_BIN_EXE_vessel");
#[path = "ordinary_public_gateway/fixture.rs"]
mod fixture;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use fixture::*;
use reqwest::{Client, Response, StatusCode, header::HeaderMap};
use serde_json::{Value, json};
use std::{collections::HashSet, fs, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;
use voyage_protocol::{
    process::{ConnectionGrant, ProcessGrant, ProcessRight},
    vessel::{
        COMMAND_PATH, EVENTS_PATH, MAX_VESSEL_BODY, PAIR_CAPABILITIES_PATH, PAIR_PATH,
        TerminalAction, VesselCommand, VesselEventRequest, VoyageCommand, VoyageReply,
    },
};

fn client() -> Client {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .build()
        .unwrap()
}
async fn value(response: Response) -> Value {
    let bytes = response.bytes().await.unwrap();
    assert!(bytes.len() <= 1024 * 1024, "bounded public response");
    serde_json::from_slice(&bytes).unwrap()
}
fn read_reply(response: voyage_protocol::vessel::VesselResponse) -> VoyageReply {
    assert!(response.error.is_none() && !response.outcome_unknown);
    serde_json::from_value(response.result).unwrap()
}
async fn raw_headers(address: std::net::SocketAddr, request: &[u8], deadline: Duration) -> String {
    tokio::time::timeout(deadline, async {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream.write_all(request).await.unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(stream.read_u8().await.unwrap());
            assert!(bytes.len() <= 8192);
        }
        String::from_utf8(bytes).unwrap()
    })
    .await
    .unwrap()
}
fn status(headers: &str) -> u16 {
    headers.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[tokio::test]
async fn invalid_gateway_cli_configuration_refuses_before_database_or_listener() {
    let mut env = Environment::new();
    for (index, bind, origin, development) in [
        (0, "0.0.0.0:9480", ORIGIN, false),
        (1, "localhost:9480", ORIGIN, false),
        (2, "127.0.0.1:0", "http://127.0.0.1:9480", false),
        (3, "127.0.0.1:0", "https://fixture.invalid/path", false),
        (
            4,
            "127.0.0.1:0",
            "https://user:fixture-secret@fixture.invalid",
            false,
        ),
        (5, "127.0.0.1:0", "http://localhost:9480", true),
    ] {
        let database = env.root.join(format!("refused-{index}.sqlite3"));
        let mut args = vec![
            "--bind".into(),
            bind.into(),
            "--database".into(),
            database.to_string_lossy().into_owned(),
            "--process-directory".into(),
            env.directory.to_string_lossy().into_owned(),
            "--public-origin".into(),
            origin.into(),
        ];
        if development {
            args.push("--allow-insecure-loopback".into());
        }
        env.cli(&args, false).await;
        assert!(!database.exists());
        assert!(!env.directory.join("process-http.json").exists());
        assert!(
            fs::read_dir(env.directory.join("sessions"))
                .unwrap()
                .next()
                .is_none()
        );
    }
    env.assert_logs_private(&["fixture-secret"]);
    env.complete();
}

#[tokio::test]
async fn unconfigured_actual_gateway_keeps_health_ready_and_metrics_but_no_process_admission() {
    let mut env = Environment::new();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let args = vec![
        "--bind".into(),
        address.to_string(),
        "--database".into(),
        env.root.join("web.sqlite3").to_string_lossy().into_owned(),
    ];
    let mut owner = ChildOwner::spawn(&env, "observations-only", &args);
    let endpoint = format!("http://{address}");
    let client = client();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            assert!(owner.is_running());
            if let Ok(response) = client
                .get(format!("{endpoint}/health"))
                .timeout(Duration::from_millis(200))
                .send()
                .await
                && response.status() == StatusCode::OK
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    owner.owns_listener(address);
    for (path, expected) in [("/health", "ok"), ("/ready", "ready")] {
        let response = client
            .get(format!("{endpoint}{path}"))
            .timeout(WAIT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        security(&response);
        assert_eq!(value(response).await["status"], expected);
    }
    let response = client
        .get(format!("{endpoint}/metrics"))
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    security(&response);
    assert!(
        response
            .text()
            .await
            .unwrap()
            .ends_with("voyage_connectivity_enabled 0\n")
    );
    for path in [PAIR_CAPABILITIES_PATH, "/ui", "/v1/diagnostics"] {
        let response = client
            .get(format!("{endpoint}{path}"))
            .timeout(WAIT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        security(&response);
    }
    assert!(!env.directory.join("catalogue.sqlite3").exists());
    owner.terminate().await;
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    env.complete();
}

#[tokio::test]
async fn configured_actual_gateway_observations_preserve_supervisor_scope_and_operator_disabled_state()
 {
    let f = Fixture::new(0, false).await;
    for (path, expected) in [("/health", "ok"), ("/ready", "ready")] {
        let response = f
            .client
            .get(format!("{}{path}", f.endpoint))
            .timeout(WAIT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        security(&response);
        assert_eq!(value(response).await["status"], expected);
    }
    let response = f
        .client
        .get(format!("{}/metrics", f.endpoint))
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert!(
        response
            .text()
            .await
            .unwrap()
            .ends_with("voyage_connectivity_enabled 1\n")
    );
    for path in ["/ui", "/v1/diagnostics"] {
        let response = f
            .client
            .get(format!("{}{path}", f.endpoint))
            .bearer_auth(TOKEN)
            .timeout(WAIT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        security(&response);
    }
    let caps = f.response(VesselCommand::Capabilities).await;
    assert!(caps.error.is_none() && !caps.outcome_unknown);
    assert_eq!(caps.result["scope"], "workspaces");
    assert_eq!(caps.result["principal_id"], json!(f.grant.principal_id));
    assert_eq!(
        caps.result["rights"],
        json!(["catalogue", "observe", "history"])
    );
    assert_eq!(caps.result["remote_updates"], false);
    let catalogue = f.response(VesselCommand::Catalogue).await;
    assert!(catalogue.error.is_none() && catalogue.result.as_array().unwrap().is_empty());
    f.finish().await;
}

#[tokio::test]
async fn exact_operator_bearer_basic_duplicate_and_malformed_auth_never_become_process_authority() {
    let f = Fixture::new(0, true).await;
    for authorization in [
        None,
        Some("Bearer wrong".into()),
        Some("Basic !!!".into()),
        Some(format!("Basic {}", STANDARD.encode([0xff, 0xff]))),
        Some(format!("Basic {}", STANDARD.encode("missing-separator"))),
        Some(format!("Bearer {OPERATOR}")),
        Some(format!(
            "Basic {}",
            STANDARD.encode(format!("ordinary:{OPERATOR}"))
        )),
    ] {
        let accepted = authorization.as_ref().is_some_and(|v| {
            v == &format!("Bearer {OPERATOR}")
                || v == &format!("Basic {}", STANDARD.encode(format!("ordinary:{OPERATOR}")))
        });
        for path in ["/ui", "/v1/diagnostics"] {
            let mut request = f.client.get(format!("{}{path}", f.endpoint));
            if let Some(value) = &authorization {
                request = request.header("Authorization", value);
            }
            let response = request.timeout(WAIT).send().await.unwrap();
            assert_eq!(
                response.status(),
                if accepted {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                }
            );
            security(&response);
            if !accepted {
                assert!(
                    response.headers()["www-authenticate"]
                        .to_str()
                        .unwrap()
                        .contains("Vessel operator")
                );
            }
            let body = response.text().await.unwrap();
            assert!(body.len() < 8192 && !body.contains(OPERATOR) && !body.contains(TOKEN));
            if accepted && path == "/v1/diagnostics" {
                let data: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(data["process_gateway"], "configured");
                assert_eq!(data["version"], env!("CARGO_PKG_VERSION"));
                assert_eq!(data.as_object().unwrap().len(), 2);
            }
        }
    }
    let request = format!(
        "GET /v1/diagnostics HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {OPERATOR}\r\nAuthorization: Bearer {OPERATOR}\r\nConnection: close\r\n\r\n",
        f.address
    );
    assert_eq!(
        status(&raw_headers(f.address, request.as_bytes(), WAIT).await),
        401
    );
    let response = f
        .client
        .post(format!("{}{}", f.endpoint, COMMAND_PATH))
        .bearer_auth(OPERATOR)
        .json(&envelope(VesselCommand::Capabilities))
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    f.finish().await;
}

#[tokio::test]
async fn correlation_bounds_and_unmatched_path_keep_private_inputs_out_of_actual_cli_logs() {
    let f = Fixture::new(0, false).await;
    for supplied in [
        "fixture_request-17".to_owned(),
        "x".repeat(129),
        "private/input?canary".to_owned(),
    ] {
        let response = f
            .client
            .get(format!("{}/health", f.endpoint))
            .header("x-request-id", &supplied)
            .timeout(WAIT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        security(&response);
        let returned = response.headers()["x-request-id"].to_str().unwrap();
        if supplied == "fixture_request-17" {
            assert_eq!(returned, supplied);
        } else {
            assert!(Uuid::parse_str(returned).is_ok() && returned != supplied);
        }
    }
    let marker = "private-path-fixture-canary";
    let response = f
        .client
        .get(format!("{}/{marker}?private-query-canary", f.endpoint))
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    security(&response);
    f.env
        .assert_logs_private(&[marker, "private-query-canary", "private/input?canary"]);
    f.finish().await;
}

#[tokio::test]
async fn real_route_body_bounds_and_stalled_receive_refuse_before_pairing_or_command_admission() {
    let f = Fixture::new(0, false).await;
    let before = read_private(&f.grant_path(), 16384);
    for (path, limit) in [
        (PAIR_PATH, 4096),
        ("/v1/vessel/browser-credentials", 4096),
        (COMMAND_PATH, MAX_VESSEL_BODY),
        (EVENTS_PATH, MAX_VESSEL_BODY),
    ] {
        let response = f
            .client
            .post(format!("{}{path}", f.endpoint))
            .header("Content-Type", "application/json")
            .body(vec![b'x'; limit + 1])
            .timeout(WAIT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        security(&response);
    }
    let request = format!(
        "POST {PAIR_PATH} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n{{",
        f.address
    );
    let headers = raw_headers(f.address, request.as_bytes(), Duration::from_secs(13)).await;
    assert_eq!(status(&headers), 408);
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("cache-control: no-store")
    );
    assert_eq!(read_private(&f.grant_path(), 16384), before);
    let after = f.response(VesselCommand::Capabilities).await;
    assert!(after.error.is_none() && !after.outcome_unknown);
    f.finish().await;
}

#[tokio::test]
async fn public_pairing_discovery_is_identity_only_and_readonly_when_service_routing_retires() {
    let mut f = Fixture::new(0, false).await;
    let response = f
        .client
        .get(format!("{}{}", f.endpoint, PAIR_CAPABILITIES_PATH))
        .header("Origin", ORIGIN)
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    security(&response);
    let before = value(response).await;
    assert_eq!(before["protocol"], 1);
    assert_eq!(before["vessel_id"], json!(f.grant.vessel_id));
    assert_eq!(before.as_object().unwrap().len(), 3);
    assert!(
        !before
            .to_string()
            .contains(f.env.workspace.to_str().unwrap())
    );
    f.stop_service().await;
    let response = f
        .client
        .get(format!("{}{}", f.endpoint, PAIR_CAPABILITIES_PATH))
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(value(response).await, before);
    f.finish().await;
}

#[tokio::test]
async fn exact_public_pairing_retry_preserves_one_credential_private_ciphertext_and_no_execution_intent()
 {
    let mut f = Fixture::new(0, false).await;
    let invitation = f.pair_invitation().await;
    let request = Fixture::pairing_request(&invitation, Uuid::new_v4());
    let first = f.pair(&request).await;
    assert!(first.error.is_none() && !first.outcome_unknown);
    let grant = first.result["grant_id"].as_str().unwrap();
    let token = first.result["token"].as_str().unwrap();
    assert_eq!(first.result["principal_id"], invitation["principal_id"]);
    assert_eq!(first.result["vessel_id"], invitation["vessel_id"]);
    let path = f
        .env
        .directory
        .join("access/connections")
        .join(format!("{grant}.json"));
    let saved = read_private(&path, 16384);
    let state_path = f.env.directory.join("access/pairing/state.json");
    let ciphertext = read_private(&state_path, 16 * 1024 * 1024);
    assert!(voyage_storage::credentials::encrypted(&ciphertext));
    for private in [token, invitation["code"].as_str().unwrap()] {
        assert!(
            !ciphertext
                .windows(private.len())
                .any(|w| w == private.as_bytes())
        );
        assert!(
            !saved
                .windows(private.len())
                .any(|w| w == private.as_bytes())
        );
    }
    let again = f.pair(&request).await;
    assert!(again.error.is_none() && !again.outcome_unknown && again.result == first.result);
    assert_eq!(read_private(&path, 16384), saved);
    assert_eq!(read_private(&state_path, 16 * 1024 * 1024), ciphertext);
    assert_eq!(
        fs::read_dir(f.env.directory.join("access/connections"))
            .unwrap()
            .count(),
        2
    );
    f.env
        .assert_logs_private(&[token, invitation["code"].as_str().unwrap()]);
    f.finish().await;
}

#[tokio::test]
async fn public_pairing_binding_conflicts_refuse_without_replacing_original_credential() {
    let mut f = Fixture::new(0, false).await;
    let invitation = f.pair_invitation().await;
    let request = Fixture::pairing_request(&invitation, Uuid::new_v4());
    let first = f.pair(&request).await;
    assert!(first.error.is_none() && !first.outcome_unknown);
    let path = f.env.directory.join("access/connections").join(format!(
        "{}.json",
        first.result["grant_id"].as_str().unwrap()
    ));
    let saved = read_private(&path, 16384);
    for field in ["principal_id", "command_id", "invitation_id", "code"] {
        let mut changed = request.clone();
        changed[field] = if field == "code" {
            json!("0".repeat(64))
        } else {
            json!(Uuid::new_v4())
        };
        let refused = f.pair(&changed).await;
        assert!(refused.error.is_some() && !refused.outcome_unknown && refused.result.is_null());
        assert_eq!(read_private(&path, 16384), saved);
    }
    let wrong_host = f
        .client
        .post(format!("{}{}", f.endpoint, PAIR_PATH))
        .header("x-voyage-vessel", Uuid::new_v4().to_string())
        .header("Origin", ORIGIN)
        .json(&request)
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_host.status(), StatusCode::OK);
    let wrong_host: voyage_protocol::vessel::VesselResponse =
        serde_json::from_value(value(wrong_host).await).unwrap();
    assert!(wrong_host.error.is_some() && !wrong_host.outcome_unknown);
    let wrong_origin = f
        .client
        .post(format!("{}{}", f.endpoint, PAIR_PATH))
        .header("Origin", "https://foreign.invalid")
        .json(&request)
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);
    let retry = f.pair(&request).await;
    assert!(retry.error.is_none() && retry.result == first.result);
    assert_eq!(read_private(&path, 16384), saved);
    f.finish().await;
}

#[tokio::test]
async fn normal_fixture_revocation_cli_removes_pairing_recovery_authority_without_replaying_or_creating()
 {
    let mut f = Fixture::new(0, false).await;
    let invitation = f.pair_invitation().await;
    let request = Fixture::pairing_request(&invitation, Uuid::new_v4());
    let first = f.pair(&request).await;
    assert!(first.error.is_none());
    let grant = first.result["grant_id"].as_str().unwrap();
    let args = vec![
        "revoke-connection".into(),
        "--directory".into(),
        f.env.directory.to_string_lossy().into_owned(),
        "--grant".into(),
        grant.into(),
        "--expected-revision".into(),
        "1".into(),
        "--command-id".into(),
        Uuid::new_v4().to_string(),
    ];
    f.env.cli(&args, true).await;
    let path = f
        .env
        .directory
        .join("access/connections")
        .join(format!("{grant}.json"));
    let retained = read_private(&path, 16384);
    let saved: ConnectionGrant = serde_json::from_slice(&retained).unwrap();
    assert!(saved.revoked && saved.revision == 2);
    let refused = f.pair(&request).await;
    assert!(refused.error.is_some() && !refused.outcome_unknown && refused.result.is_null());
    assert_eq!(read_private(&path, 16384), retained);
    assert_eq!(
        fs::read_dir(f.env.directory.join("access/connections"))
            .unwrap()
            .count(),
        2
    );
    f.finish().await;
}

#[tokio::test]
async fn readonly_http_history_receipt_controls_and_attention_preserve_exact_scope_and_payload() {
    let f = Fixture::new(1, false).await;
    let p = &f.peers[0].registration;
    let command = Uuid::new_v4();
    let history = read_reply(
        f.voyage(
            0,
            VoyageCommand::History {
                offset: 7,
                limit: 3,
                expected_revision: Some(17),
            },
        )
        .await,
    );
    assert_eq!(history.session_id, p.session_id);
    assert_eq!(history.incarnation, p.incarnation);
    assert_eq!(history.result["messages"][0]["message_index"], 7);
    assert_eq!(
        history.result["messages"][0]["content"],
        "Canonical synthetic history Ω"
    );
    let receipt = read_reply(
        f.voyage(
            0,
            VoyageCommand::Receipt {
                command_id: command,
            },
        )
        .await,
    );
    assert_eq!(receipt.result["command_id"], json!(command));
    assert_eq!(receipt.result["status"], "unknown");
    let controls = read_reply(
        f.voyage(
            0,
            VoyageCommand::Controls {
                run_id: None,
                section: "terminals".into(),
            },
        )
        .await,
    );
    assert_eq!(controls.result["section"], "terminals");
    let attention = f.response(attention()).await;
    assert!(attention.error.is_none() && !attention.outcome_unknown);
    assert_eq!(attention.result["available"], 0);
    let history_calls = f.peers[0].calls("history");
    assert_eq!(history_calls.len(), 1);
    assert_eq!(
        history_calls[0]["command"],
        json!({"op":"history","offset":7,"limit":3,"expected_revision":17})
    );
    let binding = authorization(&history_calls[0], &f.grant);
    for op in ["history", "receipt", "controls"] {
        for call in f.peers[0].calls(op) {
            let actual = authorization(&call, &f.grant);
            assert_eq!(actual.grant_id, binding.grant_id);
        }
    }
    let derived: ProcessGrant = serde_json::from_slice(&read_private(
        &f.env
            .directory
            .join("access/grants")
            .join(format!("{}.json", binding.grant_id)),
        16384,
    ))
    .unwrap();
    assert_eq!(derived.session_id, p.session_id);
    assert_eq!(derived.workspace, f.env.workspace);
    assert_eq!(
        derived.rights,
        vec![ProcessRight::Observe, ProcessRight::History]
    );
    assert_eq!(
        derived.connection_binding.unwrap().grant_id,
        f.grant.grant_id
    );
    f.finish().await;
}

#[tokio::test]
async fn missing_history_cross_workspace_and_mutative_http_controls_refuse_before_ipc_or_intent() {
    let mut f = Fixture::new(1, false).await;
    f.change_grant(|g| g.rights.retain(|r| *r != ProcessRight::History));
    for command in [
        VoyageCommand::History {
            offset: 0,
            limit: 1,
            expected_revision: None,
        },
        VoyageCommand::Receipt {
            command_id: Uuid::new_v4(),
        },
        VoyageCommand::Controls {
            run_id: None,
            section: "terminals".into(),
        },
        VoyageCommand::Terminal {
            run_id: Uuid::new_v4(),
            terminal_id: Uuid::new_v4(),
            operation: TerminalAction::Write { bytes: vec![65] },
        },
    ] {
        let response = f.voyage(0, command).await;
        assert!(response.error.is_some() && !response.outcome_unknown);
    }
    for op in ["history", "receipt", "controls", "terminal"] {
        assert!(f.peers[0].calls(op).is_empty());
    }
    f.change_grant(|g| {
        g.rights.push(ProcessRight::History);
        g.workspaces.clear();
    });
    let response = f.voyage(0, VoyageCommand::Snapshot).await;
    assert!(response.error.is_some() && !response.outcome_unknown);
    assert!(f.peers[0].calls("snapshot").is_empty());
    f.finish().await;
}

#[tokio::test]
async fn actual_http_auth_protocol_host_and_private_envelope_refusals_do_not_reach_runtime() {
    let f = Fixture::new(1, false).await;
    let body = envelope(VesselCommand::Capabilities);
    for case in 0..6 {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
        headers.insert(
            "x-voyage-grant",
            f.grant.grant_id.to_string().parse().unwrap(),
        );
        headers.insert(
            "x-voyage-vessel",
            f.grant.vessel_id.to_string().parse().unwrap(),
        );
        let mut body = body.clone();
        match case {
            0 => {
                headers.append("authorization", format!("Bearer {TOKEN}").parse().unwrap());
            }
            1 => {
                headers.append(
                    "x-voyage-grant",
                    f.grant.grant_id.to_string().parse().unwrap(),
                );
            }
            2 => {
                headers.insert("authorization", "Bearer short".parse().unwrap());
            }
            3 => {
                headers.insert("x-voyage-vessel", Uuid::nil().to_string().parse().unwrap());
            }
            4 => {
                headers.insert("x-voyage-grant", "not-a-uuid".parse().unwrap());
            }
            _ => {
                body["protocol"] = json!(99);
            }
        }
        let response = f
            .client
            .post(format!("{}{}", f.endpoint, COMMAND_PATH))
            .headers(headers)
            .json(&body)
            .timeout(WAIT)
            .send()
            .await
            .unwrap();
        assert!(matches!(
            response.status(),
            StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED
        ));
        security(&response);
    }
    let response = f
        .client
        .post(format!("{}{}", f.endpoint, COMMAND_PATH))
        .bearer_auth(TOKEN)
        .header("x-voyage-grant", f.grant.grant_id.to_string())
        .header("x-voyage-vessel", Uuid::new_v4().to_string())
        .json(&body)
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let refusal: voyage_protocol::vessel::VesselResponse =
        serde_json::from_value(value(response).await).unwrap();
    assert!(refusal.error.is_some() && !refusal.outcome_unknown);
    // The public handler rejects an internal Granted envelope as HTTP400, not a
    // successful command response; use the literal route to observe that gate.
    let internal = envelope(VesselCommand::Granted {
        expected_authority_fingerprint: None,
        expected_vessel_id: None,
        grant_id: f.grant.grant_id,
        token: TOKEN.into(),
        command: Box::new(VesselCommand::Capabilities),
    });
    let response = f
        .authorized(COMMAND_PATH)
        .json(&internal)
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        f.peers[0]
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|c| c["command"]["op"] == "health")
    );
    f.finish().await;
}

#[tokio::test]
async fn actual_sse_resume_cursor_advances_only_observed_pages_and_does_not_repeat_empty_history() {
    let f = Fixture::new(1, false).await;
    let mut stream = Stream::new(f.sse(vec![f.subscription(0, 10)]).await);
    let first = stream.next().await;
    assert!(first.error.is_none() && !first.outcome_unknown);
    assert_eq!(first.session_id, f.peers[0].registration.session_id);
    assert_eq!(first.incarnation, f.peers[0].registration.incarnation);
    assert_eq!(first.result["events"][0]["cursor"], 11);
    until(|| f.peers[0].calls("events").len() >= 2).await;
    let calls = f.peers[0].calls("events");
    assert_eq!(calls[0]["command"]["after"], 10);
    assert_eq!(calls[1]["command"]["after"], 11);
    authorization(&calls[0], &f.grant);
    f.peers[0].state.lock().unwrap().cursor = 12;
    let second = stream.next().await;
    assert_eq!(second.result["events"].as_array().unwrap().len(), 1);
    assert_eq!(second.result["events"][0]["cursor"], 12);
    assert!(second.error.is_none() && !second.outcome_unknown);
    drop(stream);
    f.finish().await;
}

#[tokio::test]
async fn actual_sse_stale_requested_owner_marks_gap_and_current_owner_without_new_execution() {
    let f = Fixture::new(1, false).await;
    let mut subscription = f.subscription(0, 10);
    subscription.incarnation = Uuid::new_v4();
    let mut stream = Stream::new(f.sse(vec![subscription]).await);
    let event = stream.next().await;
    assert_eq!(event.incarnation, f.peers[0].registration.incarnation);
    assert_eq!(event.result["owner_changed"], true);
    assert_eq!(event.result["replay_gap"], true);
    assert_eq!(event.result["recovery"], "snapshot");
    assert!(event.error.is_none() && !event.outcome_unknown);
    assert_eq!(f.peers[0].calls("events").len(), 1);
    drop(stream);
    f.finish().await;
}

#[tokio::test]
async fn sse_revocation_expiry_and_observe_right_loss_terminalize_before_another_runtime_read() {
    for case in 0..3 {
        let mut f = Fixture::new(1, false).await;
        let mut stream = Stream::new(f.sse(vec![f.subscription(0, 10)]).await);
        assert!(stream.next().await.error.is_none());
        let before = f.peers[0].calls("events").len();
        f.change_grant(|g| match case {
            0 => g.revoked = true,
            1 => g.expires_at_ms = now() - 1,
            _ => g.rights.retain(|r| *r != ProcessRight::Observe),
        });
        let terminal = stream.next().await;
        assert!(terminal.error.is_some() && !terminal.outcome_unknown);
        assert!(terminal.result.is_null());
        assert_eq!(f.peers[0].calls("events").len(), before);
        stream.ended().await;
        drop(stream);
        f.finish().await;
    }
}

#[tokio::test]
async fn sse_admission_enforces_auth_shape_and_serves_every_one_of_thirty_two_owned_sessions() {
    let f = Fixture::new(32, false).await;
    for subscriptions in [
        vec![],
        vec![f.subscription(0, 10), f.subscription(0, 10)],
        (0..33)
            .map(|i| {
                if i < 32 {
                    f.subscription(i, 10)
                } else {
                    let mut s = f.subscription(0, 10);
                    s.session_id = Uuid::new_v4();
                    s
                }
            })
            .collect(),
    ] {
        let response = f.sse(subscriptions).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let request = VesselEventRequest {
        protocol: 1,
        subscriptions: vec![f.subscription(0, 10)],
    };
    let response = f
        .client
        .post(format!("{}{}", f.endpoint, EVENTS_PATH))
        .json(&request)
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = f
        .client
        .post(format!("{}{}", f.endpoint, EVENTS_PATH))
        .bearer_auth("short")
        .header("x-voyage-grant", f.grant.grant_id.to_string())
        .json(&request)
        .timeout(WAIT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let mut stream = Stream::new(
        f.sse((0..32).map(|i| f.subscription(i, 10)).collect())
            .await,
    );
    let mut observed = HashSet::new();
    for _ in 0..32 {
        let event = stream.next().await;
        assert!(event.error.is_none() && !event.outcome_unknown);
        let peer = f
            .peers
            .iter()
            .find(|p| p.registration.session_id == event.session_id)
            .unwrap();
        assert_eq!(peer.registration.incarnation, event.incarnation);
        assert!(observed.insert(event.session_id));
    }
    assert_eq!(observed.len(), 32);
    drop(stream);
    f.finish().await;
}

#[tokio::test]
async fn sixty_four_live_sse_bodies_keep_command_lane_available_and_released_capacity_is_observed()
{
    let f = Fixture::new(1, false).await;
    let mut streams = Vec::new();
    for _ in 0..64 {
        let mut stream = Stream::new(f.sse(vec![f.subscription(0, 10)]).await);
        let event = stream.next().await;
        assert!(event.error.is_none() && !event.outcome_unknown);
        assert_eq!(event.session_id, f.peers[0].registration.session_id);
        assert_eq!(event.incarnation, f.peers[0].registration.incarnation);
        assert_eq!(event.result["events"][0]["cursor"], 11);
        streams.push(stream);
    }
    let response = f.sse(vec![f.subscription(0, 10)]).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let command = f.response(VesselCommand::Capabilities).await;
    assert!(command.error.is_none() && !command.outcome_unknown);
    let retirement = f.peers[0].state.clone();
    f.peers[0].expect_sse_body_drops(11, 65);
    drop(streams.pop());
    let response = tokio::time::timeout(WAIT, async {
        loop {
            let response = f.sse(vec![f.subscription(0, 10)]).await;
            if response.status() == StatusCode::OK {
                break response;
            }
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut recovered = Stream::new(response);
    let event = recovered.next().await;
    assert!(event.error.is_none() && !event.outcome_unknown);
    assert_eq!(event.session_id, f.peers[0].registration.session_id);
    assert_eq!(event.incarnation, f.peers[0].registration.incarnation);
    assert_eq!(event.result["events"][0]["cursor"], 11);
    drop(recovered);
    drop(streams);
    f.finish().await;
    retirement
        .lock()
        .unwrap()
        .assert_sse_body_drop_retirement(11, 65);
}

#[tokio::test]
async fn sse_private_reply_identity_decode_loss_and_unknown_refusals_end_without_retry_or_disclosure()
 {
    for fault in [
        Fault::WrongSession,
        Fault::Malformed,
        Fault::LostReply,
        Fault::Unknown,
        Fault::Refused,
    ] {
        let f = Fixture::new(1, false).await;
        f.peers[0].state.lock().unwrap().fault = fault;
        let mut stream = Stream::new(f.sse(vec![f.subscription(0, 10)]).await);
        let terminal = stream.next().await;
        assert_eq!(terminal.session_id, f.peers[0].registration.session_id);
        assert_eq!(terminal.incarnation, f.peers[0].registration.incarnation);
        assert!(terminal.error.is_some());
        assert_eq!(terminal.outcome_unknown, fault != Fault::Refused);
        if fault != Fault::Refused {
            assert!(terminal.result.is_null());
        }
        assert_eq!(f.peers[0].calls("events").len(), 1);
        stream.ended().await;
        drop(stream);
        f.finish().await;
    }
}

#[tokio::test]
async fn retired_actual_supervisor_routes_return_explicit_http_and_sse_unknown_without_restarting()
{
    let mut f = Fixture::new(1, false).await;
    let before = f.peers[0].calls("events").len();
    f.stop_service().await;
    let response = f.response(VesselCommand::Capabilities).await;
    assert!(response.error.is_some() && response.outcome_unknown && response.result.is_null());
    let mut stream = Stream::new(f.sse(vec![f.subscription(0, 10)]).await);
    let terminal = stream.next().await;
    assert!(terminal.error.is_some() && terminal.outcome_unknown && terminal.result.is_null());
    stream.ended().await;
    assert_eq!(f.peers[0].calls("events").len(), before);
    assert!(!f.env.directory.join("process-http.json").exists());
    drop(stream);
    f.finish().await;
}
