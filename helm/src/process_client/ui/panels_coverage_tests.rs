use super::*;
use serde_json::json;

#[test]
fn inventories_explain_every_builtin_without_exposing_wire_fields() {
    let names = [
        "shell",
        "process",
        "questions",
        "todo",
        "subagent",
        "completion",
        "list_directory",
        "read_file",
        "write_file",
        "apply_patch",
        "search_files",
        "extension",
    ];
    let inventory: Vec<_> = names
        .iter()
        .map(|name| json!({"name":name,"secret":"never display"}))
        .collect();
    let output = render("tools", &json!({"value":{"inventory":inventory}}));
    for expected in [
        "Run a command",
        "Interactive programs",
        "Ask a question",
        "Tasks",
        "Delegate work",
        "Read files",
        "Write files",
        "Edit files",
        "Search files",
        "extension",
    ] {
        assert!(output.contains(expected), "{expected}: {output}");
    }
    assert!(!output.contains("never display"));
    assert!(render("tools", &json!([])).contains("No tools available"));
}

#[test]
fn empty_and_populated_collections_have_human_readable_summaries() {
    for (section, value, expected) in [
        ("todos", json!({"items":[]}), "No tasks yet"),
        ("subagents", json!([]), "No agents working"),
        ("models", json!([]), "No models were returned"),
        ("workflows", json!([]), "No saved workflows"),
        (
            "todos",
            json!({"items":[{"title":"Repair parser","status":"blocked","description":"Add regression","blockers":["fixture",42]}]}),
            "Waiting on: fixture",
        ),
        (
            "subagents",
            json!({"agent":{"name":"helper","status":"completed","task":"Inspect parser","result":"Found mismatch","error":"Synthetic error"}}),
            "Found mismatch",
        ),
        (
            "models",
            json!([{"display_name":"Friendly model","id":"model-id","description":"Fast"},{"id":"fallback"}]),
            "Model name: model-id",
        ),
        (
            "workflows",
            json!([{"document":{"id":"daily","description":"Daily check"}}]),
            "Daily check",
        ),
    ] {
        let output = render(section, &value);
        assert!(output.contains(expected), "{section}: {output}");
        assert_eq!(output, render(section, &json!({"value": value})));
    }
}

#[test]
fn policy_aliases_roots_and_cleanup_limits_are_not_progress() {
    for (mode, expected) in [
        ("read-only", "Read only"),
        ("read_only", "Read only"),
        ("approval", "Ask first"),
        ("unrestricted", "Unrestricted"),
        ("future", "Permissions are set"),
    ] {
        let rules = json!({"access":mode,"read_roots":["/synthetic/read",17],"write_roots":["/synthetic/write"]});
        let output = render("policy", &json!({"rules":rules}));
        for text in [
            expected,
            "Readable folders",
            "Writable folders",
            "/synthetic/read",
            "/synthetic/write",
        ] {
            assert!(output.contains(text), "{output}");
        }
        assert_eq!(output, render("policy", &rules));
    }
    let output = render(
        "host_resources",
        &json!({"limits":{"executors":3,"terminals":null},"total":7}),
    );
    for text in [
        "Concurrent runs: up to 3",
        "Interactive terminals: no account-wide quota",
        "Unresolved cleanup records: 7",
        "do not indicate how much work has completed",
    ] {
        assert!(output.contains(text), "{output}");
    }
}

#[test]
fn inventory_is_bounded_and_terminal_controls_are_sanitized() {
    let items: Vec<_> = (0..300)
        .map(|i| json!({"name":format!("tool-{i:03}")}))
        .collect();
    let output = render("tools", &json!(items));
    assert!(output.contains("tool-255"));
    assert!(!output.contains("tool-256"));
    let output = render(
        "todos",
        &json!({"items":[{"title":"a\u{1b}[31mb","description":"x\u{7}y","status":"pending"}]}),
    );
    assert!(!output.contains('\u{1b}'));
    assert!(!output.contains('\u{7}'));
}

#[test]
fn display_wraps_words_graphemes_and_heading_styles() {
    let text = display(
        "# Heading\n\nhello world\nabcdefgh\n界界\ne\u{301}e\u{301}",
        5,
    );
    let lines: Vec<_> = text.lines.iter().map(ToString::to_string).collect();
    assert_eq!(
        lines,
        [
            "Headi",
            "ng",
            "",
            "hello",
            "world",
            "abcde",
            "fgh",
            "界界",
            "e\u{301}e\u{301}"
        ]
    );
    assert_eq!(text.lines[0].style, crate::theme::Role::Focus.style());
    assert_eq!(text.lines[3].style, ratatui::style::Style::default());
    assert_eq!(
        display("ab", 0)
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}
