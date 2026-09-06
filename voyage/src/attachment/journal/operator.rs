//! Canonical reference edits are fenced by the active explicit operator run.
use super::*;
impl Journal {
    pub(crate) fn operator_reference(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        reference: Option<crate::github::operator::Reference>,
        forget: Option<crate::github::repository::Object>,
    ) -> Result<bool> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            reference.is_some() != forget.is_some(),
            "exactly one reference edit is required"
        );
        if let Some(reference) = &reference {
            reference.validate()?;
        }
        if let Some(object) = &forget {
            object.validate()?;
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id && run.state == RunState::Running,
            "reference edit requires exact active run"
        );
        let mut current = read_session(&tx, guard.session_id)?;
        let previous = current.session.github_references.clone();
        if let Some(reference) = reference {
            current
                .session
                .github_references
                .retain(|item| item.object != reference.object);
            current.session.github_references.push(reference);
        } else if let Some(object) = forget {
            current
                .session
                .github_references
                .retain(|item| item.object != object);
        }
        ensure!(
            current.session.github_references.len() <= 256,
            "reference capacity exceeded"
        );
        let changed = previous != current.session.github_references;
        if changed {
            update_session(&tx, &current)?;
        }
        commit(tx, &self.commit_fence)?;
        Ok(changed)
    }
}
