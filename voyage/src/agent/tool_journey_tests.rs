use super::*;
use crate::model::{ModelResponse, Role, ToolCall};
use std::sync::Mutex;
struct Script {
    requests: Arc<Mutex<Vec<ModelRequest>>>,
    step: Mutex<usize>,
}
#[async_trait]
impl Provider for Script {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut step = self.step.lock().unwrap();
        let mut message = Message::new(Role::Assistant, "");
        match *step {
            0 => message.tool_calls.push(ToolCall {
                id: "write-1".into(),
                name: "write_file".into(),
                arguments: serde_json::json!({"path":"result.txt","content":"fixture data"}),
            }),
            1 => message.tool_calls.push(ToolCall {
                id: "read-1".into(),
                name: "read_file".into(),
                arguments: serde_json::json!({"path":"result.txt"}),
            }),
            _ => message.content = "Verified fixture data".into(),
        };
        *step += 1;
        Ok(ModelResponse {
            message,
            usage: Usage::default(),
            service_tier: None,
        })
    }
}
#[tokio::test]
async fn file_tool_journey_retains_results_and_finishes_canonical_answer() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Script {
        requests: requests.clone(),
        step: Mutex::new(0),
    };
    let mut tools = ToolRegistry::default();
    tools.register(crate::tools::ReadFile);
    tools.register(crate::tools::WriteFile);
    let agent = Agent::new(
        Box::new(provider),
        tools,
        context,
        Arc::new(SilentSink),
        "fixture-model".into(),
        "Fixture instructions".into(),
        1024,
        None,
    );
    let outcome = agent.run(vec![], "write then read".into()).await.unwrap();
    assert_eq!(outcome.answer, "Verified fixture data");
    assert_eq!(
        std::fs::read_to_string(root.path().join("result.txt")).unwrap(),
        "fixture data"
    );
    assert_eq!(
        outcome
            .messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .count(),
        2
    );
    let seen = requests.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(
        seen[2]
            .messages
            .iter()
            .any(|m| m.role == Role::Tool && m.content.contains("fixture data"))
    );
    assert_eq!(agent.tool_inventory().len(), 2);
    assert!(agent.plain_terminals().is_ok());
    assert!(agent.terminal_metadata().is_empty());
    assert_eq!(agent.model(), "fixture-model");
}
#[tokio::test]
async fn cancellation_prevents_inference_and_model_updates_preserve_mirror() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mirror = Arc::new(RwLock::new("initial".into()));
    let agent = Agent::new(
        Box::new(Script {
            requests: requests.clone(),
            step: Mutex::new(0),
        }),
        ToolRegistry::default(),
        context,
        Arc::new(SilentSink),
        "fixture".into(),
        "instructions".into(),
        100,
        None,
    )
    .with_model_mirror(mirror.clone());
    assert_eq!(agent.set_model("updated").unwrap(), "updated");
    assert_eq!(*mirror.read().unwrap(), "updated");
    assert!(agent.set_model(" ").is_err());
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        agent
            .run_with_cancel(vec![], "not sent".into(), cancel)
            .await,
        Err(AgentError::Cancelled)
    ));
    assert!(requests.lock().unwrap().is_empty());
}
