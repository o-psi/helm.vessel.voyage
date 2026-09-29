//! Positive non-admission evidence is separate from a fabricated zero-use run.
use super::*;
use crate::provider::goal_meter::{AllocationDispatch, RequestObservation};

type GoalDispatchRow = (String, Option<String>, Option<String>, Option<String>, bool);
type GoalClosureRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
    String,
);

impl Journal {
    pub(crate) fn dispatch_goal_child(
        &mut self,
        guard: &ExecutionGuard,
        command: Uuid,
        incarnation: Uuid,
        dispatch: AllocationDispatch,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(self.opened_schema >= 19, "Goal dispatch gate unavailable");
        let budget = dispatch.budget().context("missing dispatch budget")?;
        let encoded = serde_json::to_string(&dispatch)?;
        ensure!(
            encoded.len() <= 256 * 1024 && dispatch.valid_for(budget),
            "invalid or oversized Goal dispatch"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (run, _, _) = super::allocations::current(&tx, guard.session_id, command, incarnation)?;
        ensure!(
            matches!(
                read_run(&tx, run)?.state,
                RunState::Accepted | RunState::Running
            ),
            "Goal dispatch parent is terminal"
        );
        let row:Option<GoalDispatchRow>=tx.query_row("SELECT a.budget,a.dispatch,a.closure,a.receipt,a.dispatch_gated FROM process_goal_allocations a JOIN process_goal_requests q USING(request_id) WHERE a.request_id=?1 AND q.command_id=?2",params![budget.command_id.to_string(),command.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let (saved, prior, closure, receipt, gated) =
            row.context("unknown Goal dispatch allocation")?;
        ensure!(
            gated
                && closure.is_none()
                && receipt.is_none()
                && serde_json::from_str::<voyage_protocol::execution_budget::ExecutionBudget>(
                    &saved
                )? == *budget
                && budget.parent_run_id == run,
            "Goal dispatch is closed or mismatched"
        );
        if let Some(prior) = prior {
            ensure!(prior == encoded, "Goal dispatch payload changed");
            return Ok(());
        }
        tx.execute(
            "UPDATE process_goal_allocations SET dispatch=?1 WHERE request_id=?2",
            params![encoded, budget.command_id.to_string()],
        )?;
        commit(tx, &self.commit_fence)
    }

    /// None proves no dispatch only for a new gated allocation after its parent
    /// became terminal. Some accepts an exact authenticated negative receipt;
    /// callers never pass a refusal, timeout, unknown ID, or human attestation.
    pub(crate) fn close_goal_allocation(
        &mut self,
        guard: &ExecutionGuard,
        destination: Uuid,
        id: Uuid,
        negative: Option<Value>,
    ) -> Result<bool> {
        self.check_guard(guard, guard.session_id)?;
        self.require_content_schema(guard, false)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row:Option<GoalClosureRow>=tx.query_row("SELECT a.destination,a.dispatch,a.closure,a.receipt,a.dispatch_gated,q.observation FROM process_goal_allocations a JOIN process_goal_requests q USING(request_id) JOIN commands c ON c.id=q.command_id JOIN runs r ON r.id=c.run_id WHERE a.request_id=?1 AND r.session_id=?2 AND (r.active=0 OR ?3=1)",params![id.to_string(),guard.session_id.to_string(),negative.is_some()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        let (target, dispatch, prior, receipt, gated, observation) =
            row.context("no terminal Goal allocation to close")?;
        ensure!(
            target == destination.to_string() && receipt.is_none(),
            "Goal closure attribution conflict or admitted child"
        );
        let proof = if let Some(negative) = negative {
            ensure!(
                negative["status"] == "not_admitted" && negative["command_id"] == id.to_string(),
                "Goal closure needs a permanent non-admission receipt"
            );
            json!({"kind":"not_admitted","receipt":negative})
        } else {
            ensure!(
                gated && dispatch.is_none(),
                "Goal child dispatch is uncertain"
            );
            json!({"kind":"never_dispatched"})
        };
        if let Some(prior) = prior {
            ensure!(
                serde_json::from_str::<Value>(&prior)? == proof,
                "Goal closure proof changed"
            );
            return Ok(false);
        }
        let old: RequestObservation = serde_json::from_str(&observation)?;
        ensure!(
            old.input_tokens.unwrap_or(0) == 0 && old.output_tokens.unwrap_or(0) == 0,
            "non-admission conflicts with retained usage"
        );
        let observed = RequestObservation {
            request_id: id,
            revision: old
                .revision
                .checked_add(1)
                .context("usage revision overflow")?,
            input_tokens: Some(0),
            output_tokens: Some(0),
            complete: true,
        };
        tx.execute(
            "UPDATE process_goal_requests SET observation=?1 WHERE request_id=?2",
            params![serde_json::to_string(&observed)?, id.to_string()],
        )?;
        tx.execute(
            "UPDATE process_goal_allocations SET closure=?1,cleanup_observed=1 WHERE request_id=?2",
            params![serde_json::to_string(&proof)?, id.to_string()],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(true)
    }
}
