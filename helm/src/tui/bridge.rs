//! Provider-neutral event and operator-response channels.

use crate::{
    AgentEvent, EventSink,
    provider::ModelInfo,
    supervision::{AgentId, AgentInspection, AgentView},
    todo::TodoList,
    tools::{ApprovalOutcome, ApprovalRequest as ToolApprovalRequest, Approver},
};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
#[doc(hidden)]
pub enum UiEvent {
    Agent(AgentEvent),
    GithubApproval {
        session: uuid::Uuid,
        request: uuid::Uuid,
        approval: ApprovalRequest,
    },
    GithubResult {
        session: uuid::Uuid,
        request: uuid::Uuid,
        result: Result<crate::github::operator::CommandResult, String>,
    },
    InferenceStatus {
        session: uuid::Uuid,
        request: uuid::Uuid,
        result: Result<Vec<crate::inference::Status>, String>,
    },
    PolicyProfiles {
        request: uuid::Uuid,
        result: Result<
            (
                std::path::PathBuf,
                Vec<crate::policy_profile::store::ProfileSnapshot>,
            ),
            String,
        >,
    },
    PolicyPreview {
        request: uuid::Uuid,
        result: Result<
            (
                crate::policy_profile::switching::Target,
                Box<crate::policy_profile::switching::SwitchPreview>,
            ),
            String,
        >,
    },
    Checkpoint(super::checkpoint::Request),
    Approval(ApprovalRequest),
    Question(QuestionRequest),
    Finished(Result<crate::AgentOutcome, crate::agent::AgentError>),
    TitleReady {
        session_id: uuid::Uuid,
        completed_runs: u64,
        result: Option<crate::titles::TitleResult>,
    },
    SupervisorTree(Result<Vec<AgentView>, String>),
    SupervisorInspect(AgentId, Result<AgentInspection, String>),
    SupervisorAction(Result<String, String>),
    TodoSnapshot(Result<TodoList, String>),
    TodoAction(Result<String, String>),
    Models(Result<Vec<ModelInfo>, String>),
    Workflows {
        request: uuid::Uuid,
        definitions: Result<Vec<crate::workflow::Definition>, String>,
    },
}

#[derive(Debug)]
#[doc(hidden)]
pub struct ApprovalRequest {
    pub(super) id: uuid::Uuid,
    pub(super) action: String,
    pub(super) target: String,
    pub(super) reason: String,
    pub(super) response: oneshot::Sender<ApprovalOutcome>,
}

#[derive(Debug)]
#[doc(hidden)]
pub struct QuestionRequest {
    pub(super) question: crate::tools::Question,
    pub(super) response: oneshot::Sender<crate::tools::QuestionAnswer>,
}

#[derive(Clone)]
pub struct UiBridge {
    pub(super) tx: mpsc::UnboundedSender<UiEvent>,
}

#[async_trait]
impl EventSink for UiBridge {
    async fn emit(&self, event: AgentEvent) {
        let _ = self.tx.send(UiEvent::Agent(event));
    }
}

#[async_trait]
impl Approver for UiBridge {
    async fn ask_question(
        &self,
        question: &crate::tools::Question,
    ) -> crate::tools::QuestionAnswer {
        let (response, receive) = oneshot::channel();
        if self
            .tx
            .send(UiEvent::Question(QuestionRequest {
                question: question.clone(),
                response,
            }))
            .is_err()
        {
            return crate::tools::QuestionAnswer::Unavailable;
        }
        receive
            .await
            .unwrap_or(crate::tools::QuestionAnswer::Unavailable)
    }

    async fn approve(&self, request: &ToolApprovalRequest) -> ApprovalOutcome {
        let (response, receive) = oneshot::channel();
        if self
            .tx
            .send(UiEvent::Approval(ApprovalRequest {
                id: request.id,
                action: request.action.clone(),
                target: request.target.clone(),
                reason: request.reason.clone(),
                response,
            }))
            .is_err()
        {
            return ApprovalOutcome::Unavailable;
        }
        let outcome = receive.await.unwrap_or(ApprovalOutcome::Unavailable);
        tracing::info!(approval_id = %request.id, execution_id = %request.execution_id,
            action = %request.action, target = %request.target, outcome = ?outcome,
            "approval decided");
        outcome
    }
}

impl UiBridge {
    #[doc(hidden)]
    pub fn sender(&self) -> mpsc::UnboundedSender<UiEvent> {
        self.tx.clone()
    }
}

/// The pair passed into an Agent and [`super::run`] respectively.
pub fn bridge() -> (Arc<UiBridge>, mpsc::UnboundedReceiver<UiEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (Arc::new(UiBridge { tx }), rx)
}
