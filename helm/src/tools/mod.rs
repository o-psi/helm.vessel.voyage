mod filesystem;
pub mod mcp;
mod process;
mod shell;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use thiserror::Error;

use crate::{model::ToolDefinition, policy::Policy};
pub use filesystem::{ApplyPatch, ListDirectory, ReadFile, SearchFiles, WriteFile};
pub use process::ProcessTool;
pub use shell::Shell;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("denied: {0}")]
    Denied(String),
    #[error("tool timed out after {0:?}")]
    Timeout(Duration),
    #[error("tool was cancelled")]
    Cancelled,
    #[error("tool failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    Attended,
    Unattended,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: uuid::Uuid,
    pub execution_id: uuid::Uuid,
    pub action: String,
    pub target: String,
    pub reason: String,
    pub mode: InteractionMode,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOutcome {
    Approved,
    Denied,
    Unavailable,
}

pub struct UnattendedApprover {
    pub allow: bool,
}

#[async_trait]
impl Approver for UnattendedApprover {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        let outcome = if self.allow {
            ApprovalOutcome::Approved
        } else {
            ApprovalOutcome::Unavailable
        };
        tracing::warn!(approval_id = %request.id, execution_id = %request.execution_id,
            action = %request.action, target = %request.target, outcome = ?outcome,
            "unattended approval decided without prompting");
        outcome
    }
}

impl ApprovalOutcome {
    pub fn approved(&self) -> bool {
        *self == Self::Approved
    }
}

#[derive(Clone, Debug, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        Self {
            secrets: secrets
                .into_iter()
                .filter(|value| value.len() >= 4)
                .collect(),
        }
    }

    pub fn redact(&self, input: impl Into<String>) -> String {
        self.secrets.iter().fold(input.into(), |text, secret| {
            text.replace(secret, "[REDACTED]")
        })
    }
}

#[derive(Clone)]
pub struct ToolContext {
    pub policy: Arc<Policy>,
    pub approver: Arc<dyn Approver>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub environment: BTreeMap<String, String>,
    pub cancellation: tokio_util::sync::CancellationToken,
    pub execution_id: uuid::Uuid,
    pub interaction: InteractionMode,
    pub redactor: Arc<Redactor>,
}

impl ToolContext {
    pub fn approval(
        &self,
        action: &str,
        target: impl Into<String>,
        reason: String,
    ) -> ApprovalRequest {
        ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            execution_id: self.execution_id,
            action: action.into(),
            target: self.redactor.redact(target.into()),
            reason: self.redactor.redact(reason),
            mode: self.interaction.clone(),
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, arguments: Value, context: &ToolContext) -> Result<String, ToolError>;
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn standard() -> Self {
        let mut registry = Self::default();
        registry.register(ReadFile);
        registry.register(WriteFile);
        registry.register(ListDirectory);
        registry.register(SearchFiles);
        registry.register(Shell);
        registry.register(ApplyPatch);
        registry.register(ProcessTool::default());
        registry
    }
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.tools.insert(tool.definition().name, Arc::new(tool));
    }
    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolError> {
        let name = tool.definition().name;
        if self.tools.contains_key(&name) {
            return Err(ToolError::Failed(format!("duplicate tool name `{name}`")));
        }
        self.tools.insert(name, tool);
        Ok(())
    }
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }
    pub async fn execute(
        &self,
        name: &str,
        arguments: Value,
        context: &ToolContext,
    ) -> Result<String, ToolError> {
        let result = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::Failed(format!("unknown tool `{name}`")))?
            .execute(arguments, context)
            .await;
        result
            .map(|output| context.redactor.redact(output))
            .map_err(|error| error.redacted(&context.redactor))
    }
}

impl ToolError {
    fn redacted(self, redactor: &Redactor) -> Self {
        match self {
            Self::InvalidArguments(message) => Self::InvalidArguments(redactor.redact(message)),
            Self::Denied(message) => Self::Denied(redactor.redact(message)),
            Self::Failed(message) => Self::Failed(redactor.redact(message)),
            Self::Timeout(duration) => Self::Timeout(duration),
            Self::Cancelled => Self::Cancelled,
        }
    }
}

pub(crate) fn truncate(mut bytes: Vec<u8>, max: usize) -> String {
    if bytes.len() <= max {
        return String::from_utf8_lossy(&bytes).into_owned();
    }
    bytes.truncate(max);
    format!(
        "{}\n\n[output truncated at {max} bytes]",
        String::from_utf8_lossy(&bytes)
    )
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn redacts_all_occurrences_without_echoing_short_values() {
        let redactor = Redactor::new(["long-secret".into(), "abc".into()]);
        assert_eq!(
            redactor.redact("long-secret / long-secret / abc"),
            "[REDACTED] / [REDACTED] / abc"
        );
    }

    #[tokio::test]
    async fn unattended_denial_is_immediate_and_explicit() {
        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            execution_id: uuid::Uuid::new_v4(),
            action: "test".into(),
            target: "target".into(),
            reason: "risk".into(),
            mode: InteractionMode::Unattended,
        };
        let outcome = tokio::time::timeout(
            Duration::from_millis(50),
            UnattendedApprover { allow: false }.approve(&request),
        )
        .await
        .unwrap();
        assert_eq!(outcome, ApprovalOutcome::Unavailable);
    }

    struct DeniedTool;
    #[async_trait]
    impl Tool for DeniedTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "denied".into(),
                description: String::new(),
                input_schema: serde_json::json!({}),
            }
        }
        async fn execute(&self, _: Value, _: &ToolContext) -> Result<String, ToolError> {
            Err(ToolError::Denied("long-secret".into()))
        }
    }

    #[tokio::test]
    async fn registry_preserves_typed_errors_while_redacting() {
        let directory = tempfile::tempdir().unwrap();
        let config = crate::config::Config::default();
        let context = ToolContext {
            policy: Arc::new(Policy::new(&config, directory.path().to_owned()).unwrap()),
            approver: Arc::new(UnattendedApprover { allow: false }),
            timeout: Duration::from_secs(1),
            max_output_bytes: 1024,
            environment: BTreeMap::new(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: InteractionMode::Unattended,
            redactor: Arc::new(Redactor::new(["long-secret".into()])),
        };
        let mut registry = ToolRegistry::default();
        registry.register(DeniedTool);
        assert!(
            matches!(registry.execute("denied",serde_json::json!({}),&context).await,Err(ToolError::Denied(message)) if message=="[REDACTED]")
        );
    }
}
