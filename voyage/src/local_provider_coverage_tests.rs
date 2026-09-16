use super::*;
use crate::github::http_fixture::{Fixture, Reply};

fn args(endpoint: String, transport: Transport) -> EndpointArgs {
    EndpointArgs {
        preset: Preset::Custom,
        endpoint: Some(endpoint),
        model: None,
        api_key_env: None,
        transport,
        chat_max_completion_tokens: false,
        timeout_secs: 5,
    }
}
fn completion(transport: Transport, body: Value) -> Reply {
    Reply {
        method: "POST",
        ..Reply::json(
            match transport {
                Transport::Chat => "/v1/chat/completions",
                Transport::Responses => "/v1/responses",
            },
            body,
        )
    }
}
fn discovered() -> Reply {
    Reply::json("/v1/models", json!({"data":[{"id":"offline-model"}]}))
}
fn success(transport: Transport) -> Value {
    match transport {
        Transport::Chat => json!({"choices":[{"message":{"role":"assistant","content":"OK"}}]}),
        Transport::Responses => json!({"id":"offline-response","output":[]}),
    }
}
fn request_body(request: &[u8]) -> Value {
    let start = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&request[start..]).unwrap()
}

#[tokio::test]
async fn discovery_and_tool_free_probe_obey_both_transport_contracts() {
    for transport in [Transport::Chat, Transport::Responses] {
        for modern in [false, true] {
            let fixture = Fixture::start(vec![
                discovered(),
                completion(transport, success(transport)),
            ])
            .await;
            let mut input = args(format!("http://{}/v1", fixture.address), transport);
            input.chat_max_completion_tokens = modern;
            let (config, report) = probe(&input).await.unwrap();
            assert_eq!(config.model, "offline-model");
            assert!(!config.api_key_required);
            assert_eq!(report.credential_source, "none");
            assert_eq!(report.discovery, "available");
            assert_eq!(report.models, vec!["offline-model"]);
            assert!(report.protocol_validated);
            let requests = fixture.finish().await;
            for request in &requests {
                assert!(
                    !String::from_utf8_lossy(request)
                        .to_lowercase()
                        .contains("authorization:")
                );
            }
            let body = request_body(&requests[1]);
            assert_eq!(body["model"], "offline-model");
            assert_eq!(body["stream"], false);
            assert!(body.get("tools").is_none());
            match transport {
                Transport::Chat => {
                    let field = if modern {
                        "max_completion_tokens"
                    } else {
                        "max_tokens"
                    };
                    assert_eq!(body[field], 16);
                    assert_eq!(body["messages"][0]["content"], "Reply OK.");
                }
                Transport::Responses => {
                    assert_eq!(body["store"], false);
                    assert_eq!(body["max_output_tokens"], 16);
                    assert_eq!(body["input"], "Reply OK.");
                }
            }
        }
    }
}

