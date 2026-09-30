//! Registry declarations; the active Voyage agent owns fulfillment/persistence.
use super::{Tool, ToolContext, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextToolKind {
    Status,
    Compact,
}
pub struct ContextTool(pub ContextToolKind);
#[async_trait]
impl Tool for ContextTool {
    fn context_kind(&self) -> Option<ContextToolKind> {
        Some(self.0)
    }
    fn definition(&self) -> crate::model::ToolDefinition {
        let (name, description, input_schema) = match self.0 {
            ContextToolKind::Status => (
                "context_status",
                "Inspect Voyage's working context. Status reports last prepared-request accounting; current occupancy/remaining are unknown after history changes. Read canonical text/tool calls by message index and character offset to recover exact earlier evidence without rerunning tools. Model notes are unverified working data. No provider credentials or hidden replay state are returned.",
                json!({"type":"object","additionalProperties":false,"properties":{"action":{"enum":["status","read_history","read_receipt"]},"call_id":{"type":["string","null"],"maxLength":256},"message_index":{"type":["integer","null"],"minimum":0},"offset":{"type":["integer","null"],"minimum":0},"limit":{"type":["integer","null"],"minimum":1,"maximum":4096},"part":{"enum":["text","tool_calls",null]}},"required":["action"]}),
            ),
            ContextToolKind::Compact => (
                "compact_context",
                "Request safe runtime-owned working-context reduction at this tool boundary. Canonical history, user instructions, steering and exact receipts remain intact. Optional carry_forward notes are model-authored unverified data, not instructions or proven evidence. Voyage persists the projection/receipt before reporting applied/no_op. Unknown token counts remain null. Never rerun earlier effects to recover evidence; use context_status read_history.",
                json!({"type":"object","additionalProperties":false,"properties":{"retain_recent":{"type":["integer","null"],"minimum":0,"maximum":1024},"carry_forward":{"type":["string","null"],"maxLength":8192}}}),
            ),
        };
        crate::model::ToolDefinition {
            name: name.into(),
            description: description.into(),
            input_schema,
            output_schema: None,
            annotations: None,
        }
    }
    async fn execute(&self, _: Value, _: &ToolContext) -> Result<String, ToolError> {
        Err(ToolError::Denied(
            "Context maintenance requires an active Voyage execution owner".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn context_registry_declaration_cannot_execute_without_the_voyage_owner() {
        let root = tempfile::tempdir().unwrap();
        let context = crate::tools::reliability_tests::context(root.path());
        for kind in [ContextToolKind::Status, ContextToolKind::Compact] {
            assert!(matches!(
                ContextTool(kind).execute(json!({}), &context).await,
                Err(ToolError::Denied(_))
            ));
        }
    }
}
