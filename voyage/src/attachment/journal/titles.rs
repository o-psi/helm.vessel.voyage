//! Automatic names are metadata, not canonical conversation checkpoints.
use super::*;

impl Journal {
    pub(crate) fn title_input(&self, session_id: Uuid) -> Result<Option<(Uuid, Session)>> {
        let mut session = self.load_session(session_id)?.session;
        if !session.automatic_title_enabled() {
            return Ok(None);
        }
        let Some(id) = session
            .title_state
            .as_ref()
            .and_then(|state| state.requested_by)
        else {
            return Ok(None);
        };
        if steering::receipt_exists(&self.connection, id)? {
            let record = self.steering_record(id)?;
            if record.status == crate::model::SteeringStatus::NotApplied {
                return Ok(None);
            }
            // Queued input is intent already, even while the main model or tool is busy.
            // Never write this projection into canonical history or duplicate applied input.
            if !session.messages.iter().any(|message| {
                message
                    .steering
                    .as_ref()
                    .is_some_and(|receipt| receipt.id == id)
            }) {
                session.messages.push(record.queued_message());
            }
        }
        Ok(Some((id, session)))
    }

    pub(crate) fn apply_title(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        input_id: Uuid,
        result: crate::titles::TitleResult,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let run = read_run(&tx, run_id)?;
        ensure!(run.session_id == guard.session_id, "title run mismatch");
        let mut current = read_session(&tx, run.session_id)?;
        // A concurrent manual rename or newer input wins. Usage still belongs to
        // the auxiliary request; failed writes never replay provider inference.
        current.session.apply_generated_title(input_id, result);
        update_session(&tx, &current)?;
        commit(tx, &self.commit_fence)
    }
}
