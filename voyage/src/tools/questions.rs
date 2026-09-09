//! Model-visible clarification, never an authorization or secret-input channel.
use super::{InteractionMode, Tool, ToolContext, ToolError};
use crate::model::ToolDefinition;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const MAX_ANSWER_BYTES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub question: String,
    pub options: Vec<String>,
}

impl Question {
    pub fn validate(&self) -> Result<(), ToolError> {
        let valid_text = |s: &str, max: usize| {
            !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
        };
        if !valid_text(&self.question, 2048) {
            return Err(ToolError::InvalidArguments(
                "question must be nonblank, at most 2048 bytes, and contain no control characters"
                    .into(),
            ));
        }
        if !(2..=8).contains(&self.options.len()) {
            return Err(ToolError::InvalidArguments(
                "provide 2 to 8 options; custom input is added automatically".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for option in &self.options {
            if !valid_text(option, 256) || !seen.insert(option.trim()) {
                return Err(ToolError::InvalidArguments("options must be distinct, nonblank, at most 256 bytes each, and contain no control characters".into()));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionAnswer {
    Selected { index: usize, answer: String },
    Custom { answer: String },
    Cancelled,
    Unavailable,
}

impl QuestionAnswer {
    fn redact(&mut self, redactor: &super::Redactor) {
        match self {
            Self::Selected { answer, .. } | Self::Custom { answer } => {
                *answer = redactor.redact(std::mem::take(answer));
            }
            Self::Cancelled | Self::Unavailable => {}
        }
    }
    fn validate(&self, question: &Question) -> Result<(), ToolError> {
        match self {
            Self::Selected { index, answer } if question.options.get(*index) != Some(answer) => {
                Err(ToolError::Failed("question response does not match an offered option".into()))
            }
            Self::Custom { answer } if answer.trim().is_empty()
                || answer.len() > MAX_ANSWER_BYTES
                || answer.chars().any(char::is_control) => {
                Err(ToolError::Failed("custom answer must be nonblank, at most 4096 bytes, and contain no control characters".into()))
            }
            _ => Ok(()),
        }
    }
}

/// Fixed schema metadata is public protocol data; only answer fields contain
/// frontend-supplied text. Never run blind replacement over the encoded envelope.
pub(super) fn redact_result(output: &str, redactor: &super::Redactor) -> Result<String, ToolError> {
    let invalid = || ToolError::Failed("questions returned an invalid structured response".into());
    let mut answer: QuestionAnswer = serde_json::from_str(output).map_err(|_| invalid())?;
    // Serde's internally tagged unit variants ignore extra fields even with
    // deny_unknown_fields. Enforce the complete envelope for every variant.
    let fields: &[&str] = match &answer {
        QuestionAnswer::Selected { .. } => &["status", "index", "answer"],
        QuestionAnswer::Custom { .. } => &["status", "answer"],
        QuestionAnswer::Cancelled | QuestionAnswer::Unavailable => &["status"],
    };
    let value: Value = serde_json::from_str(output).map_err(|_| invalid())?;
    if !value.as_object().is_some_and(|object| {
        object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
    }) {
        return Err(invalid());
    }
    answer.redact(redactor);
    serde_json::to_string(&answer)
        .map_err(|_| ToolError::Failed("could not encode question response".into()))
}

pub struct Questions;

#[async_trait]
impl Tool for Questions {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            output_schema: None,
            annotations: None,
            name: "questions".into(),
            description: "Ask the user one multiple-choice clarification question (2–8 options). A custom text answer is always available. Returns selected (zero-based index and answer), custom, cancelled, or unavailable. Supported in the full-screen TUI; other/unattended frontends return unavailable. Answers are sent to the model and saved in session history: never request secrets or use this as a security approval. Do not invent an answer when cancelled or unavailable.".into(),
            input_schema: json!({"type":"object","properties":{
                "question":{"type":"string","minLength":1,"maxLength":2048},
                "options":{"type":"array","minItems":2,"maxItems":8,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":256}}
            },"required":["question","options"],"additionalProperties":false}),
        }
    }

    async fn execute(&self, arguments: Value, context: &ToolContext) -> Result<String, ToolError> {
        let question: Question = serde_json::from_value(arguments)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        question.validate()?;
        if context.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let answer = if context.interaction == InteractionMode::Unattended {
            QuestionAnswer::Unavailable
        } else {
            tokio::select! {
                biased;
                _ = context.cancellation.cancelled() => return Err(ToolError::Cancelled),
                result = tokio::time::timeout(context.timeout, context.approver.ask_question(&question)) => {
                    result.map_err(|_| ToolError::Timeout(context.timeout))?
                }
            }
        };
        answer.validate(&question)?;
        serde_json::to_string(&answer).map_err(|e| ToolError::Failed(e.to_string()))
    }
}
