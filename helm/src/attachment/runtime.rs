//! Run-owned integration of the journal and provider-neutral agent loop.
//!
//! This is local-only. Callers must perform authentication/sharing checks before
//! admission and retain the owner until their agent/terminal/subagent cleanup has
//! finished. It does not reconcile the existing JSON SessionStore or enable a port.
use super::journal::{ExecutionGuard, Journal, RunRecord, RunState, TurnAdmission};
use crate::{
    agent::{
        Agent, AgentError, AgentOutcome, CheckpointError, RunCheckpoint, SteeringReceiver,
        StopReason,
    },
    model::{Message, Usage},
};
use async_trait::async_trait;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Lifetime owner for one managed session, including time between turns.
#[derive(Clone)]
pub struct ManagedSessionOwner;
impl ManagedSessionOwner {
    pub async fn open(_directory: PathBuf, _session_id: Uuid) -> anyhow::Result<Self> {
        anyhow::bail!("managed session owner not implemented")
    }
    pub async fn snapshot(&self) -> anyhow::Result<super::journal::VersionedSession> {
        anyhow::bail!("managed session snapshot not implemented")
    }
}

struct Store {
    journal: Journal,
    guard: ExecutionGuard,
    run_id: Uuid,
}

/// A retry retrieves existing evidence; it cannot create a runnable owner.
pub enum Admission {
    New(RunOwner),
    Existing(RunRecord),
}

pub struct RunOwner {
    store: Arc<Mutex<Store>>,
    run_id: Uuid,
    workspace: PathBuf,
    model: String,
    input: Option<(Vec<Message>, String)>,
}

impl RunOwner {
    /// Own storage before reading the authoritative history. All SQLite work is
    /// offloaded from the async reactor, including open/admission and finalization.
    pub async fn admit(
        directory: PathBuf,
        request: TurnAdmission,
        now_ms: i64,
    ) -> anyhow::Result<Admission> {
        tokio::task::spawn_blocking(move || {
            let mut journal = Journal::open(directory)?;
            if let Some(run) = journal.lookup_command(&request)? {
                return Ok(Admission::Existing(run));
            }
            let guard = journal.acquire_execution(request.session_id)?;
            let session = journal.load_session(request.session_id)?.session;
            let workspace = session.workspace.canonicalize()?;
            let admission = journal.admit_turn(&guard, &request, now_ms)?;
            if admission.duplicate {
                return Ok(Admission::Existing(admission.run));
            }
            let run_id = admission.run.id;
            Ok(Admission::New(Self {
                store: Arc::new(Mutex::new(Store {
                    journal,
                    guard,
                    run_id,
                })),
                run_id,
                workspace,
                model: session.model,
                input: Some((session.messages, request.prompt)),
            }))
        })
        .await?
    }

    async fn storage<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Store) -> anyhow::Result<T> + Send + 'static,
    ) -> Result<T, CheckpointError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = store.lock().map_err(|_| CheckpointError)?;
            operation(&mut store).map_err(|_| CheckpointError)
        })
        .await
        .map_err(|_| CheckpointError)?
    }

    pub async fn record(&self) -> Result<RunRecord, CheckpointError> {
        self.storage(|store| store.journal.run(store.run_id)).await
    }

    /// Execute at most once. Even after error this owner cannot dispatch again.
    /// The owner remains borrowed/held after return, so its guard is not released
    /// before the caller completes owned-resource cleanup.
    pub async fn execute(
        &mut self,
        agent: &Agent,
        cancel: CancellationToken,
        input: Option<SteeringReceiver>,
    ) -> Result<AgentOutcome, AgentError> {
        let (history, prompt) = self.input.take().ok_or(CheckpointError)?;
        let result = if cancel.is_cancelled() {
            Err(AgentError::Cancelled)
        } else if agent.workspace() != self.workspace || agent.model() != self.model {
            Err(CheckpointError.into())
        } else {
            async {
                self.storage(|store| store.journal.mark_running(&store.guard, store.run_id))
                    .await?;
                let session = self
                    .storage(|store| {
                        let run = store.journal.run(store.run_id)?;
                        Ok(store.journal.load_session(run.session_id)?.session)
                    })
                    .await?;
                let scope = agent.prepare_run_with_id(&session, self.run_id).await?;
                if let Some(scope) = &scope {
                    let reference = scope.reference();
                    self.storage(move |store| {
                        store
                            .journal
                            .register_run_scope(&store.guard, store.run_id, reference)
                    })
                    .await?;
                }
                agent
                    .run_checkpointed_scoped(
                        history,
                        prompt,
                        cancel.clone(),
                        input,
                        self,
                        self.model.clone(),
                        scope,
                    )
                    .await
            }
            .await
        };
        // Cancellation wins over a late provider completion. A failed terminal
        // commit returns an error, never the otherwise-successful model outcome.
        let result = if cancel.is_cancelled() {
            Err(AgentError::Cancelled)
        } else {
            result
        };
        let (state, reason) = match &result {
            Ok(outcome) => match &outcome.stop_reason {
                StopReason::Completed => (RunState::Completed, None),
                StopReason::Incomplete { .. } => (
                    RunState::Incomplete,
                    Some("completion reconciliation incomplete"),
                ),
            },
            Err(AgentError::Finalization(failure))
                if matches!(&*failure.source, AgentError::Cancelled) =>
            {
                (
                    RunState::Cancelled,
                    Some("run cancelled during completion reconciliation"),
                )
            }
            Err(AgentError::Cancelled) => (RunState::Cancelled, Some("run cancelled")),
            Err(AgentError::Checkpoint(_)) => (RunState::Failed, Some("durable checkpoint failed")),
            Err(_) => (RunState::Failed, Some("provider or runtime failed")),
        };
        let classification = result
            .as_ref()
            .ok()
            .map(|outcome| outcome.stop_reason.clone());
        self.storage(move |store| {
            store.journal.finish_classified(
                &store.guard,
                store.run_id,
                state,
                reason,
                None,
                classification.as_ref(),
            )
        })
        .await?;
        result
    }
}

#[async_trait]
impl RunCheckpoint for RunOwner {
    fn run_id(&self) -> Uuid {
        self.run_id
    }
    async fn canonical(&self, messages: &[Message], usage: &Usage) -> Result<(), CheckpointError> {
        let messages = messages.to_vec();
        let usage = usage.clone();
        self.storage(move |store| {
            store
                .journal
                .checkpoint_canonical(&store.guard, store.run_id, &messages, &usage)
        })
        .await
    }
    async fn accepted(
        &self,
        messages: &[Message],
        usage: &Usage,
        reason: &StopReason,
    ) -> Result<(), CheckpointError> {
        if matches!(reason, StopReason::Completed) {
            let messages = messages.to_vec();
            let usage = usage.clone();
            self.storage(move |store| {
                store
                    .journal
                    .accept_checkpoint(&store.guard, store.run_id, &messages, &usage)
            })
            .await?;
        }
        Ok(())
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        let text = text.to_owned();
        self.storage(move |store| {
            store
                .journal
                .append_text(&store.guard, store.run_id, &text)
                .map(|_| ())
        })
        .await
    }
}

#[cfg(test)]
mod tests;
