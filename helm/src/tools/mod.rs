mod filesystem;
pub mod mcp;
mod process;
mod shell;

use async_trait::async_trait;
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
    async fn approve(&self, reason: &str) -> bool;
}

#[derive(Clone)]
pub struct ToolContext {
    pub policy: Arc<Policy>,
    pub approver: Arc<dyn Approver>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub environment: BTreeMap<String, String>,
    pub cancellation: tokio_util::sync::CancellationToken,
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
    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.definition().name, tool);
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
        self.tools
            .get(name)
            .ok_or_else(|| ToolError::Failed(format!("unknown tool `{name}`")))?
            .execute(arguments, context)
            .await
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
