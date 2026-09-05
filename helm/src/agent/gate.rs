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
    Interrupted,
}

#[derive(Clone, Debug, Default)]
pub struct OwnedShutdown {
    /// Last observed active IDs; empty is inconclusive unless observation_complete.
    pub remaining: Vec<AgentId>,
    pub observation_complete: bool,
}

#[derive(Debug, Error)]
#[error("run was not accepted: {source}")]
pub struct FinalizationFailure {
    #[source]
    pub source: Box<AgentError>,
    pub recovery: CanonicalRecovery,
    pub readiness: Option<Readiness>,
    pub shutdown: OwnedShutdown,
}

#[derive(Clone)]
pub(super) struct GateResources {
    pub todos: Arc<TodoStore>,
    pub agents: AgentTreeStore,
    pub runtime: Arc<SubagentRuntime>,
    pub reconciliation_timeout: Duration,
    pub shutdown_timeout: Duration,
}

impl GateResources {
    pub async fn shutdown_owned(&self, scope: &RunHandle) -> OwnedShutdown {
        let reference = scope.reference();
        let mut report = OwnedShutdown::default();
        let work = async {
            loop {
                report.remaining = self
                    .runtime
                    .tree(None)
                    .await
                    .into_iter()
                    .filter(|record| {
                        record.completion.as_ref() == Some(&reference)
                            && !record.status.is_terminal()
                    })
                    .map(|record| record.id)
                    .collect();
                if report.remaining.is_empty() {
                    report.observation_complete = true;
                    return;
                }
                for id in &report.remaining {
                    // A cancellation request is not proof of termination. The next
                    // iteration observes persisted runtime state before claiming it.
                    let _ = self.runtime.cancel(*id).await;
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

pub(super) fn reconciliation_prompt(readiness: &Readiness) -> String {
    let mut readiness = readiness.clone();
    readiness.omitted_unresolved += readiness.unresolved.len().saturating_sub(32);
    readiness.unresolved.truncate(32);
    readiness.incomplete_obligations.truncate(32);
    format!(
        "Helm has withheld final acceptance. This is the one bounded reconciliation pass for the current run. Review the owned obligation IDs and states below using the completion tool. Read and incorporate useful agent results; wait for useful active agents or cancel unnecessary work with an explicit reason. Verify todo evidence. Preserve blocked or deferred statuses and account for their impact truthfully; never mark every item completed merely to pass the check. Failed/cancelled/interrupted children are not success. Do not delete, archive, unassign, hide, or broaden authority to evade obligations. Use normal policy-controlled tools and produce a revised final only after reconciliation. The next final proposal is rechecked against fresh state; unresolved or accounted-incomplete work yields an incomplete outcome.\n{}",
        serde_json::to_string(&readiness).expect("readiness is serializable")
    )
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
        "Run incomplete: {} of {} obligations accounted for; {} accounted obligations incomplete. Unresolved IDs (first 16): [{}]. Accounted incomplete IDs (first 16): [{}]. Full IDs remain in structured readiness.",
        readiness.accounted, readiness.total, readiness.incomplete, unresolved, incomplete
    )
}

pub(super) async fn guarded<F: std::future::Future>(
    future: F,
    cancel: &CancellationToken,
    deadline: Option<tokio::time::Instant>,
) -> Result<F::Output, AgentError> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AgentError::Cancelled),
        _ = async { match deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } } => Err(AgentError::ReconciliationExpired),
        result = future => Ok(result),
    }
}
