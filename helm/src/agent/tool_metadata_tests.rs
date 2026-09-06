//! Provider-boundary metadata checks without modifying registry definitions.
use super::*;
use crate::model::{ModelResponse, Role};
use crate::tools::{Tool, ToolError};
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

const SECRET: &str = "metadata-秘密\"\\\n-canary";

#[derive(Default)]
struct Evidence {
    requests: Mutex<Vec<ModelRequest>>,
    effects: AtomicUsize,
}

struct Recorder(Arc<Evidence>);
#[async_trait]
impl Provider for Recorder {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.0.requests.lock().unwrap().push(request);
        Ok(ModelResponse {
            message: Message::new(Role::Assistant, "metadata checked"),
            usage: Usage::default(),
        })
    }
}

struct Metadata {
    definition: ToolDefinition,
    evidence: Arc<Evidence>,
}
#[async_trait]
impl Tool for Metadata {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }
    async fn execute(&self, _: Value, _: &ToolContext) -> Result<String, ToolError> {
        self.evidence.effects.fetch_add(1, Ordering::SeqCst);
        Ok("fixture effect".into())
    }
}

#[tokio::test]
async fn metadata_projection_preserves_registry_and_refuses_executable_secrets() {
    for location in [
        "description",
        "schema_description",
        "nested",
        "enum",
        "default",
        "key",
        "name",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let evidence = Arc::new(Evidence::default());
        let mut definition = ToolDefinition {
            name: "metadata_fixture".into(),
            description: "ordinary description".into(),
            input_schema: json!({"type":"object","properties":{"choice":{"type":"string","enum":["stable-雪\\choice"],"default":"stable-雪\\choice"}},"required":["choice"]}),
        };
        match location {
            "description" => definition.description = format!("before {SECRET} after"),
            "schema_description" => definition.input_schema["description"] = json!(SECRET),
            "nested" => definition.input_schema["examples"] = json!([{"nested":[SECRET]}]),
            "enum" => definition.input_schema["properties"]["choice"]["enum"] = json!([SECRET]),
            "default" => definition.input_schema["properties"]["choice"]["default"] = json!(SECRET),
            "key" => {
                definition.input_schema["properties"][SECRET] = json!({"type":"string"});
            }
            "name" => definition.name = SECRET.into(),
            _ => unreachable!(),
        }
        let original = serde_json::to_value(&definition).unwrap();
        let mut agent = tests::agent(Box::new(Recorder(evidence.clone())), &directory);
        agent.context.redactor = Arc::new(crate::tools::Redactor::new([SECRET.into()]));
        agent.tools.register(Metadata {
            definition,
            evidence: evidence.clone(),
        });
        let result = agent.run(vec![], "Check tool metadata".into()).await;
        assert_eq!(
            serde_json::to_value(&agent.tool_inventory()[0]).unwrap(),
            original,
            "registry changed for {location}"
        );
        assert_eq!(evidence.effects.load(Ordering::SeqCst), 0);
        let requests = evidence.requests.lock().unwrap();
        if location == "description" {
            result.unwrap();
            assert_eq!(requests.len(), 1);
            let projected = &requests[0].tools[0];
            assert_eq!(projected.description, "before [REDACTED] after");
            assert_eq!(projected.name, original["name"].as_str().unwrap());
            assert_eq!(projected.input_schema, original["input_schema"]);
            assert!(
                !requests[0]
                    .messages
                    .iter()
                    .any(|message| message.content.contains(SECRET))
            );
        } else {
            assert!(result.is_err(), "accepted secret in {location}");
            assert!(
                requests.is_empty(),
                "provider received secret in {location}"
            );
            assert!(!result.unwrap_err().to_string().contains(SECRET));
        }
    }
}
