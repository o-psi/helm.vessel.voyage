//! Working projections are metadata over canonical history, never replacement history.
use super::*;
use crate::context::WorkingContext;

impl Journal {
    pub(crate) fn load_working_context(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
    ) -> Result<WorkingContext> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id,
            "working context run mismatch"
        );
        ensure!(
            run.state == RunState::Running,
            "working context requires a running run"
        );
        let current = read_session(&tx, run.session_id)?;
        current
            .session
            .working_context
            .validate(&current.session.messages)?;
        let context = current.session.working_context;
        commit(tx, &self.commit_fence)?;
        Ok(context)
    }

    pub(crate) fn save_context_observation(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        observation: &voyage_protocol::context_accounting::ContextObservation,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            observation.execution_id == run_id,
            "context accounting run mismatch"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id && run.state == RunState::Running,
            "context accounting requires this running owner"
        );
        let mut current = read_session(&tx, run.session_id)?;
        ensure!(
            observation.projection_generation == current.session.working_context.generation,
            "context accounting generation mismatch"
        );
        current.session.context_observation = Some(observation.clone());
        update_session(&tx, &current)?;
        append_event(&tx, &run, EventKind::CanonicalCheckpoint)?;
        commit(tx, &self.commit_fence)
    }

    pub(crate) fn save_working_context(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        context: &WorkingContext,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id,
            "working context run mismatch"
        );
        ensure!(
            run.state == RunState::Running,
            "working context requires a running run"
        );
        let mut current = read_session(&tx, run.session_id)?;
        // Validate against durable, unhydrated canonical messages. The projection
        // must not duplicate source text, provider replay state or artifact bytes.
        context.validate(&current.session.messages)?;
        current.session.working_context = context.clone();
        update_session(&tx, &current)?;
        append_event(&tx, &run, EventKind::CanonicalCheckpoint)?;
        commit(tx, &self.commit_fence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_accounting_is_owner_fenced_durable_and_does_not_replace_history() {
        use voyage_protocol::context_accounting::*;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("journal");
        let mut session = Session::new(root.path().into(), "fixture".into());
        session
            .messages
            .push(Message::new(Role::User, "Preserve the exact constraint"));
        let mut journal = Journal::open(directory.clone()).unwrap();
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        let run = journal
            .admit_turn(
                &guard,
                &TurnAdmission {
                    budget: None,
                    coordination: None,
                    operator_name: None,
                    command_id: Uuid::new_v4(),
                    machine_id: Uuid::new_v4(),
                    principal_id: Uuid::new_v4(),
                    session_id: session.id,
                    expected_revision: 0,
                    expires_at_ms: 10000,
                    prompt: "count".into(),
                    parts: vec![],
                },
                1,
            )
            .unwrap()
            .run;
        journal.mark_running(&guard, run.id).unwrap();
        let original =
            serde_json::to_value(&journal.load_session(session.id).unwrap().session.messages)
                .unwrap();
        let mut observation = ContextObservation {
            execution_id: run.id,
            projection_generation: 0,
            count: RequestTokenCount {
                scope: ContextScope {
                    model: "fixture".into(),
                    transport: "fixture".into(),
                    endpoint_fingerprint: None,
                    account: None,
                },
                observed_at_ms: 1,
                input_tokens: None,
                precision: CountPrecision::Unknown,
                method: "unavailable".into(),
                complete: false,
                input_fingerprint: None,
                limitations: vec![],
            },
            capacity: None,
            operator_limit: None,
            output_reserve: None,
            reserve_source: "unknown".into(),
            safety_margin: 0,
            pressure: ContextPressure::Unknown,
            scope: "prepared_provider_input_before_dispatch".into(),
        };
        journal
            .save_context_observation(&guard, run.id, &observation)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&journal.load_session(session.id).unwrap().session.messages)
                .unwrap(),
            original
        );
        observation.projection_generation = 1;
        assert!(
            journal
                .save_context_observation(&guard, run.id, &observation)
                .is_err()
        );
        observation.projection_generation = 0;
        observation.execution_id = Uuid::new_v4();
        assert!(
            journal
                .save_context_observation(&guard, run.id, &observation)
                .is_err()
        );
        drop(guard);
        drop(journal);
        let reopened = Journal::open(directory.clone()).unwrap();
        let saved = reopened.load_session(session.id).unwrap();
        assert_eq!(
            saved.session.context_observation.unwrap().execution_id,
            run.id
        );
        assert_eq!(
            serde_json::to_value(saved.session.messages).unwrap(),
            original
        );
    }

    #[test]
    fn projection_commit_is_fenced_revisioned_and_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("journal");
        let mut session = Session::new(root.path().into(), "fixture".into());
        for index in 0..12 {
            session
                .messages
                .push(Message::new(Role::User, format!("question {index}")));
            session.messages.push(Message::new(
                Role::Assistant,
                format!("answer {index}: {}", "source details ".repeat(1000)),
            ));
        }
        let mut journal = Journal::open(directory.clone()).unwrap();
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        let run = journal
            .admit_turn(
                &guard,
                &TurnAdmission {
                    budget: None,
                    coordination: None,
                    operator_name: None,
                    command_id: Uuid::new_v4(),
                    machine_id: Uuid::new_v4(),
                    principal_id: Uuid::new_v4(),
                    session_id: session.id,
                    expected_revision: 0,
                    expires_at_ms: 10000,
                    prompt: "next question".into(),
                    parts: vec![],
                },
                1,
            )
            .unwrap()
            .run;
        // Accepted-but-not-running is not execution authority for these hooks.
        assert!(journal.load_working_context(&guard, run.id).is_err());
        assert!(
            journal
                .save_working_context(&guard, run.id, &WorkingContext::default())
                .is_err()
        );
        journal.mark_running(&guard, run.id).unwrap();
        let before = journal.load_session(session.id).unwrap();
        let canonical = serde_json::to_value(&before.session.messages).unwrap();
        let mut context = journal.load_working_context(&guard, run.id).unwrap();
        assert!(context.compact(&before.session.messages, 4).unwrap() > 0);
        journal
            .save_working_context(&guard, run.id, &context)
            .unwrap();
        let saved = journal.load_session(session.id).unwrap();
        assert_eq!(saved.revision, before.revision + 1);
        assert_eq!(
            serde_json::to_value(&saved.session.messages).unwrap(),
            canonical
        );
        assert_eq!(
            serde_json::to_value(&saved.session.run_summaries).unwrap(),
            serde_json::to_value(&before.session.run_summaries).unwrap()
        );
        assert!(
            journal
                .save_working_context(&guard, Uuid::new_v4(), &context)
                .is_err()
        );
        assert_eq!(
            journal.load_session(session.id).unwrap().revision,
            saved.revision
        );
        drop(guard);
        drop(journal);
        let journal = Journal::open(directory).unwrap();
        let loaded = journal.load_session(session.id).unwrap();
        assert_eq!(loaded.revision, saved.revision);
        assert_eq!(
            serde_json::to_value(&loaded.session.messages).unwrap(),
            canonical
        );
        assert_eq!(
            serde_json::to_value(&loaded.session.working_context).unwrap(),
            serde_json::to_value(&context).unwrap()
        );
        assert!(
            serde_json::to_vec(
                &loaded
                    .session
                    .working_context
                    .project(&loaded.session.messages)
                    .unwrap()
            )
            .unwrap()
            .len()
                < serde_json::to_vec(&loaded.session.messages).unwrap().len()
        );
    }
}
