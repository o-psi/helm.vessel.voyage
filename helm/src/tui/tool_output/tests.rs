use super::render;
use crate::model::{Message, ToolCall};
use ratatui::{style::Color, text::Line};
use serde_json::json;

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "test-call".into(),
        name: name.into(),
        arguments,
    }
}

fn text(lines: &[Line<'_>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn shell_preserves_lines_and_indentation_without_empty_stderr() {
    let call = call("shell", json!({"command":"cat source.rs"}));
    let result = Message::tool(
        "test-call",
        "exit: 0\nstdout:\nfn main() {\n    run();\n}\n\nstderr:\n",
    );
    let output = text(&render(&call, Some(&result), false, false, 80));
    assert!(output.contains("shell · cat source.rs"));
    assert!(output.contains("exit 0"));
    assert!(output.contains("fn main() {\n      run();\n  }"));
    assert!(!output.contains("stderr"));
    assert!(!output.contains("\"command\""));
}

#[test]
fn nonzero_and_signal_shell_exits_are_failures_even_when_invocation_succeeded() {
    let call = call("shell", json!({"command":"build"}));
    for exit in ["2", "signal"] {
        let result = Message::tool(
            "test-call",
            format!("exit: {exit}\nstdout:\n\nstderr:\nbuild failed\n"),
        );
        let lines = render(&call, Some(&result), false, false, 80);
        let output = text(&lines);
        assert!(output.contains("✗"));
        assert!(output.contains(&format!("exit {exit}")));
        assert!(output.contains("stderr\n  build failed"));
        assert!(lines.iter().any(|line| line.style.fg == Some(Color::Red)));
    }
}

#[test]
fn stderr_is_visible_when_stdout_exceeds_preview_budget() {
    let call = call("shell", json!({"command":"build"}));
    let result = Message::tool(
        "test-call",
        format!(
            "exit: 1\nstdout:\n{}\nstderr:\nfatal diagnostic",
            "progress\n".repeat(50)
        ),
    );
    let output = text(&render(&call, Some(&result), false, false, 80));
    assert!(output.contains("fatal diagnostic"));
    assert!(output.contains("Ctrl+O"));
    assert!(output.find("fatal diagnostic") < output.find("progress"));
}

#[test]
fn empty_and_truncated_shell_results_retain_exit_and_available_output() {
    let call = call("shell", json!({"command":"true"}));
    let empty = Message::tool("test-call", "exit: 0\nstdout:\n\nstderr:\n");
    let output = text(&render(&call, Some(&empty), false, false, 80));
    assert!(output.contains("exit 0"));
    assert!(!output.contains("stderr"));
    let partial = Message::tool(
        "test-call",
        "exit: 0\nstdout:\npartial stdout\n\n[output truncated at 100 bytes]",
    );
    let output = text(&render(&call, Some(&partial), false, false, 80));
    assert!(output.contains("partial stdout"));
    assert!(output.contains("[output truncated at 100 bytes]"));
}

#[test]
fn subagent_mixed_results_surface_failure_before_long_success() {
    let call = call(
        "subagent",
        json!({"action":"wait_many","ids":["one","two"]}),
    );
    let result = Message::tool("test-call", json!({"ids":["one","two"],"results":[{"status":"completed","result":"success\n".repeat(30)},{"status":"failed","error":"worker timed out"}]}).to_string());
    let output = text(&render(&call, Some(&result), false, false, 80));
    assert!(output.contains("2 agents"));
    assert!(output.contains("1 unsuccessful"));
    assert!(output.contains("worker timed out"));
    assert!(output.contains("✗"));
    assert!(output.find("worker timed out") < output.find("success\n"));
}

#[test]
fn todo_uses_title_and_status_without_confusing_blocked_item_with_failed_call() {
    let call = call("todo", json!({"action":"edit","id":"item-id"}));
    let result = Message::tool("test-call", json!({"archived_at":null,"created_at":"2026-09-04","description":"verbose description".repeat(50),"title":"Repair output previews","status":"blocked","blockers":["Waiting for fixture"]}).to_string());
    let output = text(&render(&call, Some(&result), false, false, 100));
    assert!(output.contains("Repair output previews"));
    assert!(output.contains("blocked"));
    assert!(output.contains("Waiting for fixture"));
    assert!(output.contains("✓"));
    assert!(!output.contains("✗"));
    assert!(!output.contains("archived_at"));
}

#[test]
fn malformed_and_unknown_results_keep_readable_fallback() {
    for name in ["subagent", "mcp_custom"] {
        let call = call(name, json!({"action":"status"}));
        let result = Message::tool("test-call", "{not JSON\n  meaningful result");
        let output = text(&render(&call, Some(&result), false, false, 80));
        assert!(output.contains("{not JSON\n    meaningful result"));
    }
    let call = call("mcp_custom", json!({}));
    let result = Message::tool("test-call", "{\"nested\":{\"answer\":42}}");
    let output = text(&render(&call, Some(&result), false, false, 80));
    assert!(output.contains("\"answer\": 42"));
}

#[test]
fn expanded_details_expose_original_arguments_and_full_result_tail() {
    let call = call(
        "shell",
        json!({"command":"printf many-lines","extra":"argument-tail"}),
    );
    let result = Message::tool(
        "test-call",
        format!(
            "exit: 0\nstdout:\n{}last-result-tail\nstderr:\n",
            "body\n".repeat(30)
        ),
    );
    let collapsed = text(&render(&call, Some(&result), false, false, 80));
    assert!(!collapsed.contains("last-result-tail"));
    assert!(collapsed.contains("Ctrl+O"));
    let expanded = text(&render(&call, Some(&result), false, true, 80));
    assert!(expanded.contains("argument-tail"));
    assert!(expanded.contains("last-result-tail"));
    assert!(expanded.contains("exit: 0"));
    assert!(!expanded.contains("Ctrl+O"));
}

#[test]
fn untrusted_controls_and_unicode_fit_narrow_display_cells() {
    let call = call(
        "shell",
        json!({"command":"echo 界e\u{301}\u{1b}]52;clipboard\u{7}"}),
    );
    let result = Message::tool(
        "test-call",
        "exit: 0\nstdout:\n界e\u{301}🙂\r\n\tindent\u{1b}[2J\u{7}\nstderr:\n",
    );
    for width in [1, 2, 3, 4, 8, 20] {
        for expanded in [false, true] {
            let lines = render(&call, Some(&result), false, expanded, width);
            assert!(
                lines.iter().all(|line| line.width() <= width),
                "width {width}: {}",
                text(&lines)
            );
            let output = text(&lines);
            assert!(!output.chars().any(|ch| ch.is_control() && ch != '\n'));
        }
    }
    let output = text(&render(&call, Some(&result), false, false, 80));
    assert!(output.contains("界e\u{301}🙂"));
}

#[test]
fn render_never_changes_canonical_messages_or_arguments() {
    let call = call("todo", json!({"action":"edit","title":"Title\n\u{1b}[0m"}));
    let result = Message::tool(
        "test-call",
        "{\"title\":\"Original\",\"status\":\"pending\"}",
    );
    let before_call = serde_json::to_value(&call).unwrap();
    let before_result = serde_json::to_value(&result).unwrap();
    for expanded in [false, true] {
        render(&call, Some(&result), false, expanded, 15);
    }
    assert_eq!(before_call, serde_json::to_value(&call).unwrap());
    assert_eq!(before_result, serde_json::to_value(&result).unwrap());
}

#[test]
fn absent_result_and_explicit_errors_have_honest_status() {
    let call = call("shell", json!({"command":"work"}));
    assert!(text(&render(&call, None, true, false, 80)).contains("running"));
    assert!(text(&render(&call, None, false, false, 80)).contains("interrupted / no result"));
    for error in ["permission denied", "cancelled", "timed out"] {
        let result = Message::tool_result("test-call", error, false);
        let output = text(&render(&call, Some(&result), false, false, 80));
        assert!(output.contains("✗"));
        assert!(output.contains(error));
    }
}
