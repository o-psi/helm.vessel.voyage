//! Public-consumer tests: importing the codec must compile, not merely leave a
//! source file containing tests outside the module graph.
use serde_json::{Value, json};
use voyage_protocol::{
    attachment::{Command, MAX_FRAME_BYTES, MAX_PROMPT_BYTES},
    stream::Frame,
};

const ID: &str = "12345678-1234-4234-8234-123456789abc";
const OTHER: &str = "22345678-1234-4234-8234-123456789abc";
const NIL: &str = "00000000-0000-0000-0000-000000000000";
fn command() -> Value {
    json!({"type":"command","command":{"version":2,"connection_id":ID,
        "machine_id":ID,"principal_id":ID,"command_id":ID,"expires_at_ms":1000,
        "operation":{"type":"submit","session_id":ID,"expected_revision":0,"prompt":"hello 世界"}}})
}
fn auth() -> Value {
    json!({"type":"authenticate","version":2,"proof":{"challenge":{"version":2,"id":ID,
        "origin":"https://vessel.example","expires_at_ms":1000,"server_tag":vec![1u8;32],
        "operation":{"type":"connect","machine_id":ID,"epoch":1}},
        "signature":vec![2u8;64],"new_signature":null}})
}
fn session() -> Value {
    json!({"id":ID,"revision":0,"name":"Example 世界","model":"fixture","sharing":"metadata","archived":false})
}
fn result(reply: Value) -> Value {
    json!({"type":"result","connection_id":ID,"command_id":ID,"reply":reply})
}
fn history() -> Value {
    result(
        json!({"type":"history","session_id":ID,"entries":[{"role":"assistant","text":"evidence 世界"}],"next":null}),
    )
}
fn valid(value: &Value) {
    let decoded = Frame::decode(&serde_json::to_vec(value).unwrap()).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&decoded.encode().unwrap()).unwrap(),
        *value
    );
}
fn invalid(value: &Value) {
    assert!(
        Frame::decode(&serde_json::to_vec(value).unwrap()).is_err(),
        "accepted invalid frame: {value}"
    );
    if let Ok(frame) = serde_json::from_value::<Frame>(value.clone()) {
        assert!(frame.encode().is_err(), "encoded invalid frame: {value}");
    }
}
#[test]
fn every_frame_and_reply_roundtrips_through_public_codec() {
    let mut frames = vec![
        auth(),
        command(),
        json!({"type":"welcome","version":2,"connection_id":ID,"machine_id":ID,"owner_id":ID,"epoch":1,"lease_ms":1000}),
        json!({"type":"heartbeat","connection_id":ID}),
        json!({"type":"lease","connection_id":ID,"lease_ms":30000}),
        result(json!({"type":"sessions","sessions":[session()]})),
        result(json!({"type":"session","session":session()})),
        result(json!({"type":"accepted"})),
        history(),
    ];
    for state in [
        "accepted",
        "running",
        "completed",
        "incomplete",
        "cancelled",
        "failed",
        "interrupted",
    ] {
        frames.push(result(
            json!({"type":"run","session_id":ID,"run_id":ID,"state":state}),
        ));
    }
    for code in [
        "invalid_request",
        "unauthorized",
        "conflict",
        "not_found",
        "expired",
        "busy",
        "resource_exhausted",
        "snapshot_required",
        "internal",
    ] {
        frames.push(result(json!({"type":"denied","code":code})));
    }
    for value in frames {
        valid(&value);
    }
}
#[test]
fn heartbeat_has_stable_v2_wire_representation() {
    let golden = format!(r#"{{"type":"heartbeat","connection_id":"{ID}"}}"#);
    assert_eq!(
        Frame::decode(golden.as_bytes()).unwrap().encode().unwrap(),
        golden
    );
}
#[test]
fn nested_commands_receive_identity_operation_and_version_checks() {
    for (pointer, value) in [
        ("/command/version", json!(1)),
        ("/command/connection_id", json!(NIL)),
        ("/command/machine_id", json!(NIL)),
        ("/command/principal_id", json!(NIL)),
        ("/command/command_id", json!(NIL)),
        ("/command/expires_at_ms", json!(0)),
        ("/command/operation/session_id", json!(NIL)),
        ("/command/operation/expected_revision", json!(i64::MAX)),
        ("/command/operation/prompt", json!("")),
        ("/command/operation/prompt", json!("\u{0}")),
        (
            "/command/operation/prompt",
            json!("x".repeat(MAX_PROMPT_BYTES + 1)),
        ),
    ] {
        let mut v = command();
        *v.pointer_mut(pointer).unwrap() = value;
        invalid(&v);
    }
}
#[test]
fn envelope_structure_does_not_renew_or_admit_expired_commands() {
    for deadline in [1, 900_000] {
        let mut v = command();
        v["command"]["expires_at_ms"] = json!(deadline);
        valid(&v);
        let Frame::Command { command } = Frame::decode(&serde_json::to_vec(&v).unwrap()).unwrap()
        else {
            panic!()
        };
        assert!(command.validate(1000).is_err());
        assert!(Command::decode(&serde_json::to_vec(&command).unwrap(), 1000).is_err());
        assert_eq!(command.expires_at_ms, deadline);
    }
}
#[test]
fn authentication_is_connect_only_and_checks_proof_shape_before_crypto() {
    for (pointer, value) in [
        ("/version", json!(1)),
        ("/proof/challenge/version", json!(1)),
        ("/proof/challenge/id", json!(NIL)),
        ("/proof/challenge/origin", json!("")),
        ("/proof/challenge/origin", json!("x".repeat(2049))),
        ("/proof/challenge/origin", json!("https://vessel.example\n")),
        ("/proof/challenge/expires_at_ms", json!(0)),
        ("/proof/challenge/server_tag", json!(vec![1u8; 31])),
        ("/proof/signature", json!(vec![1u8; 63])),
        ("/proof/signature", json!(vec![1u8; 16385])),
        ("/proof/new_signature", json!(vec![1u8; 64])),
        ("/proof/challenge/operation/machine_id", json!(NIL)),
        ("/proof/challenge/operation/epoch", json!(0)),
        ("/proof/challenge/operation/epoch", json!(i64::MAX)),
        (
            "/proof/challenge/operation",
            json!({"type":"revoke","machine_id":ID,"epoch":1,"transaction_id":ID}),
        ),
    ] {
        let mut v = auth();
        *v.pointer_mut(pointer).unwrap() = value;
        invalid(&v);
    }
    // Structural decoding deliberately does not authenticate or extend expiry.
    let Frame::Authenticate { proof, .. } =
        Frame::decode(&serde_json::to_vec(&auth()).unwrap()).unwrap()
    else {
        panic!()
    };
    assert!(
        proof
            .challenge
            .validate("https://other.example", 0)
            .is_err()
    );
    assert!(
        proof
            .challenge
            .validate("https://vessel.example", 1001)
            .is_err()
    );
    assert!(proof.verify(&[0; 32], "https://vessel.example", 0).is_err());
}
#[test]
fn response_projections_reject_invalid_identity_bounds_and_untyped_status() {
    let values = [
        result(json!({"type":"run","session_id":NIL,"run_id":ID,"state":"running"})),
        result(json!({"type":"run","session_id":ID,"run_id":NIL,"state":"running"})),
        result(json!({"type":"run","session_id":ID,"run_id":ID,"state":"success probably"})),
        result(json!({"type":"denied","code":"private provider error sentinel"})),
        result(json!({"type":"history","session_id":NIL,"entries":[],"next":null})),
        result(json!({"type":"history","session_id":ID,"entries":[],"next":u64::MAX})),
        result(
            json!({"type":"history","session_id":ID,"entries":[{"role":"system","text":"runtime instructions"}],"next":null}),
        ),
        result(
            json!({"type":"history","session_id":ID,"entries":[{"role":"assistant","text":"\u{0}"}],"next":null}),
        ),
        result(json!({"type":"sessions","sessions":vec![session();101]})),
        result(json!({"type":"sessions","sessions":[session(),session()]})),
    ];
    for value in values {
        invalid(&value);
    }
    for (field, value) in [
        ("id", json!(NIL)),
        ("revision", json!(u64::MAX)),
        ("name", json!("")),
        ("name", json!("x".repeat(257))),
        ("name", json!("hidden\u{202e}")),
        ("model", json!("\n")),
    ] {
        let mut view = session();
        view[field] = value;
        invalid(&result(json!({"type":"session","session":view})));
    }
}
#[test]
fn frame_identity_and_lease_boundaries_apply_to_encode_too() {
    for lease in [0, 999, 30001, u32::MAX] {
        invalid(&json!({"type":"lease","connection_id":ID,"lease_ms":lease}));
    }
    for value in [
        json!({"type":"heartbeat","connection_id":NIL}),
        result(json!({"type":"accepted"})),
    ] {
        let mut v = value;
        v["connection_id"] = json!(NIL);
        invalid(&v);
    }
    let mut v = result(json!({"type":"accepted"}));
    v["command_id"] = json!(NIL);
    invalid(&v);
    let welcome = json!({"type":"welcome","version":2,"connection_id":ID,"machine_id":ID,"owner_id":ID,"epoch":1,"lease_ms":1000});
    for (field, value) in [
        ("version", json!(1)),
        ("machine_id", json!(NIL)),
        ("owner_id", json!(NIL)),
        ("epoch", json!(0)),
        ("epoch", json!(i64::MAX)),
        ("lease_ms", json!(30001)),
    ] {
        let mut v = welcome.clone();
        v[field] = value;
        invalid(&v);
    }
}
#[test]
fn strict_nested_unknown_duplicate_fields_and_legacy_shapes_fail_closed() {
    for mut v in [auth(), command(), history()] {
        v["unknown"] = json!(true);
        invalid(&v);
    }
    let mut v = command();
    v["command"]["operation"]["unknown"] = json!(true);
    invalid(&v);
    let mut v = auth();
    v["proof"]["challenge"]["unknown"] = json!(true);
    invalid(&v);
    let mut v = history();
    v["reply"]["entries"][0]["provider_state"] = json!({"secret":"sentinel"});
    invalid(&v);
    let text = serde_json::to_string(&command()).unwrap().replace(
        "\"expected_revision\":0",
        "\"expected_revision\":0,\"expected_revision\":1",
    );
    assert!(Frame::decode(text.as_bytes()).is_err());
    for text in [
        r#"{"type":"heartbeat","connection_id":"x","connection_id":"y"}"#,
        r#"{"type":"task","task_id":"old-v1"}"#,
        r#"{"type":"lease","connection_id":null,"lease_ms":1000}"#,
    ] {
        assert!(Frame::decode(text.as_bytes()).is_err());
    }
}
#[test]
fn count_byte_unicode_and_escaped_frame_limits_are_explicit() {
    let mut views = Vec::new();
    for index in 1..=100 {
        let mut view = session();
        view["id"] = json!(uuid::Uuid::from_u128(index).to_string());
        views.push(view);
    }
    valid(&result(json!({"type":"sessions","sessions":views})));
    let mut last_revision = session();
    last_revision["revision"] = json!(i64::MAX);
    valid(&result(json!({"type":"session","session":last_revision})));
    for role in ["user", "assistant", "tool"] {
        valid(&result(
            json!({"type":"history","session_id":ID,"entries":[{"role":role,"text":"text"}],"next":i64::MAX}),
        ));
    }
    let entry = json!({"role":"tool","text":""});
    let v = result(
        json!({"type":"history","session_id":ID,"entries":vec![entry.clone();100],"next":1}),
    );
    valid(&v);
    invalid(&result(
        json!({"type":"history","session_id":ID,"entries":vec![entry;101],"next":1}),
    ));
    let mut v = history();
    v["reply"]["entries"][0]["text"] = json!("é".repeat(MAX_PROMPT_BYTES / 2));
    valid(&v);
    v["reply"]["entries"][0]["text"] = json!("é".repeat(MAX_PROMPT_BYTES / 2) + "x");
    invalid(&v);
    // Text is within the entry byte limit, but escaping exceeds frame budget.
    let mut escaped = history();
    escaped["reply"]["entries"][0]["text"] = json!("\u{1}".repeat(MAX_PROMPT_BYTES));
    invalid(&escaped);
    let raw = format!(
        "{{\"type\":\"result\",\"connection_id\":\"{ID}\",\"command_id\":\"{ID}\",\"reply\":{{\"type\":\"history\",\"session_id\":\"{ID}\",\"entries\":[{{\"role\":\"tool\",\"text\":\"{}\"}}],\"next\":null}}}}",
        "\\u0001".repeat(MAX_PROMPT_BYTES)
    );
    assert!(Frame::decode(raw.as_bytes()).is_err());
    assert!(Frame::decode(&vec![b' '; MAX_FRAME_BYTES + 1]).is_err());
    // Exactly one frame limit including whitespace is accepted; one byte more is not.
    let mut exact = serde_json::to_vec(&json!({"type":"heartbeat","connection_id":OTHER})).unwrap();
    exact.resize(MAX_FRAME_BYTES, b' ');
    assert!(Frame::decode(&exact).is_ok());
    exact.push(b' ');
    assert!(Frame::decode(&exact).is_err());
}
#[test]
fn debug_and_errors_do_not_disclose_frame_content() {
    let mut v = command();
    v["command"]["operation"]["prompt"] = json!("sensitive-sentinel");
    let frame = Frame::decode(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(!format!("{frame:?}").contains("sentinel"));
    v["secret-sentinel"] = json!("sensitive-sentinel");
    let error = Frame::decode(&serde_json::to_vec(&v).unwrap()).unwrap_err();
    assert!(!format!("{error:?}").contains("sentinel"));
}

#[test]
fn inclusive_identity_label_epoch_and_prompt_limits_remain_usable() {
    let mut cmd = command();
    cmd["command"]["operation"]["expected_revision"] = json!(i64::MAX - 1);
    cmd["command"]["operation"]["prompt"] = json!("x".repeat(MAX_PROMPT_BYTES));
    valid(&cmd);
    let mut view = session();
    view["name"] = json!("é".repeat(128));
    view["model"] = json!("x".repeat(256));
    valid(&result(json!({"type":"session","session":view})));
    let mut proof = auth();
    proof["proof"]["challenge"]["operation"]["epoch"] = json!(i64::MAX - 1);
    proof["proof"]["challenge"]["origin"] = json!("x".repeat(2048));
    valid(&proof);
    valid(
        &json!({"type":"welcome","version":2,"connection_id":ID,"machine_id":ID,"owner_id":ID,
        "epoch":i64::MAX - 1,"lease_ms":30000}),
    );
}
