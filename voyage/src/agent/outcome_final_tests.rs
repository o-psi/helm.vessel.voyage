//! Parent wiring in agent.rs: #[cfg(test)]
//! #[path = "agent/outcome_final_tests.rs"] mod outcome_final_tests;
//! Provider-output boundaries only: no provider or tool is executed.
use super::{reasoning_preview, tool_preview, tool_replay};
use crate::{
    model::{Message, Role, ToolCall},
    provider::ProviderDelta,
    tools::Redactor,
};
use serde_json::json;
use uuid::Uuid;

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}
fn assistant(calls: Vec<ToolCall>) -> Message {
    let mut message = Message::new(Role::Assistant, "tool proposal");
    message.tool_calls = calls;
    message
}

#[test]
fn interrupted_calls_are_unknown_not_success_and_never_reexecuted() {
    let original = vec![
        Message::new(Role::User, "write once"),
        assistant(vec![call(
            "write",
            "write_file",
            json!({"path":"result","content":"x"}),
        )]),
    ];
    let mut projected = original.clone();
    tool_replay::project_interrupted_calls(&mut projected);
    assert_eq!(original.len(), 2);
    assert_eq!(projected.len(), 3);
    let result = &projected[2];
    assert_eq!(result.role, Role::Tool);
    assert_eq!(result.tool_call_id.as_deref(), Some("write"));
    assert!(result.content.contains("outcome is unknown"));
    assert!(result.content.contains("may have taken effect"));
    assert!(result.content.contains("has not replayed"));
    assert!(result.tool_calls.is_empty());
    assert_eq!(
        result.tool_outcome.as_ref().unwrap().execution,
        voyage_protocol::tool_result::ExecutionOutcome::Unknown
    );
    tool_replay::project_interrupted_calls(&mut projected);
    assert_eq!(projected.len(), 3);
}

#[test]
fn interrupted_projection_keeps_observed_results_and_repairs_only_missing_siblings() {
    let mut history = vec![
        assistant(vec![
            call("one", "shell", json!({})),
            call("two", "shell", json!({})),
        ]),
        Message::new(Role::System, "runtime boundary"),
        Message::tool_result("one", "observed failure", false),
        Message::new(Role::User, "continue"),
    ];
    tool_replay::project_interrupted_calls(&mut history);
    assert_eq!(history.len(), 5);
    assert_eq!(history[1].role, Role::System);
    assert_eq!(history[2].content, "observed failure");
    assert_eq!(history[3].tool_call_id.as_deref(), Some("two"));
    assert_eq!(history[4].role, Role::User);
}

#[test]
fn repeated_call_ids_in_separate_responses_are_not_cross_response_deduplicated() {
    let mut history = vec![
        assistant(vec![call("reused", "shell", json!({"command":"first"}))]),
        Message::tool_result("reused", "first observed", false),
        assistant(vec![call("reused", "shell", json!({"command":"second"}))]),
    ];
    tool_replay::project_interrupted_calls(&mut history);
    assert_eq!(history.len(), 4);
    assert_eq!(history[1].content, "first observed");
    assert!(history[3].content.contains("unknown"));
    assert_eq!(history[3].tool_call_id.as_deref(), Some("reused"));
}

#[test]
fn batch_normalization_is_atomic_even_when_invalid_call_is_last() {
    for bad in [
        call("", "shell", json!({})),
        call("a\n", "shell", json!({})),
        call(&"a".repeat(1025), "shell", json!({})),
        call("b", " ", json!({})),
        call("b", "shell\t", json!({})),
    ] {
        let mut calls = vec![
            call("same", "shell", json!({})),
            call("same", "shell", json!({})),
            bad,
        ];
        let before = serde_json::to_value(&calls).unwrap();
        assert!(tool_replay::normalize(&mut calls).is_err());
        assert_eq!(serde_json::to_value(&calls).unwrap(), before);
    }
}

