use super::*;
use crate::todo::TodoScope;

const ID: &str = "00112233-4455-4677-8899-aabbccddeeff";

fn definition(root: &std::path::Path) -> ToolDefinition {
    CompletionTool::new(
        Arc::new(TodoStore::new(
            root.join("todos.json"),
            TodoScope::workspace(root.to_owned()),
        )),
        AgentTreeStore::new(root.join("agents.json")),
    )
    .definition()
}

use crate::provider::schema_fixture::validator;

// Expected results are independent of the advertised schema. This compares
// canonical JSON UUID strings/integer values, not every lenient serde spelling.
fn corpus() -> Vec<(Value, bool)> {
    let mut cases = vec![(json!({"action":"snapshot"}), true)];
    for kind in ["todo", "agent"] {
        cases.push((json!({"action":"read","kind":kind,"id":ID}), true));
        cases.push((
            json!({"action":"adopt","kind":kind,"id":ID,"revision":0}),
            true,
        ));
        for disposition in [
            "completed_with_evidence",
            "cancelled_with_reason",
            "blocked_with_impact",
            "deferred_with_impact",
            "incorporated",
            "failure_with_impact",
            "not_needed_with_reason",
        ] {
            cases.push((json!({"action":"account","kind":kind,"id":ID,"revision":1,"fingerprint":"snapshot-fingerprint","disposition":disposition,"reason":"Verified evidence 雪\nwith a concrete impact"}),true));
        }
    }
    cases.push((
        json!({"action":"adopt","kind":"todo","id":ID,"revision":u64::MAX}),
        true,
    ));
    // Empty review strings are syntactically valid. Runtime semantics still
    // reject inadequate evidence/reasons/fingerprints; schemas cannot grant them.
    cases.push((json!({"action":"account","kind":"todo","id":ID,"revision":0,"fingerprint":"","disposition":"completed_with_evidence","reason":""}),true));
    let valid = cases.clone();
    for (value, _) in valid {
        for key in value.as_object().unwrap().keys() {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(key);
            cases.push((missing, false));
            let mut null = value.clone();
            null[key] = Value::Null;
            cases.push((null, false));
        }
        for (key, replacement) in [
            ("kind", json!("todo")),
            ("id", json!(ID)),
            ("revision", json!(1)),
            ("fingerprint", json!("fingerprint")),
            ("disposition", json!("deferred_with_impact")),
            ("reason", json!("reason")),
            ("unexpected", json!(true)),
        ] {
            if value.get(key).is_none() {
                let mut extra = value.clone();
                extra[key] = replacement;
                cases.push((extra, false));
            }
        }
    }
    cases.extend([
        (json!({"action":"adopt","kind":"todo","id":ID,"revision":(u64::MAX as f64)*2.0}),false),
        (json!({"action":"read","kind":"todo","reason":"","revision":1}),false),
        (json!({"action":"read","id":ID}),false),
        (json!({"action":"read","kind":"unknown","id":ID}),false),
        (json!({"action":"read","kind":"todo","id":"not-a-uuid"}),false),
        (json!({"action":"read","kind":"todo","id":false}),false),
        (json!({"action":"adopt","kind":"todo","id":ID,"revision":-1}),false),
        (json!({"action":"adopt","kind":"todo","id":ID,"revision":1.5}),false),
        (json!({"action":"adopt","kind":"todo","id":ID,"revision":"1"}),false),
        (json!({"action":"account","kind":"todo","id":ID,"revision":1,"fingerprint":1,"disposition":"incorporated","reason":"why"}),false),
        (json!({"action":"account","kind":"todo","id":ID,"revision":1,"fingerprint":"f","disposition":"done","reason":"why"}),false),
        (json!({"action":"account","kind":"todo","id":ID,"revision":1,"fingerprint":"f","disposition":"incorporated","reason":[]}),false),
        (json!({"action":"unknown"}),false),(json!({}),false),(json!([]),false),(json!(null),false),
    ]);
    cases
}

#[test]
fn advertised_completion_schema_matches_strict_action_corpus() {
    let root = tempfile::tempdir().unwrap();
    let schema = definition(root.path()).input_schema;
    let validator = validator(&schema);
    for (args, expected) in corpus() {
        assert_eq!(
            serde_json::from_value::<Args>(args.clone()).is_ok(),
            expected,
            "decoder: {args}"
        );
        assert_eq!(validator.is_valid(&args), expected, "schema: {args}");
    }
}

#[test]
fn completion_action_examples_are_valid_and_describe_fresh_review() {
    let root = tempfile::tempdir().unwrap();
    let definition = definition(root.path());
    let schema = &definition.input_schema;
    let validator = validator(schema);
    let examples = schema["examples"]
        .as_array()
        .expect("valid per-action examples");
    assert_eq!(examples.len(), 4);
    for example in examples {
        assert!(validator.is_valid(example), "{example}");
        assert!(serde_json::from_value::<Args>(example.clone()).is_ok());
    }
    assert!(definition.description.contains("revision and fingerprint"));
    assert!(
        definition
            .description
            .contains("read accepts only action, kind and id")
    );
    for (_, property) in schema["properties"].as_object().unwrap() {
        assert!(
            property["description"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
        );
    }
}

#[tokio::test]
async fn native_http_providers_preserve_completion_action_contract() {
    let root = tempfile::tempdir().unwrap();
    crate::provider::schema_fixture::assert_native_schema(definition(root.path()), corpus(), 4)
        .await;
}
