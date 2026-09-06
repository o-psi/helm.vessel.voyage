//! Explicit human operator actions use the same exclusive durable run owner.
use super::*;
impl RunOwner {
    pub(crate) async fn start_operator(&mut self) -> Result<(), CheckpointError> {
        let (mut history, prompt) = self.input.take().ok_or(CheckpointError)?;
        self.steering_receiver.take();
        let authority = self.token.execution_authority.clone();
        self.storage(move |store| {
            if let Some(authority) = authority {
                authority.check()?;
            }
            store.journal.mark_running(&store.guard, store.run_id)?;
            history.push(Message::new(crate::model::Role::User, prompt));
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
    ) -> Result<RunRecord, CheckpointError> {
        let clean = lease.readiness.ready() && lease.readiness.incomplete == 0;
        self.storage(move |store| {
            let mut saved = store.journal.load_session(store.session_id)?;
            saved
                .session
                .messages
                .push(Message::new(crate::model::Role::Assistant, text));
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
            store.journal.finish(
                &store.guard,
                store.run_id,
                if clean {
                    RunState::Completed
                } else {
                    RunState::Incomplete
                },
                (!clean).then_some("operator action left incomplete obligations"),
                None,
            )
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
