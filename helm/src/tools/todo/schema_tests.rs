use super::*;

#[test]
fn clear_completed_rejects_irrelevant_action_fields() {
    assert!(
        serde_json::from_value::<Args>(
            json!({"action":"clear_completed","id":"00112233-4455-4677-8899-aabbccddeeff"})
        )
        .is_err()
    );
}

#[test]
fn advertised_status_rejects_observed_cross_action_text() {
    let root = tempfile::tempdir().unwrap();
    let tool = TodoTool::new(Arc::new(TodoStore::new(
        root.path().join("todos.json"),
        crate::todo::TodoScope::workspace(root.path().into()),
    )));
    let args = json!({"action":"status","id":"00112233-4455-4677-8899-aabbccddeeff","status":"blocked","text":"Verification evidence recorded; disposition pending."});
    assert!(serde_json::from_value::<Args>(args.clone()).is_err());
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .should_validate_formats(true)
        .build(&tool.definition().input_schema)
        .unwrap();
    assert!(!validator.is_valid(&args));
}

const ID: &str = "00112233-4455-4677-8899-aabbccddeeff";

fn definition(root: &std::path::Path) -> ToolDefinition {
    TodoTool::new(Arc::new(TodoStore::new(
        root.join("todos.json"),
        crate::todo::TodoScope::workspace(root.into()),
    )))
    .definition()
}

// Independent protocol corpus: table is deliberately separate from production
// schema construction. Omission/default/null behavior must remain compatible.
fn corpus() -> Vec<(Value, bool)> {
    let fields = json!({"id":ID,"title":"Verify measured records","description":"Read actual.txt","priority":"normal","status":"pending","order":-1,"assignees":["worker"],"blockers":["waiting for approval"],"add":[ID],"remove":[ID],"text":"Measured records: 42 雪\n","author":"operator","include_archived":true});
    let specs: [(&str, &[&str], &[&str], &[&str]); 14] = [
        (
            "create",
            &["title"],
            &["description", "priority", "order", "assignees"],
            &["order"],
        ),
        ("list", &[], &["include_archived", "status"], &["status"]),
        (
            "edit",
            &["id"],
            &["title", "description", "priority"],
            &["title", "description", "priority"],
        ),
        ("status", &["id", "status"], &[], &[]),
        ("block", &["id"], &["blockers"], &[]),
        ("dependencies", &["id"], &["add", "remove"], &[]),
        ("assign", &["id"], &["assignees"], &[]),
        ("note", &["id", "text"], &["author"], &["author"]),
        ("progress", &["id", "text"], &["author"], &["author"]),
        ("evidence", &["id", "text"], &["author"], &["author"]),
        ("reorder", &["id", "order"], &[], &[]),
        ("remove", &["id"], &[], &[]),
        ("archive", &["id"], &[], &[]),
        ("clear_completed", &[], &[], &[]),
    ];
    let mut cases = Vec::new();
    for (action, required, optional, nullable) in specs {
        let mut minimum = json!({"action":action});
        for key in required {
            minimum[*key] = fields[*key].clone();
        }
        cases.push((minimum.clone(), true));
        let mut full = minimum.clone();
        for key in optional {
            full[*key] = fields[*key].clone();
        }
        cases.push((full.clone(), true));
        for key in std::iter::once(&"action").chain(required.iter()) {
            let mut missing = full.clone();
            missing.as_object_mut().unwrap().remove(*key);
            cases.push((missing, false));
            let mut null = full.clone();
            null[*key] = Value::Null;
            cases.push((null, false));
        }
        for key in optional {
            let mut omitted = full.clone();
            omitted.as_object_mut().unwrap().remove(*key);
            cases.push((omitted, true));
            let mut null = full.clone();
            null[*key] = Value::Null;
            cases.push((null, nullable.contains(key)));
        }
        for key in fields.as_object().unwrap().keys() {
            if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
                let mut extra = full.clone();
                extra[key] = fields[key].clone();
                cases.push((extra, false));
            }
        }
        let mut unknown = full;
        unknown["unexpected"] = json!(true);
        cases.push((unknown, false));
    }
    for status in [
        "pending",
        "in_progress",
        "blocked",
        "completed",
        "cancelled",
    ] {
        cases.push((json!({"action":"status","id":ID,"status":status}), true));
    }
    for priority in ["low", "normal", "high", "critical"] {
        cases.push((
            json!({"action":"create","title":"task","priority":priority}),
            true,
        ));
    }
    for order in [i64::MIN, i64::MAX] {
        cases.push((json!({"action":"reorder","id":ID,"order":order}), true));
    }
    cases.extend([
        (json!({"action":"status","id":ID,"status":"blocked","text":"Observed invalid live call"}),false),
        (json!({"action":"status","id":ID,"status":"done"}),false),
        (json!({"action":"create","title":"task","priority":"urgent"}),false),
        (json!({"action":"create","title":1}),false),
        (json!({"action":"create","title":"","description":"","assignees":[]}),true),
        (json!({"action":"block","id":ID,"blockers":[]}),true),
        (json!({"action":"block","id":ID,"blockers":[42]}),false),
        (json!({"action":"assign","id":ID,"assignees":"worker"}),false),
        (json!({"action":"dependencies","id":ID,"add":["bad-uuid"]}),false),
        (json!({"action":"dependencies","id":ID,"remove":[false]}),false),
        (json!({"action":"list","include_archived":1}),false),
        (json!({"action":"list","status":1}),false),
        (json!({"action":"note","id":ID,"text":[]}),false),
        (json!({"action":"progress","id":ID,"text":"note","author":42}),false),
        (json!({"action":"evidence","id":ID,"text":""}),true),
        (json!({"action":"remove","id":"bad-uuid"}),false),
        (json!({"action":"reorder","id":ID,"order":1.5}),false),
        (json!({"action":"reorder","id":ID,"order":u64::MAX}),false),
        (json!({"action":"reorder","id":ID,"order":(i64::MIN as f64)*2.0}),false),
        (json!({"action":"unknown"}),false),(json!({}),false),(json!([]),false),(json!(null),false),
    ]);
    cases
}

