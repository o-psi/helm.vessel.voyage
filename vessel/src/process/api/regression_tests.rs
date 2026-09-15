use super::*;
use serde_json::json;
use uuid::Uuid;

// Public/private commands intentionally share field semantics, not wire tags.
// Exercise every scalar and nested payload rather than only checking enum kinds.
#[test]
fn translation_preserves_payloads_and_authority_across_command_families() {
    let id = Uuid::new_v4();
    let mut cases = vec![
        json!({"op":"prepare_browser"}),
        json!({"op":"snapshot"}),
        json!({"op":"decisions"}),
        json!({"op":"history", "offset":17,"limit":31,"expected_revision":9}),
        json!({"op":"provider_attempts","run_id":id,"offset":17,"limit":31,"expected_revision":9}),
        json!({"op":"message_chunk","index":4,"offset":17,"limit":31,"expected_revision":9}),
        json!({"op":"run_output","run_id":id,"offset":17,"limit":31}),
        json!({"op":"read_artifact","artifact_id":id,"offset":17,"limit":31}),
        json!({"op":"upload_image","upload_id":id,"name":"fixture.png","data_base64":"AA=="}),
        json!({"op":"receipt","command_id":id}),
        json!({"op":"events","after":13,"limit":11,"wait_ms":0}),
        json!({"op":"assignment_observe","run_id":id,"assignment_id":id,"participant":"peer","cancel":true}),
        json!({"op":"workflow_inputs","input_id":id,"values":[["name","value"]]}),
        json!({"op":"workflow_preview","id":"fixture","scope":"workspace","user_directory":"/fixture","inputs":[["x","y"]],"trust_digest":"digest","optional_secret_names":["optional"]}),
    ];
    cases.push(json!({"op":"browser","operation":{"operation":"pending","binding":{"session_id":id,"incarnation":id,"run_id":id,"browser_id":id,"resource_id":id,"executor_id":id,"controller_epoch":3,"capture_epoch":4,"expires_at_ms":987654},"limit":9}}));
    cases.push(json!({"op":"set_account_inference","command_id":id,"expected_revision":19,"expires_at_ms":123456789,"model":"fixture","reasoning_effort":"high","service_tier":"auto","account":{"account_id":id,"connection_id":id,"identity_generation":3,"connection_revision":7,"transport":"openai_responses"}}));
    for operation in [
        json!({"action":"attach"}),
        json!({"action":"snapshot"}),
        json!({"action":"write","bytes":[0,27,255]}),
        json!({"action":"resize","columns":120,"rows":40}),
    ] {
        cases.push(json!({"op":"terminal","run_id":id,"terminal_id":id,"operation":operation}));
    }
    for section in ["host_resources", "tools", "policy", "models"] {
        cases.push(json!({"op":"controls","run_id":id,"section":section}));
    }
    for (name, fields) in [
        ("clear", json!({"confirm_session_id":id})),
        ("compact", json!({"retain":3,"preserve_canonical":true})),
        (
            "operator_tool",
            json!({"name":"fixture","arguments":{"array":[1,true,"x"]}}),
        ),
        (
            "execute_tool",
            json!({"run_id":id,"name":"fixture","arguments":{"n":2}}),
        ),
        ("github", json!({"words":["issue","view","287"]})),
        ("set_access", json!({"access":"read_only"})),
        ("configure", json!({"config_path":"/fixture/config.toml"})),
        (
            "workflow_submit",
            json!({"id":"fixture","scope":"workspace","user_directory":"/fixture","inputs":[["x","y"]],"trust_digest":"digest","private_inputs_id":id}),
        ),
        (
            "submit",
            json!({"prompt":"offline prompt","coordination":null}),
        ),
        ("submit_content", json!({"content":[]})),
        ("cancel", json!({"run_id":id})),
        (
            "steer",
            json!({"run_id":id,"prompt":"offline steering","coordination":null}),
        ),
        ("rename", json!({"name":"new name"})),
        (
            "set_inference",
            json!({"model":"fixture","reasoning_effort":"high","service_tier":"auto"}),
        ),
        ("set_model", json!({"model":"fixture"})),
        ("archive", json!({"archived":true})),
        ("delete", json!({"confirm_session_id":id})),
        (
            "respond",
            json!({"run_id":id,"decision_id":id,"response":{"choice":"deny"}}),
        ),
    ] {
        let mut value = fields;
        value["op"] = json!(name);
        value["command_id"] = json!(id);
        value["expected_revision"] = json!(19);
        value["expires_at_ms"] = json!(123456789);
        cases.push(value);
    }
    for value in cases {
        let command: VoyageCommand =
            serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{value}: {e}"));
        let expected_right = required_right(&command);
        let public = serde_json::to_value(&command).unwrap();
        let translated = runtime(command).unwrap();
        assert_eq!(
            required_process_right(&translated),
            expected_right,
            "{value}"
        );
        assert_eq!(serde_json::to_value(translated).unwrap(), public, "{value}");
    }
}

