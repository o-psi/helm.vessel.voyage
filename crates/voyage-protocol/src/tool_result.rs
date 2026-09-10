//! Ordered tool results. Artifact references are scoped to the executing session.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub id: Uuid,
    pub sha256: String,
    /// Display label only; never used as a path.
    pub name: String,
    /// Server-declared MIME type, not proof of safe or decodable content.
    pub mime_type: String,
    pub byte_size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolContent {
    Text {
        text: String,
    },
    Image {
        artifact: ArtifactReference,
    },
    Audio {
        artifact: ArtifactReference,
    },
    Resource {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<ArtifactReference>,
    },
    ResourceLink {
        uri: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolOutput {
    pub content: Vec<ToolContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(default)]
    pub is_error: bool,
}
impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text { text: text.into() }],
            ..Self::default()
        }
    }
    /// Deterministic adapter for text-only providers; never includes binary bytes.
    /// The full typed result remains canonical in persisted history.
    pub fn text_fallback(&self) -> String {
        let mut blocks: Vec<String> = self
            .content
            .iter()
            .map(|part| match part {
                ToolContent::Text { text } => text.clone(),
                other => serde_json::to_string(other)
                    .unwrap_or_else(|_| "[unavailable tool content]".into()),
            })
            .collect();
        if let Some(value) = &self.structured_content {
            blocks.push(serde_json::json!({"structuredContent": value}).to_string());
        }
        blocks.join("\n")
    }
    pub fn artifacts(&self) -> impl Iterator<Item = &ArtifactReference> {
        self.content.iter().filter_map(|part| match part {
            ToolContent::Image { artifact } | ToolContent::Audio { artifact } => Some(artifact),
            ToolContent::Resource { artifact, .. } => artifact.as_ref(),
            _ => None,
        })
    }
}

/// Additive sibling metadata on messages, not a field of strict ToolOutput.
/// None on an old message means unclassified, not verified success/completeness.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolOutcome {
    /// Monotonic invocation duration, including policy waits, excluding persistence.
    /// Absent for legacy history and calls without an observed result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    pub execution: ExecutionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<IncompleteReason>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOutcome {
    #[default]
    Succeeded,
    ExecutionError,
    PolicyRefused,
    Cancelled,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommandOutcome {
    Exited { code: i64 },
    Signalled,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IncompleteReason {
    OutputLimit,
    CaptureLimit,
    Withheld,
}
impl ToolOutcome {
    /// Coarse compatibility flag; structured command and completeness facts remain
    /// independent. This never asserts rollback, cleanup or independent-work completion.
    pub fn success(&self) -> bool {
        self.execution == ExecutionOutcome::Succeeded
            && self.incomplete.is_none()
            && matches!(
                self.command,
                None | Some(CommandOutcome::Exited { code: 0 })
            )
    }
    pub fn label(&self) -> &'static str {
        match self.execution {
            ExecutionOutcome::PolicyRefused => "Refused",
            ExecutionOutcome::Cancelled => "Cancelled",
            ExecutionOutcome::Unknown => "Unconfirmed",
            ExecutionOutcome::ExecutionError => "Execution error",
            ExecutionOutcome::Succeeded => match self.command {
                Some(CommandOutcome::Exited { code }) if code != 0 => "Command failed",
                Some(CommandOutcome::Signalled) => "Command signalled",
                _ if self.incomplete.is_some() => "Output incomplete",
                _ => "Done",
            },
        }
    }
}
