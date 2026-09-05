use super::*;
use crate::{
    model::{ModelResponse, Role, ToolCall},
    provider::ProviderError,
    workflow::secrets::SecretInputs,
};
use std::sync::Mutex;

struct PrivateCall(Arc<Mutex<Vec<ModelRequest>>>);
#[async_trait]
impl Provider for PrivateCall {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let used = request.messages.iter().any(|m| m.role == Role::Tool);
        self.0.lock().unwrap().push(request);
        let mut message = Message::new(Role::Assistant, if used { "finished" } else { "" });
        if !used {
            message.tool_calls.push(ToolCall { id: "private-call".into(), name: "shell".into(), arguments: serde_json::json!({"command":"test \"$HELM_WORKFLOW_TOKEN\" = \"$HELM_WORKFLOW_TOKEN\" && test -n \"$HELM_WORKFLOW_TOKEN\"", "workflow_secrets":["token"]}) });
        }
        Ok(ModelResponse {
            message,
            usage: Usage::default(),
        })
    }
}
fn bindings(id: uuid::Uuid) -> crate::workflow::secrets::RunBindings {
    let doc = crate::workflow::parse(b"schema_version=1\nid='private-test'\nversion='1'\ndescription='Private input'\nprompt='Use {{token}}'\n[parameters.token]\ntype='string'\nsecret=true\nrequired=true\n").unwrap();
    SecretInputs::collect(&doc, vec![("token".into(), "private-秘密-value".into())])
        .unwrap()
        .bind(id)
        .unwrap()
}
fn checkpoint(dir: &tempfile::TempDir, id: uuid::Uuid) -> crate::session::SessionCheckpoint {
    let mut session = crate::session::Session::new(dir.path().into(), "test".into());
    session.begin_run_summary(id);
    crate::session::SessionCheckpoint::new(session, None, id)
}
#[tokio::test]
async fn secret_binding_fences_checkpoint_without_completion_scope_before_provider_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let agent = tests::agent(Box::new(PrivateCall(requests.clone())), &dir);
    let cp = checkpoint(&dir, uuid::Uuid::new_v4());
    let result = agent
        .run_checkpointed_scoped_with_workflow_secrets(
            vec![],
            "public reference".into(),
            CancellationToken::new(),
            None,
            &cp,
            "test".into(),
            None,
            Some(bindings(uuid::Uuid::new_v4())),
        )
        .await;
    assert!(result.is_err());
    assert!(requests.lock().unwrap().is_empty());
}
#[cfg(unix)]
#[tokio::test]
async fn secret_binding_is_run_local_and_never_enters_requests_or_canonical_history() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut agent = tests::agent(Box::new(PrivateCall(requests.clone())), &dir);
    agent.tools.register(crate::tools::Shell);
    let id = uuid::Uuid::new_v4();
    let cp = checkpoint(&dir, id);
    let result = agent
        .run_checkpointed_scoped_with_workflow_secrets(
            vec![],
            "Use the public token reference".into(),
            CancellationToken::new(),
            None,
            &cp,
            "test".into(),
            None,
            Some(bindings(id)),
        )
        .await
        .unwrap();
    let tool = result
        .messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&tool.content).unwrap(),
        serde_json::json!({"status":"exited","code":0})
    );
    let snapshot = cp.snapshot_for_finish().await;
    for text in [
        serde_json::to_string(&snapshot).unwrap(),
        serde_json::to_string(&result.messages).unwrap(),
        serde_json::to_string(&*requests.lock().unwrap()).unwrap(),
    ] {
        assert!(!text.contains("private-秘密-value"));
    }
    let later = agent
        .run(vec![], "Try the same reference again".into())
        .await
        .unwrap();
    assert!(
        later
            .messages
            .iter()
            .any(|m| m.role == Role::Tool && m.content.contains("unavailable"))
    );
    assert!(agent.context.environment.is_empty());
}
