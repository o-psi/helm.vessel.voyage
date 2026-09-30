//! Retired-owner bookkeeping cannot silently resume a cached Goal authorization.
use super::*;
impl Journal {
    pub(crate) fn fence_offline_goal(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_goals')",
            [],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT state,authority FROM process_goals WHERE session_id=?1",
                [guard.session_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((Some(encoded), authority)) = prior {
            let mut goal: voyage_protocol::goals::Goal = serde_json::from_str(&encoded)?;
            if goal.continuation_authorized
                || goal.status == voyage_protocol::goals::GoalStatus::Active
                || authority.is_some()
            {
                goal.continuation_authorized = false;
                if goal.status == voyage_protocol::goals::GoalStatus::Active {
                    goal.status = voyage_protocol::goals::GoalStatus::NeedsAttention;
                    goal.stop_reason =
                        Some(voyage_protocol::goals::GoalStopReason::UnresolvedEffects);
                }
                tx.execute("UPDATE process_goals SET state=?1,authority=NULL,revision=revision+1 WHERE session_id=?2",params![serde_json::to_string(&goal)?,guard.session_id.to_string()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_goal_fence_preserves_objective_and_never_renews_continuation() {
        use voyage_protocol::{
            goals::{GoalAction, GoalLimits, GoalStatus},
            process::RuntimeCommand,
        };
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(root.path().join("journal")).unwrap();
        let session = Session::new(root.path().canonicalize().unwrap(), "synthetic".into());
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        journal.initialize_lifecycle(&guard).unwrap();
        journal.initialize_observations(&guard).unwrap();
        let authority = super::super::goals::GoalAuthority {
            installation_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            grant: None,
        };
        journal
            .initialize_command_bindings(&guard, authority.principal_id)
            .unwrap();
        let command = RuntimeCommand::GoalUpdate {
            command_id: Uuid::new_v4(),
            expected_revision: journal.load_session(session.id).unwrap().revision,
            expires_at_ms: 61_000,
            action: GoalAction::Set {
                objective: "retained objective; no automatic execution".into(),
                limits: GoalLimits::default(),
                replace_goal_id: None,
                continue_automatically: true,
            },
        };
        journal
            .update_goal(&guard, authority.clone(), &command, 1000)
            .unwrap();
        journal.fence_offline_goal(&guard).unwrap();
        let snapshot = journal.goal(session.id).unwrap();
        let goal = snapshot.goal.as_ref().unwrap();
        assert_eq!(goal.status, GoalStatus::NeedsAttention);
        assert!(!goal.continuation_authorized);
        assert_eq!(goal.objective, "retained objective; no automatic execution");
        journal.fence_offline_goal(&guard).unwrap();
        assert_eq!(journal.goal(session.id).unwrap(), snapshot);
    }
}
