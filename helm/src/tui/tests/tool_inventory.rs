use super::*;

const SECRET: &str = "inventory-秘密\"\\\nsecret";

struct NoProvider;
#[async_trait]
impl crate::provider::Provider for NoProvider {
    async fn complete(
        &self,
        _: crate::model::ModelRequest,
    ) -> Result<crate::model::ModelResponse, crate::provider::ProviderError> {
        panic!("operator inventory must not call a provider")
    }
}
struct MetadataTool(crate::model::ToolDefinition);
#[async_trait]
impl crate::tools::Tool for MetadataTool {
    fn definition(&self) -> crate::model::ToolDefinition {
        self.0.clone()
    }
    async fn execute(
        &self,
        _: serde_json::Value,
        _: &crate::tools::ToolContext,
    ) -> Result<String, crate::tools::ToolError> {
        panic!("operator inventory must not execute a tool")
    }
}

#[tokio::test]
async fn tool_inventory_activity_redacts_and_escapes_without_changing_executable_registry() {
    let directory = tempfile::tempdir().unwrap();
    let definition = crate::model::ToolDefinition {
        name: "mcp_fixture_echo".into(),
        description: format!("before {SECRET} after \u{1b}[31mRED\u{1b}[0m\u{202e}visible"),
        input_schema: serde_json::json!({"type":"object","properties":{"choice":{"enum":["unchanged"]}}}),
    };
    let original = serde_json::to_value(&definition).unwrap();
    let mut tools = crate::tools::ToolRegistry::default();
    tools.register(MetadataTool(definition));
    let agent = Arc::new(Agent::new(
        Box::new(NoProvider),
        tools,
        crate::tools::ToolContext {
            github: None,
            completion: None,
            policy: Arc::new(
                crate::policy::Policy::new(
                    &crate::config::Config::default(),
                    directory.path().into(),
                )
                .unwrap(),
            ),
            approver: Arc::new(crate::tools::UnattendedApprover { allow: false }),
            timeout: Duration::from_secs(1),
            max_output_bytes: 4096,
            environment: Default::default(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: Uuid::new_v4(),
            interaction: crate::tools::InteractionMode::Attended,
            redactor: Arc::new(crate::tools::Redactor::new([SECRET.into()])),
        },
        Arc::new(crate::agent::SilentSink),
        "fixture".into(),
        "system".into(),
        100,
        None,
    ));
    let mut app = App::new(
        Session::new(directory.path().into(), "fixture".into()),
        vec![],
    );
    app.composer.insert_str("retained draft 雪");
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let canonical = serde_json::to_value(&app.session.messages).unwrap();
    handle_command("/tools", &mut app, &mut store, Some(&agent), None)
        .await
        .unwrap();
    let output = app.activity.join("\n");
    assert!(!output.contains(SECRET));
    assert!(!output.contains('\u{1b}') && !output.contains('\u{202e}'));
    assert!(output.contains("mcp_fixture_echo") && output.contains("before [REDACTED] after"));
    assert!(output.contains("RED") && output.contains("visible"));
    assert_eq!(
        serde_json::to_value(&agent.tool_inventory()[0]).unwrap(),
        original
    );
    assert_eq!(
        serde_json::to_value(&app.session.messages).unwrap(),
        canonical
    );
    for width in [10, 40, 100] {
        let rendered = transcript(&app, width).to_string();
        assert!(!rendered.contains(SECRET));
        assert!(!rendered.contains('\u{1b}') && !rendered.contains('\u{202e}'));
    }
}