#[test]
fn todo_schema_matches_every_action_and_optional_field_contract() {
    let root = tempfile::tempdir().unwrap();
    let schema = definition(root.path()).input_schema;
    let validator = crate::provider::schema_fixture::validator(&schema);
    for (args, expected) in corpus() {
        assert_eq!(
            serde_json::from_value::<Args>(args.clone()).is_ok(),
            expected,
            "decoder {args}"
        );
        assert_eq!(validator.is_valid(&args), expected, "schema {args}");
    }
}

#[test]
fn todo_examples_cover_every_action_with_valid_arguments() {
    let root = tempfile::tempdir().unwrap();
    let definition = definition(root.path());
    println!(
        "todo definition bytes: {}",
        serde_json::to_vec(&definition).unwrap().len()
    );
    let completion = crate::completion::tool::CompletionTool::new(
        Arc::new(TodoStore::new(
            root.path().join("completion-todos.json"),
            crate::todo::TodoScope::workspace(root.path().into()),
        )),
        crate::subagent::AgentTreeStore::new(root.path().join("agents.json")),
    )
    .definition();
    println!(
        "completion definition bytes: {}",
        serde_json::to_vec(&completion).unwrap().len()
    );
    let schema = definition.input_schema;
    println!(
        "todo schema bytes: {}",
        serde_json::to_vec(&schema).unwrap().len()
    );
    let validator = crate::provider::schema_fixture::validator(&schema);
    let encoded_examples = schema["examples"].to_string();
    assert!(
        !encoded_examples.contains("actual.txt") && !encoded_examples.contains("claims.txt"),
        "runtime examples must not embed evaluation seed files"
    );
    let examples = schema["examples"]
        .as_array()
        .expect("one valid example per action");
    let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
    assert_eq!(examples.len(), 14);
    assert_eq!(examples.len(), actions.len());
    for action in actions {
        assert_eq!(
            examples.iter().filter(|e| e["action"] == *action).count(),
            1
        );
    }
    for example in examples {
        assert!(validator.is_valid(example), "{example}");
        assert!(serde_json::from_value::<Args>(example.clone()).is_ok());
    }
    for (_, property) in schema["properties"].as_object().unwrap() {
        assert!(
            property["description"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );
    }
    assert!(definition.description.contains("block"));
    assert!(definition.description.contains("evidence"));
}

#[tokio::test]
async fn native_http_providers_preserve_all_todo_actions() {
    let root = tempfile::tempdir().unwrap();
    crate::provider::schema_fixture::assert_native_schema(definition(root.path()), corpus(), 14)
        .await;
}

#[tokio::test]
async fn invalid_clear_completed_cannot_mutate_durable_records() {
    let root = tempfile::tempdir().unwrap();
    let context = super::tests::context(&root);
    let tool = TodoTool::new(Arc::new(TodoStore::new(
        root.path().join("todos.json"),
        crate::todo::TodoScope::workspace(root.path().into()),
    )));
    let mut ids = Vec::new();
    for title in ["one", "two"] {
        let item: Value = serde_json::from_str(
            &tool
                .execute(json!({"action":"create","title":title}), &context)
                .await
                .unwrap(),
        )
        .unwrap();
        tool.execute(
            json!({"action":"status","id":item["id"],"status":"completed"}),
            &context,
        )
        .await
        .unwrap();
        ids.push(item["id"].clone());
    }
    let before = std::fs::read(root.path().join("todos.json")).unwrap();
    assert!(matches!(
        tool.execute(json!({"action":"clear_completed","id":ids[0]}), &context)
            .await,
        Err(ToolError::InvalidArguments(_))
    ));
    assert_eq!(
        std::fs::read(root.path().join("todos.json")).unwrap(),
        before
    );
    let result: Value = serde_json::from_str(
        &tool
            .execute(json!({"action":"clear_completed"}), &context)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["archived"], 2);
    assert!(
        tool.store
            .snapshot()
            .await
            .unwrap()
            .items
            .values()
            .all(|item| item.archived())
    );
}

#[tokio::test]
async fn blocked_status_error_names_callable_action_and_preserves_state() {
    let root = tempfile::tempdir().unwrap();
    let context = super::tests::context(&root);
    let tool = TodoTool::new(Arc::new(TodoStore::new(
        root.path().join("todos.json"),
        crate::todo::TodoScope::workspace(root.path().into()),
    )));
    let item: Value = serde_json::from_str(
        &tool
            .execute(
                json!({"action":"create","title":"await real approval"}),
                &context,
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let before = std::fs::read(root.path().join("todos.json")).unwrap();
    let error = tool
        .execute(
            json!({"action":"status","id":item["id"],"status":"blocked"}),
            &context,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("action=block") && error.contains("id") && error.contains("blockers"),
        "{error}"
    );
    assert!(!error.contains("set_blockers"));
    assert_eq!(
        std::fs::read(root.path().join("todos.json")).unwrap(),
        before
    );
    let blocked: Value = serde_json::from_str(
        &tool
            .execute(
                json!({"action":"block","id":item["id"],"blockers":["approval outstanding"]}),
                &context,
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["blockers"], json!(["approval outstanding"]));
    assert_eq!(blocked["evidence"], json!([]));
    let reopened: Value = serde_json::from_str(
        &tool
            .execute(
                json!({"action":"block","id":item["id"],"blockers":[]}),
                &context,
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reopened["status"], "pending");
    assert_eq!(reopened["blockers"], json!([]));
}