#[test]
fn identical_duplicates_collapse_but_conflicting_effects_refuse_the_batch() {
    let mut calls = vec![
        call("a", "shell", json!({"command":"one"})),
        call("b", "read_file", json!({"path":"x"})),
        call("a", "shell", json!({"command":"one"})),
    ];
    tool_replay::normalize(&mut calls).unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].id, "a");
    assert_eq!(calls[1].id, "b");
    for conflict in [
        call("a", "write_file", json!({"command":"one"})),
        call("a", "shell", json!({"command":"two"})),
    ] {
        let mut calls = vec![call("a", "shell", json!({"command":"one"})), conflict];
        assert!(tool_replay::normalize(&mut calls).is_err());
        assert_eq!(calls.len(), 2);
    }
}

fn delta(index: usize, name: &str, arguments: &str) -> ProviderDelta {
    ProviderDelta::ToolCall {
        index,
        id: Some(format!("id-{index}")),
        name: Some(name.into()),
        arguments: arguments.into(),
    }
}

#[test]
fn executable_sensitive_tools_never_stream_arguments() {
    let redactor = Redactor::new(Vec::<String>::new());
    for name in [
        "process",
        "workflow",
        "browser",
        "custom_extension",
        "vessel",
    ] {
        let mut previews = tool_preview::Previews::default();
        previews.update(&delta(0, name, r#"{"password":"private-input"}"#));
        let public = previews.public(Uuid::nil(), &redactor);
        assert_eq!(public.len(), 1);
        let encoded = serde_json::to_string(&public).unwrap();
        assert!(!encoded.contains("private-input"));
        assert!(encoded.contains("withheld"));
    }
}

#[test]
fn safe_tool_preview_redacts_decoded_escapes_and_unfinished_secret_prefixes() {
    let redactor = Redactor::new(vec!["private-token".into()]);
    let mut previews = tool_preview::Previews::default();
    previews.update(&delta(0, "shell", r#"{"command":"echo private-"#));
    let public = serde_json::to_string(&previews.public(Uuid::nil(), &redactor)).unwrap();
    assert!(!public.contains("private-"));
    previews.update(&ProviderDelta::ToolCall {
        index: 0,
        id: None,
        name: None,
        arguments: r#"\u0074oken"}"#.into(),
    });
    let public = serde_json::to_string(&previews.public(Uuid::nil(), &redactor)).unwrap();
    assert!(!public.contains("private-token"));
    assert!(public.contains("REDACTED"));
}

#[test]
fn preview_indexes_are_ordered_and_attempt_identity_is_not_inferred_from_call_id() {
    let mut previews = tool_preview::Previews::default();
    previews.update(&delta(8, "read_file", r#"{"path":"b"}"#));
    previews.update(&delta(2, "read_file", r#"{"path":"a"}"#));
    let attempt = Uuid::new_v4();
    let public = previews.public(attempt, &Redactor::new(Vec::<String>::new()));
    assert_eq!(public.len(), 2);
    assert_eq!(public[0].index, 2);
    assert_eq!(public[1].index, 8);
    assert!(public.iter().all(|preview| preview.attempt_id == attempt));
}

#[test]
fn reasoning_is_display_only_bounded_and_never_finalized_by_a_delta() {
    let mut previews = reasoning_preview::Previews::default();
    let redactor = Redactor::new(vec!["hidden-value".into()]);
    previews.update(&ProviderDelta::Reasoning {
        kind: voyage_protocol::reasoning_preview::ReasoningKind::Summary,
        index: 0,
        text: "hidden-".into(),
    });
    assert!(
        !serde_json::to_string(&previews.public(Uuid::nil(), &redactor))
            .unwrap()
            .contains("hidden-")
    );
    previews.update(&ProviderDelta::Reasoning {
        kind: voyage_protocol::reasoning_preview::ReasoningKind::Summary,
        index: 0,
        text: "value".into(),
    });
    let public = previews.public(Uuid::nil(), &redactor);
    assert!(public[0].text.contains("REDACTED"));
    assert!(!public[0].finalized);
    previews.update(&ProviderDelta::Reasoning {
        kind: voyage_protocol::reasoning_preview::ReasoningKind::Summary,
        index: 1,
        text: "界".repeat(100_000),
    });
    let public = previews.public(Uuid::nil(), &redactor);
    assert!(public[1].truncated);
    assert!(public[1].text.is_char_boundary(public[1].text.len()));
}
