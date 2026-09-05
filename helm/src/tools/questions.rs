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
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QuestionAnswer {
    Selected { index: usize, answer: String },
    Custom { answer: String },
    Cancelled,
    Unavailable,
}

impl QuestionAnswer {
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

pub struct Questions;

#[async_trait]
impl Tool for Questions {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ApprovalOutcome, ApprovalRequest, Approver, Redactor, ToolRegistry};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    struct Frontend {
        answer: QuestionAnswer,
        calls: AtomicUsize,
        hang: bool,
    }
    #[async_trait]
    impl Approver for Frontend {
        async fn approve(&self, _: &ApprovalRequest) -> ApprovalOutcome {
            panic!("questions must never request approval")
        }
        async fn ask_question(&self, _: &Question) -> QuestionAnswer {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.hang {
                std::future::pending::<()>().await;
            }
            self.answer.clone()
        }
    }
    fn args() -> Value {
        json!({"question":"Which format?", "options":["JSON", "Markdown"]})
    }
    fn context(
        path: &std::path::Path,
        answer: QuestionAnswer,
        hang: bool,
    ) -> (ToolContext, Arc<Frontend>) {
        let frontend = Arc::new(Frontend {
            answer,
            calls: AtomicUsize::new(0),
            hang,
        });
        let config = crate::Config {
            access: Some(crate::config::AccessMode::ReadOnly),
            ..Default::default()
        };
        (
            ToolContext {
                policy: Arc::new(crate::policy::Policy::new(&config, path.into()).unwrap()),
                approver: frontend.clone(),
                timeout: Duration::from_millis(25),
                max_output_bytes: 4096,
                environment: Default::default(),
                cancellation: Default::default(),
                execution_id: uuid::Uuid::new_v4(),
                interaction: InteractionMode::Attended,
                redactor: Arc::new(Redactor::default()),
            },
            frontend,
        )
    }

