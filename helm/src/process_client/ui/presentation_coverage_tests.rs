use super::*;
use serde_json::json;

#[test]
fn notices_translate_transport_and_stale_identity_failures() {
    for (input, expected) in [
        ("deadline elapsed", "may still be running"),
        ("TIMED OUT", "may still be running"),
        ("identity mismatch", "view changed"),
        ("revision conflict", "Review the latest"),
        ("incarnation changed", "reconnected"),
        ("stale control", "reconnected"),
    ] {
        assert!(notice(input).contains(expected));
    }
    let id = uuid::Uuid::new_v4();
    assert_eq!(
        notice(&format!("request ({id}) failed")),
        "request this request failed"
    );
}

#[test]
fn receipt_statuses_and_reasons_have_user_facing_copy() {
    for (status, expected) in [
        ("accepted", "Request received."),
        ("applied", "Update confirmed."),
        ("completed", "Done."),
        ("rejected", "draft is retained in memory"),
        ("not_admitted", "draft is retained in memory"),
        ("unknown", "Not confirmed yet"),
        ("failed", "couldn't be completed"),
        ("other", "Status updated."),
    ] {
        assert!(receipt(&json!({"status":status})).contains(expected));
    }
    assert!(
        receipt(&json!({"status":"failed","reason":"revision conflict"}))
            .contains("Review the latest")
    );
    assert!(
        receipt(&json!({"status":"failed","detail":"detail wins","reason":"ignored"}))
            .contains("detail wins")
    );
}

#[test]
fn structured_operator_messages_translate_terminal_and_question_results() {
    assert_eq!(
        structured_message(
            &format!("started PTY process {}", uuid::Uuid::new_v4()),
            "process"
        )
        .unwrap(),
        "Terminal opened. Press F3 to view it."
    );
    assert_eq!(
        structured_message(r#"{"status":"selected","answer":"yes"}"#, "questions").unwrap(),
        "Answer sent: yes"
    );
    assert_eq!(
        structured_message(r#"{"status":"custom","answer":"custom"}"#, "questions").unwrap(),
        "Answer sent: custom"
    );
    assert_eq!(
        structured_message(r#"{"status":"cancelled"}"#, "questions").unwrap(),
        "Question skipped."
    );
    assert!(
        structured_message(r#"{"status":"unavailable"}"#, "questions")
            .unwrap()
            .contains("couldn't be shown")
    );
    for input in ["not json", "42", "null", "\"plain\""] {
        assert!(structured_message(input, "shell").is_none());
    }
}

#[test]
fn field_rendering_handles_empty_nested_and_scalar_values() {
    let text = fields(
        &json!({"enabled":true,"disabled":false,"missing":null,"count":7,"list":["first",{"nested_key":"value"}],"empty":[]}),
    );
    for expected in [
        "Yes",
        "No",
        "Not set",
        "7",
        "first",
        "Nested key",
        "value",
        "None",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert_eq!(scalar(&json!({})), "Details available in panel");
    assert!(fields(&json!({})).is_empty());
}

#[test]
fn field_display_limits_bound_large_arrays() {
    let text = fields(&json!(vec!["row"; 1600]));
    assert!(text.ends_with("[Display limit reached]"));
    assert!(text.lines().count() <= 1501);
}

#[test]
fn styled_wrapping_preserves_graphemes_and_handles_zero_width() {
    use ratatui::{
        style::{Color, Style},
        text::{Span, Text},
    };
    let style = Style::default().fg(Color::Red);
    let text = wrap(Text::from(Span::styled("a界e\u{301}z", style)), 3);
    assert_eq!(
        text.lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["a界", "e\u{301}z"]
    );
    assert!(
        text.lines
            .iter()
            .flat_map(|line| &line.spans)
            .all(|span| span.style == style)
    );
    assert_eq!(wrap(Text::from("abc"), 0).lines.len(), 3);
}

#[test]
fn every_run_state_has_a_nontechnical_label() {
    for (state, label) in [
        ("accepted", "Starting"),
        ("running", "Working"),
        ("awaiting_decision", "Waiting for you"),
        ("cancel_requested", "Stopping"),
        ("completed", "Finished"),
        ("failed", "Needs attention"),
        ("cancelled", "Stopped"),
        ("interrupted", "Interrupted"),
        ("unknown", "Needs attention"),
    ] {
        assert_eq!(run_state(state), label);
    }
}
