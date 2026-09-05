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