    #[tokio::test]
    async fn questions_selected_custom_cancelled_and_unavailable_are_structured_read_only_results()
    {
        let dir = tempfile::tempdir().unwrap();
        for answer in [
            QuestionAnswer::Selected {
                index: 1,
                answer: "Markdown".into(),
            },
            QuestionAnswer::Custom {
                answer: "日本語 🛶".into(),
            },
            QuestionAnswer::Cancelled,
            QuestionAnswer::Unavailable,
        ] {
            let (ctx, frontend) = context(dir.path(), answer.clone(), false);
            let mut registry = ToolRegistry::standard();
            registry.retain_read_only();
            let result = registry.execute("questions", args(), &ctx).await.unwrap();
            assert_eq!(
                serde_json::from_str::<QuestionAnswer>(&result).unwrap(),
                answer
            );
            assert_eq!(frontend.calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn questions_reject_malformed_and_adversarial_arguments_before_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, frontend) = context(dir.path(), QuestionAnswer::Unavailable, false);
        for invalid in [
            json!(null),
            json!({}),
            json!({"question":"q","options":["a","b"],"answer":"a"}),
            json!({"question":" ","options":["a","b"]}),
            json!({"question":"q","options":["a"]}),
            json!({"question":"q","options":["a"," a "]}),
            json!({"question":"q","options":["a",""]}),
            json!({"question":"q","options":["a","\u{001b}[2J"]}),
            json!({"question":"q".repeat(2049),"options":["a","b"]}),
            json!({"question":"q","options":["a","b".repeat(257)]}),
            json!({"question":"q","options":(0..9).map(|i|i.to_string()).collect::<Vec<_>>()}),
        ] {
            assert!(matches!(
                Questions.execute(invalid, &ctx).await,
                Err(ToolError::InvalidArguments(_))
            ));
        }
        assert_eq!(frontend.calls.load(Ordering::SeqCst), 0);
        let boundary = Question {
            question: "q".repeat(2048),
            options: (0..8).map(|i| format!("{i}{}", "x".repeat(255))).collect(),
        };
        assert!(boundary.validate().is_ok());
    }

    #[tokio::test]
    async fn questions_unattended_never_dispatches_even_with_a_capable_frontend() {
        let dir = tempfile::tempdir().unwrap();
        let (mut ctx, frontend) = context(
            dir.path(),
            QuestionAnswer::Custom {
                answer: "never".into(),
            },
            true,
        );
        ctx.interaction = InteractionMode::Unattended;
        assert_eq!(
            Questions.execute(args(), &ctx).await.unwrap(),
            "{\"status\":\"unavailable\"}"
        );
        assert_eq!(frontend.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn questions_timeout_and_cancellation_are_bounded_and_cancellation_wins() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, frontend) = context(dir.path(), QuestionAnswer::Unavailable, true);
        assert!(matches!(
            Questions.execute(args(), &ctx).await,
            Err(ToolError::Timeout(_))
        ));
        let cancel = ctx.cancellation.clone();
        let future = Questions.execute(args(), &ctx);
        let trigger = async {
            tokio::time::sleep(Duration::from_millis(1)).await;
            cancel.cancel();
        };
        let (result, _) = tokio::join!(future, trigger);
        assert!(matches!(result, Err(ToolError::Cancelled)));
        assert!(matches!(
            Questions.execute(args(), &ctx).await,
            Err(ToolError::Cancelled)
        ));
        assert_eq!(frontend.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn questions_reject_invalid_frontend_results_and_redact_known_secrets() {
        let dir = tempfile::tempdir().unwrap();
        for answer in [
            QuestionAnswer::Selected {
                index: 9,
                answer: "JSON".into(),
            },
            QuestionAnswer::Selected {
                index: 0,
                answer: "forged".into(),
            },
            QuestionAnswer::Custom { answer: " ".into() },
            QuestionAnswer::Custom {
                answer: "x".repeat(MAX_ANSWER_BYTES + 1),
            },
            QuestionAnswer::Custom {
                answer: "\u{001b}[2J".into(),
            },
        ] {
            let (ctx, _) = context(dir.path(), answer, false);
            assert!(matches!(
                Questions.execute(args(), &ctx).await,
                Err(ToolError::Failed(_))
            ));
        }
        let (mut ctx, _) = context(
            dir.path(),
            QuestionAnswer::Custom {
                answer: "known-secret".into(),
            },
            false,
        );
        ctx.redactor = Arc::new(Redactor::new(["known-secret".into()]));
        let result = ToolRegistry::standard()
            .execute("questions", args(), &ctx)
            .await
            .unwrap();
        assert!(!result.contains("known-secret"));
        assert!(result.contains("[REDACTED]"));
    }
    struct QuestionProvider {
        expected: QuestionAnswer,
    }
    #[async_trait]
    impl crate::provider::Provider for QuestionProvider {
        async fn complete(
            &self,
            request: crate::model::ModelRequest,
        ) -> Result<crate::model::ModelResponse, crate::provider::ProviderError> {
            use crate::model::{Message, ModelResponse, Role, ToolCall};
            assert!(request.tools.iter().any(|t| t.name == "questions"));
            let message =
                if let Some(result) = request.messages.iter().find(|m| m.role == Role::Tool) {
                    assert_eq!(result.tool_call_id.as_deref(), Some("question-1"));
                    assert_eq!(result.tool_success, Some(true));
                    assert_eq!(
                        serde_json::from_str::<QuestionAnswer>(&result.content).unwrap(),
                        self.expected
                    );
                    Message::new(Role::Assistant, "Answer received")
                } else {
                    let mut message = Message::new(Role::Assistant, "");
                    message.tool_calls.push(ToolCall {
                        id: "question-1".into(),
                        name: "questions".into(),
                        arguments: args(),
                    });
                    message
                };
            Ok(ModelResponse {
                message,
                usage: Default::default(),
            })
        }
    }

    #[tokio::test]
    async fn questions_provider_loop_continues_and_answers_survive_session_round_trip() {
        use crate::session::{Session, SessionStore};
        for answer in [
            QuestionAnswer::Selected {
                index: 0,
                answer: "JSON".into(),
            },
            QuestionAnswer::Custom {
                answer: "CSV".into(),
            },
            QuestionAnswer::Cancelled,
            QuestionAnswer::Unavailable,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (ctx, _) = context(dir.path(), answer.clone(), false);
            let agent = crate::Agent::new(
                Box::new(QuestionProvider {
                    expected: answer.clone(),
                }),
                ToolRegistry::standard(),
                ctx,
                Arc::new(crate::agent::SilentSink),
                "fixture".into(),
                "Ask for clarification".into(),
                1024,
                None,
            );
            let outcome = agent.run(vec![], "Choose a format".into()).await.unwrap();
            assert_eq!(outcome.turns, 2);
            assert_eq!(outcome.answer, "Answer received");
            let mut session = Session::new(dir.path().into(), "fixture".into());
            session.messages = outcome.messages;
            let store = SessionStore::new(dir.path().join("sessions"));
            store.save(&mut session).await.unwrap();
            let restored = store.load(session.id).await.unwrap();
            let result = restored
                .messages
                .iter()
                .find(|m| m.role == crate::Role::Tool)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<QuestionAnswer>(&result.content).unwrap(),
                answer
            );
        }
    }
    #[tokio::test]
    async fn questions_redacts_json_escaped_known_secrets_without_breaking_answers() {
        let dir = tempfile::tempdir().unwrap();
        for secret in [
            r#"private"quote"#,
            r"private\path",
            "日本語\"secret",
            r#"private\"combined"#,
        ] {
            for selected in [false, true] {
                let answer = if selected {
                    QuestionAnswer::Selected {
                        index: 1,
                        answer: secret.into(),
                    }
                } else {
                    QuestionAnswer::Custom {
                        answer: format!("before {secret} after"),
                    }
                };
                let (mut ctx, _) = context(dir.path(), answer, false);
                ctx.redactor = Arc::new(Redactor::new([secret.to_owned()]));
                let result = ToolRegistry::standard()
                    .execute(
                        "questions",
                        json!({"question":"Which format?", "options":["public",secret]}),
                        &ctx,
                    )
                    .await
                    .unwrap();
                let decoded: QuestionAnswer = serde_json::from_str(&result).unwrap();
                let expected = if selected {
                    QuestionAnswer::Selected {
                        index: 1,
                        answer: "[REDACTED]".into(),
                    }
                } else {
                    QuestionAnswer::Custom {
                        answer: "before [REDACTED] after".into(),
                    }
                };
                assert_eq!(decoded, expected);
            }
        }
    }
}
