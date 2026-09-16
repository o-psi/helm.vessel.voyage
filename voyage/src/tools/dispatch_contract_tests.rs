//! Offline dispatch fixtures: no shell, terminal, provider, or global environment effects.
use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    name: &'static str,
    calls: Arc<AtomicUsize>,
    output: Option<Value>,
    fail: bool,
}
#[async_trait]
impl Tool for Fixture {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.into(),
            description: "offline fixture".into(),
            input_schema: json!({"type":"object"}),
            output_schema: self.output.clone(),
            annotations: None,
        }
    }
    async fn execute(&self, _: Value, _: &ToolContext) -> Result<String, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(ToolError::Failed("private-fixture-value".into()))
        } else {
            Ok("private-fixture-value".into())
        }
    }
}
fn fixture(name: &'static str) -> Fixture {
    Fixture {
        name,
        calls: Arc::new(AtomicUsize::new(0)),
        output: None,
        fail: false,
    }
}
#[test]
fn registry_registration_is_atomic_and_retention_updates_contracts() {
    let mut registry = ToolRegistry::default();
    registry.register_arc(Arc::new(fixture("z"))).unwrap();
    registry
        .register_arc(Arc::new(fixture("read_file")))
        .unwrap();
    assert!(registry.register_arc(Arc::new(fixture("z"))).is_err());
    let mut invalid = fixture("bad");
    invalid.output = Some(json!({"$ref":"https://example.invalid/schema"}));
    assert!(registry.register_arc(Arc::new(invalid)).is_err());
    assert_eq!(
        registry
            .definitions()
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        ["read_file", "z"]
    );
    registry.retain_read_only();
    assert_eq!(registry.definitions().len(), 1);
    registry.retain_allowed(&Default::default());
    assert!(registry.definitions().is_empty());
    assert!(registry.terminals().is_none());
}
#[test]
fn declared_outputs_are_required_but_unknown_tools_have_no_output_contract() {
    let mut registry = ToolRegistry::default();
    let mut tool = fixture("typed");
    tool.output = Some(json!({"type":"integer"}));
    registry.register(tool);
    assert!(registry.validate_output("typed", None).is_err());
    assert!(
        registry
            .validate_output("typed", Some(&json!("private")))
            .is_err()
    );
    assert!(registry.validate_output("typed", Some(&json!(3))).is_ok());
    assert!(registry.validate_output("unknown", None).is_ok());
}
#[tokio::test]
async fn dispatch_validates_before_effects_and_redacts_success_and_failure() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = reliability_tests::context(root.path());
    ctx.redactor = Arc::new(Redactor::new(["private-fixture-value".into()]));
    let mut registry = ToolRegistry::default();
    let tool = fixture("fixture");
    let calls = tool.calls.clone();
    registry.register(tool);
    assert!(registry.execute("absent", json!({}), &ctx).await.is_err());
    assert!(registry.execute("fixture", json!([]), &ctx).await.is_err());
    assert!(
        registry
            .execute("fixture", json!({"workflow_secrets":[]}), &ctx)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let result = registry.execute("fixture", json!({}), &ctx).await.unwrap();
    assert!(!result.contains("private-fixture-value"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut failing = fixture("fixture");
    failing.fail = true;
    registry.register(failing);
    let error = registry
        .execute("fixture", json!({}), &ctx)
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("private-fixture-value"));
    ctx.cancellation.cancel();
    assert!(matches!(
        registry.execute("fixture", json!({}), &ctx).await,
        Err(ToolError::Cancelled)
    ));
}
#[tokio::test]
async fn invalid_output_is_withheld_without_replaying_and_tiny_budget_is_enforced() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = reliability_tests::context(root.path());
    let mut registry = ToolRegistry::default();
    let mut tool = fixture("fixture");
    tool.output = Some(json!({"type":"object"}));
    let calls = tool.calls.clone();
    registry.register(tool);
    let report = registry
        .execute_report_with_workflow_secrets("fixture", json!({}), &ctx, None)
        .await
        .unwrap();
    assert!(report.outcome.incomplete.is_some());
    assert!(
        !report
            .output
            .text_fallback()
            .contains("private-fixture-value")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    ctx.max_output_bytes = 1;
    let report = registry
        .execute_report_with_workflow_secrets("fixture", json!({}), &ctx, None)
        .await
        .unwrap();
    assert!(report.output.text_fallback().is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
#[test]
fn read_only_action_matrix_is_fail_closed() {
    for (name, actions) in [
        ("process", vec!["read", "list"]),
        ("todo", vec!["list"]),
        ("completion", vec!["snapshot", "read"]),
        ("github", vec!["read", "logs", "inspect", "list"]),
        (
            "vessel",
            vec![
                "inspect",
                "capabilities",
                "list",
                "search",
                "history",
                "follow",
                "wait",
                "receipt",
                "routes",
                "operations",
                "controls",
            ],
        ),
        (
            "subagent",
            vec![
                "status",
                "list",
                "archive",
                "wait",
                "wait_many",
                "message",
                "follow_up",
                "worktree_status",
                "worktree_conflicts",
                "cancel",
            ],
        ),
    ] {
        for action in actions {
            assert!(
                allowed_in_read_only(name, &json!({"action":action})),
                "{name}/{action}"
            );
        }
        for value in [
            json!({}),
            json!({"action":null}),
            json!({"action":"unexpected"}),
        ] {
            assert!(!allowed_in_read_only(name, &value));
        }
    }
    for name in [
        "questions",
        "read_file",
        "list_directory",
        "search_files",
        "result",
    ] {
        assert!(allowed_in_read_only(name, &json!({})));
    }
    assert!(allowed_in_read_only("subagent", &json!({"action":"spawn"})));
    assert!(!allowed_in_read_only(
        "subagent",
        &json!({"action":"spawn","worktree":true})
    ));
    for name in ["shell", "write_file", "unknown", "browser"] {
        assert!(!allowed_in_read_only(name, &json!({})));
    }
}
#[tokio::test]
async fn read_only_dispatch_refuses_mutation_before_fixture_execution() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = reliability_tests::context(root.path());
    ctx.policy = Arc::new(
        Policy::new(
            &crate::Config {
                access: Some(AccessMode::ReadOnly),
                ..Default::default()
            },
            root.path().into(),
        )
        .unwrap(),
    );
    let mut registry = ToolRegistry::default();
    let tool = fixture("process");
    let calls = tool.calls.clone();
    registry.register(tool);
    assert!(matches!(
        registry
            .execute("process", json!({"action":"start"}), &ctx)
            .await,
        Err(ToolError::Denied(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    registry
        .execute("process", json!({"action":"list"}), &ctx)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn shell_secret_references_fail_before_any_tool_effect() {
    let root = tempfile::tempdir().unwrap();
    let ctx = reliability_tests::context(root.path());
    let mut registry = ToolRegistry::default();
    let tool = fixture("shell");
    let calls = tool.calls.clone();
    registry.register(tool);
    for value in [
        json!({"workflow_secrets":null}),
        json!({"workflow_secrets":["missing"]}),
        json!({"workflow_secrets":"missing"}),
    ] {
        assert!(registry.execute("shell", value, &ctx).await.is_err());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    registry
        .execute("shell", json!({"workflow_secrets":[]}), &ctx)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