#[tokio::test]
async fn manual_model_survives_unavailable_discovery_but_implicit_model_does_not() {
    for code in [404, 405, 501] {
        let unavailable = || Reply {
            status: code,
            ..Reply::json("/v1/models", json!({}))
        };
        let fixture = Fixture::start(vec![
            unavailable(),
            completion(Transport::Chat, success(Transport::Chat)),
        ])
        .await;
        let mut input = args(format!("http://{}/v1", fixture.address), Transport::Chat);
        input.model = Some("manual".into());
        let (config, report) = probe(&input).await.unwrap();
        assert_eq!(config.model, "manual");
        assert_eq!(report.discovery, "unavailable_manual_model");
        assert!(report.models.is_empty());
        fixture.finish().await;
        let fixture = Fixture::start(vec![unavailable()]).await;
        let input = args(format!("http://{}/v1", fixture.address), Transport::Chat);
        assert!(
            probe(&input)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("supply --model")
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn incompatible_or_failed_completions_never_validate_a_protocol() {
    for transport in [Transport::Chat, Transport::Responses] {
        for value in [
            json!({}),
            json!({"choices":[]}),
            json!({"choices":[{"message":null}]}),
            json!({"id":7,"output":[]}),
        ] {
            let fixture = Fixture::start(vec![discovered(), completion(transport, value)]).await;
            let input = args(format!("http://{}/v1", fixture.address), transport);
            assert!(
                probe(&input)
                    .await
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("incompatible completion")
            );
            fixture.finish().await;
        }
        for code in [401, 403, 429, 500, 302] {
            let mut reply = completion(
                transport,
                json!({"private":"never disclose fixture response"}),
            );
            reply.status = code;
            let fixture = Fixture::start(vec![discovered(), reply]).await;
            let input = args(format!("http://{}/v1", fixture.address), transport);
            let error = probe(&input).await.err().unwrap().to_string();
            assert!(!error.contains("never disclose"));
            assert!(error.contains(if code == 401 || code == 403 {
                "authentication failure"
            } else {
                "protocol failure"
            }));
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn malformed_and_oversized_protocol_bodies_are_bounded() {
    for (body, expected) in [
        (b"not JSON".to_vec(), "invalid JSON"),
        (vec![b'x'; MAX_BODY + 1], "exceeds 1 MiB"),
    ] {
        let mut reply = completion(Transport::Chat, json!({}));
        reply.body = body;
        let fixture = Fixture::start(vec![discovered(), reply]).await;
        let input = args(format!("http://{}/v1", fixture.address), Transport::Chat);
        assert!(
            probe(&input)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains(expected)
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn setup_publishes_a_reloadable_private_config_and_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("local.toml");
    let fixture = Fixture::start(vec![
        discovered(),
        completion(Transport::Responses, success(Transport::Responses)),
    ])
    .await;
    let endpoint = args(
        format!("http://{}/v1", fixture.address),
        Transport::Responses,
    );
    run(Command::Setup {
        endpoint: endpoint.clone(),
        output: output.clone(),
    })
    .await
    .unwrap();
    fixture.finish().await;
    let bytes = std::fs::read(&output).unwrap();
    let config: Config = toml::from_str(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(config.model, "offline-model");
    assert!(!config.api_key_required);
    assert!(
        run(Command::Setup {
            endpoint,
            output: output.clone()
        })
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(output).unwrap(), bytes);
    assert!(publication(&root.path().join("missing/config")).is_err());
    run(Command::Presets).await.unwrap();
}

#[test]
fn endpoint_and_model_validation_are_explicit_and_side_effect_free() {
    for raw in [
        "http://127.0.0.1:80/v1/",
        "http://[::1]:8000/v1",
        "https://example.invalid/v1",
    ] {
        assert!(validate_endpoint(raw).is_ok(), "{raw}");
    }
    for raw in [
        "",
        "ftp://127.0.0.1/v1",
        "http://example.invalid/v1",
        "https://user:pass@example.invalid/v1",
        "https://example.invalid/v1?q=1",
        "https://example.invalid/v1#x",
        "https://example.invalid/\n",
    ] {
        assert!(validate_endpoint(raw).is_err(), "{raw}");
    }
    for id in ["", "  ", "bad\nmodel", &"x".repeat(513)] {
        assert!(validate_model(id).is_err());
    }
    for preset in PRESETS {
        let mut input = args(preset.endpoint().unwrap().into(), Transport::Chat);
        input.preset = preset;
        input.endpoint = None;
        assert_eq!(
            input.resolve().unwrap().base_url.as_deref(),
            preset.endpoint()
        );
    }
    assert!(Preset::Custom.endpoint().is_none());
    for name in ["", "1KEY", "KEY-NAME", &"A".repeat(129)] {
        let mut input = args("http://127.0.0.1/v1".into(), Transport::Chat);
        input.api_key_env = Some(name.into());
        assert!(input.resolve().is_err());
    }
    for transport in [Transport::Chat, Transport::Responses] {
        let mut input = args("http://127.0.0.1/v1".into(), transport);
        input.api_key_env = Some("OFFLINE_CREDENTIAL_REFERENCE".into());
        let config = input.resolve().unwrap();
        let value = diagnostics(&config);
        assert_eq!(
            value["credential_source"],
            "environment:OFFLINE_CREDENTIAL_REFERENCE"
        );
        assert_eq!(value["endpoint"], "http://127.0.0.1/v1");
    }
}

#[tokio::test]
async fn model_discovery_failure_never_dispatches_a_completion() {
    for body in [
        json!({}),
        json!({"data":{}}),
        json!({"data":[{}]}),
        json!({"data":[{"id":""}]}),
        json!({"data":[{"id":"bad\nmodel"}]}),
        json!({"data":[{"id":"x".repeat(513)}]}),
        json!({"data":vec![json!({"id":"m"});1025]}),
    ] {
        let fixture = Fixture::start(vec![Reply::json("/v1/models", body)]).await;
        let input = args(format!("http://{}/v1", fixture.address), Transport::Chat);
        assert!(probe(&input).await.is_err());
        assert_eq!(fixture.finish().await.len(), 1);
    }
    let fixture = Fixture::start(vec![Reply::json("/v1/models", json!({"data":[]}))]).await;
    let input = args(format!("http://{}/v1", fixture.address), Transport::Chat);
    assert!(
        probe(&input)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("supply --model")
    );
    fixture.finish().await;
}

#[tokio::test]
async fn explicit_model_overrides_discovery_and_probe_command_uses_same_contract() {
    let fixture = Fixture::start(vec![
        discovered(),
        completion(Transport::Chat, success(Transport::Chat)),
    ])
    .await;
    let mut input = args(format!("http://{}/v1", fixture.address), Transport::Chat);
    input.model = Some("explicit-model".into());
    run(Command::Probe(input)).await.unwrap();
    let requests = fixture.finish().await;
    assert_eq!(request_body(&requests[1])["model"], "explicit-model");
}

#[tokio::test]
async fn unsuccessful_setup_does_not_publish_partial_configuration() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("failed.toml");
    let fixture = Fixture::start(vec![discovered(), completion(Transport::Chat, json!({}))]).await;
    let input = args(format!("http://{}/v1", fixture.address), Transport::Chat);
    assert!(
        run(Command::Setup {
            endpoint: input,
            output: output.clone()
        })
        .await
        .is_err()
    );
    fixture.finish().await;
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn setup_rejects_dangling_symlink_before_any_probe() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("absent");
    let output = root.path().join("config.toml");
    std::os::unix::fs::symlink(&target, &output).unwrap();
    let input = args("http://127.0.0.1:1/v1".into(), Transport::Chat);
    assert!(
        run(Command::Setup {
            endpoint: input,
            output: output.clone()
        })
        .await
        .is_err()
    );
    assert_eq!(std::fs::read_link(output).unwrap(), target);
    assert!(!target.exists());
}
