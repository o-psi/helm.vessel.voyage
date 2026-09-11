//! Explicit human operator actions use the same exclusive durable run owner.
use super::*;
impl RunOwner {
    /// Dispatch once, and account for errors from every operator phase. Cleanup
    /// remains separately owned by the execution driver even if persistence fails.
    pub(crate) async fn execute_operator(
        &mut self,
        agent: &Agent,
        cancel: CancellationToken,
        name: &str,
        arguments: serde_json::Value,
        redactor: Arc<crate::tools::Redactor>,
    ) -> anyhow::Result<()> {
        let mut reason = "Operator action failed during setup.";
        let result: anyhow::Result<()> = async {
            anyhow::ensure!(!cancel.is_cancelled(), "operator action cancelled");
            self.start_operator_scope(agent).await?;
            agent
                .operator_run_lifecycle(self.run_id, cancel.clone(), "run_start")
                .await;
            reason = "Operator tool failed.";
            let session_id = self.record().await?.session_id;
            let text = self
                .checkpoint()
                .with_title_updates(
                    agent,
                    cancel.clone(),
                    agent.operator_tool(session_id, self.run_id, cancel.clone(), name, arguments),
                )
                .await?;
            reason = "Operator action failed during finalization.";
            anyhow::ensure!(!cancel.is_cancelled(), "operator action cancelled");
            agent
                .operator_run_lifecycle(self.run_id, cancel.clone(), "run_finish")
                .await;
            anyhow::ensure!(!cancel.is_cancelled(), "operator action cancelled");
            let lease = agent.operator_lease(session_id, self.run_id).await?;
            self.finish_operator_scoped(redactor.redact(text), lease, cancel.clone())
                .await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            // Only authored phase labels are public. A late error must not
            // overwrite an already committed outcome or retry the tool effect.
            self.storage(move |store| {
                let run = store.journal.run(store.run_id)?;
                if !matches!(run.state, RunState::Accepted | RunState::Running) {
                    return Ok(run);
                }
                let (state, reason) = if cancel.is_cancelled() {
                    (RunState::Cancelled, "operator action cancelled")
                } else {
                    (RunState::Failed, reason)
                };
                store
                    .journal
                    .finish(&store.guard, store.run_id, state, Some(reason), None)
            })
            .await?;
        }
        result
    }

    pub(crate) async fn start_operator(&mut self) -> Result<(), CheckpointError> {
        let _input = self.input.take().ok_or(CheckpointError)?;
        self.steering_receiver.take();
        let authority = self.token.execution_authority.clone();
        self.storage(move |store| {
            if let Some(authority) = authority {
                authority.check()?;
            }
            store.journal.mark_running(&store.guard, store.run_id)?;
            let history = store
                .journal
                .load_session(store.session_id)?
                .session
                .messages;
            store.journal.checkpoint_canonical(
                &store.guard,
                store.run_id,
                &history,
                &crate::model::Usage::default(),
            )
        })
        .await
    }
    pub(crate) async fn start_operator_scope(
        &mut self,
        agent: &Agent,
    ) -> Result<(), crate::agent::AgentError> {
        self.start_operator().await?;
        let session = self
            .storage(|store| Ok(store.journal.load_session(store.session_id)?.session))
            .await?;
        if let Some(scope) = agent.prepare_run_with_id(&session, self.run_id).await? {
            let reference = scope.reference();
            self.storage(move |store| {
                store
                    .journal
                    .register_run_scope(&store.guard, store.run_id, reference)
            })
            .await?;
        }
        Ok(())
    }
    pub(crate) async fn finish_operator(
        &self,
        result: Result<String, String>,
        cancelled: bool,
    ) -> Result<RunRecord, CheckpointError> {
        self.storage(move |store| {
            let (state, reason, text) = if cancelled {
                (RunState::Cancelled, Some("operator action cancelled"), None)
            } else {
                match &result {
                    Ok(text) => (RunState::Completed, None, Some(text.as_str())),
                    Err(_) => (RunState::Failed, Some("operator action failed"), None),
                }
            };
            store
                .journal
                .finish(&store.guard, store.run_id, state, reason, text)
        })
        .await
    }
    pub(crate) async fn finish_operator_scoped(
        &self,
        text: String,
        lease: crate::completion::runtime::ReadinessLease,
        cancel: CancellationToken,
    ) -> Result<RunRecord, CheckpointError> {
        let clean = lease.readiness.ready() && lease.readiness.incomplete == 0;
        self.storage(move |store| {
            let mut saved = store.journal.load_session(store.session_id)?;
            let mut message = Message::new(crate::model::Role::Assistant, text);
            message.operator_name = saved
                .session
                .messages
                .last()
                .and_then(|m| m.operator_name.clone());
            saved.session.messages.push(message);
            let run = store.journal.run(store.run_id)?;
            store.journal.checkpoint_canonical(
                &store.guard,
                store.run_id,
                &saved.session.messages,
                &run.usage,
            )
        })
        .await?;
        lease
            .seal(
                if clean {
                    crate::completion::FinalOutcome::Completed
                } else {
                    crate::completion::FinalOutcome::Incomplete
                },
                (!clean).then(|| "operator action left incomplete obligations".to_owned()),
            )
            .await
            .map_err(|_| CheckpointError)?;
        self.storage(move |store| {
            let saved = store.journal.load_session(store.session_id)?;
            let run = store.journal.run(store.run_id)?;
            store.journal.accept_checkpoint(
                &store.guard,
                store.run_id,
                &saved.session.messages,
                &run.usage,
            )?;
            let (state, reason) = if cancel.is_cancelled() {
                (RunState::Cancelled, Some("operator action cancelled"))
            } else if clean {
                (RunState::Completed, None)
            } else {
                (
                    RunState::Incomplete,
                    Some("operator action left incomplete obligations"),
                )
            };
            store
                .journal
                .finish(&store.guard, store.run_id, state, reason, None)
        })
        .await
    }
    pub(crate) async fn github_reference(
        &self,
        reference: Option<crate::github::operator::Reference>,
        forget: Option<crate::github::repository::Object>,
    ) -> Result<bool, CheckpointError> {
        self.storage(move |store| {
            store
                .journal
                .operator_reference(&store.guard, store.run_id, reference, forget)
        })
        .await
    }
}
