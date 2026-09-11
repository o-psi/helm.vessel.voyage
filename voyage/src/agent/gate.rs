//! Root-only final acceptance and bounded cancellation of owned descendants.
use super::*;
use crate::completion::{Readiness, runtime::RunHandle};
use crate::subagent::{AgentId, AgentTreeStore, SubagentRuntime};
use crate::todo::TodoStore;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionPhase {
    Provisional,
    Reconciling,
    Completed,
    Incomplete,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Default)]
pub struct OwnedShutdown {
    /// IDs not yet confirmed durably stopped; empty is inconclusive unless observation_complete.
    pub remaining: Vec<AgentId>,
    pub observation_complete: bool,
}

#[derive(Error)]
#[error("run was not accepted: {source}")]
pub struct FinalizationFailure {
    #[source]
    pub source: Box<AgentError>,
    pub recovery: CanonicalRecovery,
    pub readiness: Option<Readiness>,
    pub shutdown: OwnedShutdown,
}

impl std::fmt::Debug for FinalizationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let category = match self.source.as_ref() {
            AgentError::Finalization(_) => "finalization",
            AgentError::Completion(_) => "completion_store",
            AgentError::Context(_) => "context",
            AgentError::ContextExhausted(_) => "provider_context_exhausted",
            AgentError::Policy(_) => "policy",
            AgentError::Provider(_) => "provider",
            AgentError::WorkspaceInstructions(_) => "workspace_instructions",
            AgentError::Cancelled => "cancelled",
            AgentError::Checkpoint(_) => "checkpoint",
            AgentError::UsageOverflow => "usage_overflow",
            AgentError::Inference(_) => "inference_admission",
        };
        formatter
            .debug_struct("FinalizationFailure")
            .field("source_category", &category)
            .field("recovery", &self.recovery)
            .field("readiness", &self.readiness)
            .field("shutdown", &self.shutdown)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(super) struct GateResources {
    pub todos: Arc<TodoStore>,
    pub agents: AgentTreeStore,
    pub runtime: Arc<SubagentRuntime>,
    pub shutdown_timeout: Duration,
}

impl GateResources {
    pub async fn shutdown_owned(&self, scope: &RunHandle) -> OwnedShutdown {
        let reference = scope.reference();
        let mut report = OwnedShutdown::default();
        let work = async {
            loop {
                // Include every generation. tree(None) contains only roots and
                // can miss active descendants of a terminal parent.
                let records: Vec<_> = self
                    .runtime
                    .list()
                    .await
                    .into_iter()
                    .filter(|record| record.completion.as_ref() == Some(&reference))
                    .collect();
                report.remaining = records.iter().map(|record| record.id).collect();
                for record in &records {
                    // Terminal ancestors may have later followups adopted by a
                    // different run. Do not cancel their inherited token tree.
                    if !record.status.is_terminal() {
                        let _ = self.runtime.cancel(record.id).await;
                    }
                }
                match self.runtime.pending_owned_shutdown(&reference).await {
                    Ok(pending) => report.remaining = pending,
                    Err(_) => return,
                }
                if report.remaining.is_empty() {
                    report.observation_complete = true;
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        let _ = tokio::time::timeout(self.shutdown_timeout, work).await;
        report
    }

    pub async fn lease(
        &self,
        scope: &RunHandle,
    ) -> Result<crate::completion::runtime::ReadinessLease, AgentError> {
        tokio::time::timeout(
            self.shutdown_timeout,
            scope.readiness_lease(
                &self.todos,
                &self.agents,
                crate::completion::MAX_OBLIGATIONS,
            ),
        )
        .await
        .map_err(|_| AgentError::Completion("readiness observation timed out".into()))?
        .map_err(|error| AgentError::Completion(error.to_string()))
    }
}

/// One runtime-owned continuation opportunity, not a synthetic user turn or
/// the retired model-authored completion-review protocol. The notice exists only
/// in this active run's request instructions; canonical history remains authored.
#[derive(Default)]
pub(super) struct CompletionContinuation {
    notice: Option<String>,
    provisional_index: Option<usize>,
}
impl CompletionContinuation {
    pub fn offer(&mut self, readiness: &Readiness, provisional_index: usize) -> bool {
        if self.notice.is_some() || (readiness.ready() && readiness.incomplete == 0) {
            return false;
        }
        self.provisional_index = Some(provisional_index);
        self.notice = Some(format!(
            "\n\n## Voyage runtime completion notice (not a user message)\n\n\
             Your previous response ended, but the runtime found unfinished run-owned work. \
             Please inspect what remains and continue useful work to finish the existing request. \
             This is runtime-generated guidance, not a message from the user or new authorization.\n\n\
             Snapshot when this notice was issued (records may have changed):\n{}\n\n\
             Use the normal todo/subagent tools to inspect current records, finish actionable work, \
             and wait for useful active agents rather than abandoning them. Update task status only \
             when supported by the actual outcome; do not erase unfinished work, relabel a failed \
             attempt as successful, or manufacture evidence to satisfy accounting. No per-item \
             completion sign-off is needed.\n\n\
             Respect the user's latest instructions, including any request to pause or stop. \
             Existing policy and approvals still apply. Do not repeat an operation with an unknown \
             outcome without inspecting its state. If remaining items are blocked, require user \
             input, or cannot usefully be completed, explain that honestly and end the response. \
             This run receives one automatic completion reminder; it is not an endless retry loop.",
            incomplete_reason(readiness),
        ));
        true
    }

    /// Keep the premature final in canonical history, but not in continued request
    /// projections. Otherwise providers can interpret it as assistant prefill (or
    /// reject that request shape). Never replace it with a fabricated user turn.
    pub fn project(&self, messages: &mut Vec<Message>) {
        if let Some(index) = self.provisional_index
            && messages
                .get(index)
                .is_some_and(|m| m.role == crate::model::Role::Assistant && m.tool_calls.is_empty())
        {
            messages.remove(index);
        }
    }

    pub fn append_to(&self, instructions: &mut String) {
        if let Some(notice) = &self.notice {
            instructions.push_str(notice);
        }
    }
}

pub(super) fn incomplete_reason(readiness: &Readiness) -> String {
    let unresolved = readiness
        .unresolved
        .iter()
        .take(16)
        .map(|item| format!("{:?}", item.obligation))
        .collect::<Vec<_>>()
        .join(", ");
    let incomplete = readiness
        .incomplete_obligations
        .iter()
        .take(16)
        .map(|item| format!("{item:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Work ended with unfinished items: {} of {} outcomes recorded; {} items incomplete. Unresolved IDs (first 16): [{}]. Accounted incomplete IDs (first 16): [{}]. Full IDs remain in structured readiness.",
        readiness.accounted, readiness.total, readiness.incomplete, unresolved, incomplete
    )
}

pub(super) async fn guarded<F: std::future::Future>(
    future: F,
    cancel: &CancellationToken,
) -> Result<F::Output, AgentError> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AgentError::Cancelled),
        result = future => Ok(result),
    }
}