#[test]
fn resolution_recurses_but_never_broadens_authority_for_mismatched_ids() {
    let id = Uuid::new_v4();
    let original = VoyageCommand::Rename {
        command_id: id,
        expected_revision: 7,
        expires_at_ms: 99,
        name: "name".into(),
    };
    let resolve = VoyageCommand::Resolve {
        command_id: id,
        original: Some(Box::new(original.clone())),
    };
    assert_eq!(required_right(&resolve), Some(ProcessRight::Lifecycle));
    match runtime(resolve).unwrap() {
        RuntimeCommand::Resolve {
            command_id,
            original: Some(original),
        } => {
            assert_eq!(command_id, id);
            assert!(matches!(
                *original,
                RuntimeCommand::Rename {
                    expected_revision: 7,
                    ..
                }
            ));
        }
        _ => panic!("resolution payload lost"),
    }
    assert_eq!(
        required_right(&VoyageCommand::Resolve {
            command_id: Uuid::new_v4(),
            original: Some(Box::new(original))
        }),
        None
    );
    assert_eq!(
        required_right(&VoyageCommand::Resolve {
            command_id: id,
            original: None
        }),
        None
    );
}

fn private_response() -> RuntimeResponse {
    RuntimeResponse {
        protocol: PROCESS_PROTOCOL,
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        resumed_from: None,
        error: None,
        outcome_unknown: false,
        result: json!({"accepted":false,"receipt":"fixture"}),
    }
}

#[test]
fn replies_keep_structured_failures_and_uncertainty_without_private_envelopes() {
    let r = private_response();
    let session = r.session_id;
    let incarnation = r.incarnation;
    let public = response(reply(r));
    assert!(public.error.is_none());
    assert!(!public.outcome_unknown);
    assert_eq!(public.result["session_id"], json!(session));
    assert_eq!(public.result["incarnation"], json!(incarnation));
    assert!(public.result.get("protocol").is_none());
    for unknown in [false, true] {
        let mut r = private_response();
        let result = r.result.clone();
        r.error = Some("rejected\n\t\0 safely".into());
        r.outcome_unknown = unknown;
        let public = response(reply(r));
        assert_eq!(public.result, result);
        assert_eq!(public.outcome_unknown, unknown);
        assert_eq!(public.error.as_deref(), Some("rejected safely"));
    }
    let mut r = private_response();
    r.outcome_unknown = true;
    let public = response(reply(r));
    assert!(public.outcome_unknown);
    assert!(public.result.is_null());
    let mut r = private_response();
    r.protocol = PROCESS_PROTOCOL + 1;
    assert!(
        reply(r)
            .unwrap_err()
            .to_string()
            .contains("unsupported private runtime protocol")
    );
    let public = response(Err(anyhow::anyhow!("é".repeat(700))));
    assert_eq!(public.error.unwrap().chars().count(), 512);
    assert!(public.result.is_null());
    assert!(!public.outcome_unknown);
}
