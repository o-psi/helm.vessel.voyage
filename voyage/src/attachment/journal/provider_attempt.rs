//! Fenced metadata checkpoints never grant authority to dispatch or retry.
use super::*;
use voyage_protocol::provider_attempt::ProviderAttempt;

impl Journal {
    pub(crate) fn provider_attempt(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        attempt: &ProviderAttempt,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id,
            "provider attempt run mismatch"
        );
        // CancelRequested is stored separately by catalogue, while state remains
        // Running. Do not consult pending cancellation or execution policy here:
        // the still-owning runtime must be able to durably record why it stopped.
        ensure!(
            run.state == RunState::Running,
            "provider attempt requires an owned running run"
        );
        let mut current = read_session(&tx, run.session_id)?;
        current.session.upsert_provider_attempt(run_id, attempt)?;
        update_session(&tx, &current)?;
        append_event(&tx, &run, EventKind::CanonicalCheckpoint)?;
        commit(tx, &self.commit_fence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voyage_protocol::provider_attempt::{AttemptPhase, RetryDecision};

    #[test]
    fn attempt_history_is_fenced_upserted_and_durable_after_cancel_request() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("journal");
        let session = Session::new(root.path().into(), "fixture".into());
        let mut journal = Journal::open(directory.clone()).unwrap();
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        let run = journal
            .admit_turn(
                &guard,
                &TurnAdmission {
                    coordination: None,
                    operator_name: None,
                    command_id: Uuid::new_v4(),
                    machine_id: Uuid::new_v4(),
                    principal_id: Uuid::new_v4(),
                    session_id: session.id,
                    expected_revision: 0,
                    expires_at_ms: 10000,
                    prompt: "hello".into(),
                    parts: vec![],
                },
                1,
            )
            .unwrap()
            .run;
        let mut attempt = ProviderAttempt {
            request_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            provider: "fixture".into(),
            model: "fixture".into(),
            attempt: 1,
            limit: 8,
            started_at_ms: 1,
            duration_ms: 0,
            phase: AttemptPhase::Dispatch,
            category: None,
            http_status: None,
            text_observed: false,
            tool_fragment_observed: false,
            retry_delay_ms: None,
            decision: RetryDecision::InFlight,
        };
        assert!(journal.provider_attempt(&guard, run.id, &attempt).is_err());
        journal.mark_running(&guard, run.id).unwrap();
        let before = journal.load_session(session.id).unwrap();
        journal.provider_attempt(&guard, run.id, &attempt).unwrap();
        let mut wrong = attempt.clone();
        wrong.request_id = Uuid::new_v4();
        assert!(journal.provider_attempt(&guard, run.id, &wrong).is_err());
        assert!(
            journal
                .provider_attempt(&guard, Uuid::new_v4(), &attempt)
                .is_err()
        );
        journal
            .request_cancel_local_with_clock(
                &LocalCancelRequest {
                    session_id: session.id,
                    run_id: run.id,
                    installation_id: run.machine_id,
                    principal_id: run.principal_id,
                    expires_at_ms: 10000,
                },
                || Ok(2),
            )
            .unwrap();
        attempt.decision = RetryDecision::Cancelled;
        attempt.duration_ms = 2;
        journal.provider_attempt(&guard, run.id, &attempt).unwrap();
        let saved = journal.load_session(session.id).unwrap();
        assert_eq!(
            saved.session.run_summaries[0].provider_attempts,
            vec![attempt.clone()]
        );
        assert_eq!(
            serde_json::to_value(&saved.session.messages).unwrap(),
            serde_json::to_value(&before.session.messages).unwrap()
        );
        assert!(saved.revision > before.revision);
        journal
            .finish(
                &guard,
                run.id,
                RunState::Cancelled,
                Some("run cancelled"),
                None,
            )
            .unwrap();
        assert!(journal.provider_attempt(&guard, run.id, &attempt).is_err());
        drop(guard);
        drop(journal);
        let journal = Journal::open(directory).unwrap();
        let loaded = journal.load_session(session.id).unwrap();
        assert_eq!(
            loaded.session.run_summaries[0].provider_attempts,
            vec![attempt]
        );
    }
}
