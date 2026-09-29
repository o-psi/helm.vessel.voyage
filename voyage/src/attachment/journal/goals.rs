//! Canonical Goal state and receipts. Callers authenticate human authority before
//! mutations; objective text is never interpreted as authority by this store.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::{
    goals::*,
    process::{GrantBinding, RuntimeCommand},
};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS process_goals(session_id TEXT PRIMARY KEY REFERENCES sessions(id),revision INTEGER NOT NULL CHECK(revision>=0),state TEXT,authority TEXT);
CREATE TABLE IF NOT EXISTS process_goal_turns(command_id TEXT PRIMARY KEY,session_id TEXT NOT NULL REFERENCES sessions(id),goal_id TEXT NOT NULL,incarnation TEXT NOT NULL,started_at_ms INTEGER NOT NULL,state TEXT NOT NULL CHECK(state IN ('reserved','settled','abandoned')),request TEXT NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS one_reserved_goal_turn ON process_goal_turns(session_id) WHERE state='reserved';
CREATE TABLE IF NOT EXISTS process_goal_settlements(command_id TEXT PRIMARY KEY REFERENCES process_goal_turns(command_id),receipt TEXT NOT NULL,progress_digest TEXT);
CREATE TABLE IF NOT EXISTS process_goal_meters(command_id TEXT PRIMARY KEY REFERENCES commands(id),incarnation TEXT NOT NULL,budget TEXT,started_at_ms INTEGER NOT NULL,settlement TEXT);
CREATE TABLE IF NOT EXISTS process_goal_requests(request_id TEXT PRIMARY KEY,command_id TEXT NOT NULL REFERENCES process_goal_meters(command_id),observation TEXT NOT NULL);";

mod continuation;
pub(crate) use continuation::GoalTurnReservation;
mod allocations;
mod dispatch;
mod reconciliation;
pub(crate) use reconciliation::GoalAllocation;
mod delegated;
mod metering;
mod settlement;
pub(crate) use settlement::GoalMeasurement;

pub(super) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(SCHEMA)?;
    let columns: u64 = db.query_row(
        "SELECT count(*) FROM pragma_table_info('process_goal_meters')",
        [],
        |r| r.get(0),
    )?;
    if columns == 2 {
        // Schema 15 only admitted meters attached to Goal reservations. Preserve
        // their exact observations while moving the FK to the admitted command.
        db.execute_batch("CREATE TABLE budget_meters_v16(command_id TEXT PRIMARY KEY REFERENCES commands(id),incarnation TEXT NOT NULL,budget TEXT,started_at_ms INTEGER NOT NULL,settlement TEXT);
            INSERT INTO budget_meters_v16 SELECT m.command_id,m.incarnation,NULL,t.started_at_ms,NULL FROM process_goal_meters m JOIN process_goal_turns t USING(command_id);
            CREATE TABLE budget_requests_v16(request_id TEXT PRIMARY KEY,command_id TEXT NOT NULL REFERENCES budget_meters_v16(command_id),observation TEXT NOT NULL);
            INSERT INTO budget_requests_v16 SELECT * FROM process_goal_requests;
            DROP TABLE process_goal_requests;
            DROP TABLE process_goal_meters;
            ALTER TABLE budget_meters_v16 RENAME TO process_goal_meters;
            ALTER TABLE budget_requests_v16 RENAME TO process_goal_requests;")?;
    }
    ensure!(
        db.query_row(
            "SELECT count(*) FROM pragma_table_info('process_goal_meters')",
            [],
            |r| r.get::<_, u64>(0)
        )? == 5,
        "unsupported Goal meter schema"
    );
    db.execute_batch("CREATE TABLE IF NOT EXISTS process_goal_allocations(request_id TEXT PRIMARY KEY REFERENCES process_goal_requests(request_id),destination TEXT NOT NULL,budget TEXT NOT NULL,receipt TEXT,cleanup_observed INTEGER NOT NULL DEFAULT 0);")?;
    let gated:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('process_goal_allocations') WHERE name='dispatch_gated')",[],|r|r.get(0))?;
    if !gated {
        db.execute_batch("ALTER TABLE process_goal_allocations ADD COLUMN dispatch_gated INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE process_goal_allocations ADD COLUMN dispatch TEXT;
        ALTER TABLE process_goal_allocations ADD COLUMN closure TEXT;")?;
    }
    db.execute_batch("CREATE TABLE IF NOT EXISTS process_goal_reconciliations(command_id TEXT PRIMARY KEY REFERENCES process_goal_meters(command_id),input_tokens INTEGER NOT NULL,output_tokens INTEGER NOT NULL,local_cleanup_observed INTEGER NOT NULL DEFAULT 0);")?;
    Ok(())
}

/// Host-private continuation binding. It is never copied into snapshots/events.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GoalAuthority {
    pub installation_id: Uuid,
    pub principal_id: Uuid,
    pub grant: Option<GrantBinding>,
}

impl GoalAuthority {
    fn valid(&self) -> bool {
        !self.installation_id.is_nil()
            && !self.principal_id.is_nil()
            && self
                .grant
                .as_ref()
                .is_none_or(|g| !g.grant_id.is_nil() && g.principal_id == self.principal_id)
    }
}

pub(super) fn read(db: &Connection, session: Uuid) -> Result<GoalSnapshot> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_goals')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(GoalSnapshot::default());
    }
    let row: Option<(u64, Option<String>)> = db
        .query_row(
            "SELECT revision,state FROM process_goals WHERE session_id=?1",
            [session.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        None => Ok(GoalSnapshot::default()),
        Some((revision, state)) => {
            let goal: Option<Goal> = state.map(|s| serde_json::from_str(&s)).transpose()?;
            ensure!(
                goal.as_ref().is_none_or(|g| g.session_id == session),
                "goal session mismatch"
            );
            Ok(GoalSnapshot { revision, goal })
        }
    }
}

fn persist(
    tx: &Transaction<'_>,
    session: Uuid,
    snapshot: &GoalSnapshot,
    authority: Option<&GoalAuthority>,
) -> Result<()> {
    tx.execute("INSERT INTO process_goals(session_id,revision,state,authority) VALUES(?1,?2,?3,?4) ON CONFLICT(session_id) DO UPDATE SET revision=excluded.revision,state=excluded.state,authority=excluded.authority",
        params![session.to_string(), i64::try_from(snapshot.revision)?, snapshot.goal.as_ref().map(serde_json::to_string).transpose()?, authority.map(serde_json::to_string).transpose()?])?;
    // The public-v1 event is an invalidation; v2 carries only non-sensitive Goal
    // metadata. Objective text requires a History-authorized snapshot/read.
    let observations: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_observations')",
        [],
        |r| r.get(0),
    )?;
    if observations {
        let payload = json!({"goal_revision":snapshot.revision,"goal_id":snapshot.goal.as_ref().map(|g|g.id),"status":snapshot.goal.as_ref().map(|g|g.status),"usage":snapshot.goal.as_ref().map(|g|&g.usage),"stop_reason":snapshot.goal.as_ref().and_then(|g|g.stop_reason)});
        tx.execute("INSERT INTO process_observations(session_id,kind,revision,entity_id,payload) VALUES(?1,'goal',(SELECT revision FROM sessions WHERE id=?1),?2,?3)", params![session.to_string(), snapshot.goal.as_ref().map(|g|g.id.to_string()), serde_json::to_string(&payload)?])?;
    }
    Ok(())
}

impl Journal {
    pub(crate) fn goal(&self, session: Uuid) -> Result<GoalSnapshot> {
        self.check_schema()?;
        self.load_session(session)?;
        read(&self.connection, session)
    }

    pub(crate) fn update_goal(
        &mut self,
        guard: &ExecutionGuard,
        authority: GoalAuthority,
        command: &RuntimeCommand,
        now: i64,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(authority.valid() && now >= 0, "invalid goal actor or time");
        let RuntimeCommand::GoalUpdate {
            command_id,
            expected_revision,
            expires_at_ms,
            action,
        } = command
        else {
            anyhow::bail!("not a goal mutation");
        };
        ensure!(!command_id.is_nil(), "nil goal command ID");
        // An old process must not continue writing after it can no longer enforce
        // the persisted Goal authority and continuation rules.
        self.require_content_schema(guard, false)?;
        self.bind_process_command(guard, *command_id, authority.principal_id, command)?;
        let request = serde_json::to_string(command)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT request,receipt FROM process_commands WHERE id=?1",
                [command_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((original, receipt)) = prior {
            ensure!(
                super::deletion::request_matches(&original, &request),
                "goal command payload conflict"
            );
            return Ok(serde_json::from_str(&receipt)?);
        }
        let collisions: u64 = tx.query_row("SELECT (SELECT count(*) FROM commands WHERE id=?1)+(SELECT count(*) FROM steering WHERE id=?1)", [command_id.to_string()], |r|r.get(0))?;
        ensure!(collisions == 0, "goal command ID already used");
        let expiry = i64::try_from(*expires_at_ms)?;
        ensure!(
            expiry > now && expiry - now <= 300_000,
            "invalid goal command deadline"
        );
        super::lifecycle::ensure_admissible(&tx, guard.session_id)?;
        let saved = read_session(&tx, guard.session_id)?;
        ensure!(
            saved.revision == *expected_revision,
            "session revision conflict"
        );
        let mut current = read(&tx, guard.session_id)?;
        let continuation: Option<GoalAuthority>;
        let now = u64::try_from(now)?;
        match action {
            GoalAction::Set {
                objective,
                limits,
                replace_goal_id,
                continue_automatically,
            } => {
                ensure!(
                    valid_objective(objective) && limits.valid(),
                    "invalid goal objective or limits"
                );
                match &current.goal {
                    Some(old) => ensure!(
                        *replace_goal_id == Some(old.id),
                        "confirm replacement of the current goal"
                    ),
                    None => ensure!(
                        replace_goal_id.is_none(),
                        "replacement goal no longer exists"
                    ),
                }
                ensure_idle(&tx, guard.session_id)?;
                current.goal = Some(Goal {
                    id: *command_id,
                    session_id: guard.session_id,
                    objective: objective.clone(),
                    status: if *continue_automatically {
                        GoalStatus::Active
                    } else {
                        GoalStatus::Paused
                    },
                    continuation_authorized: *continue_automatically,
                    limits: limits.clone(),
                    usage: GoalUsage::default(),
                    created_at_ms: now,
                    updated_at_ms: now,
                    stop_reason: (!continue_automatically).then_some(GoalStopReason::UserPaused),
                });
                continuation = continue_automatically.then_some(authority);
            }
            GoalAction::Edit {
                goal_id,
                objective,
                limits,
            } => {
                ensure!(
                    valid_objective(objective) && limits.valid(),
                    "invalid goal objective or limits"
                );
                ensure_idle(&tx, guard.session_id)?;
                let goal = selected(&mut current, *goal_id)?;
                ensure!(
                    goal.status != GoalStatus::Complete,
                    "completed goal requires explicit replacement"
                );
                goal.objective = objective.clone();
                goal.limits = limits.clone();
                // Continuation consent describes the accepted objective/limits.
                // An edit retains usage but needs an explicit fresh resume.
                goal.status = GoalStatus::Paused;
                goal.stop_reason = Some(GoalStopReason::UserPaused);
                goal.continuation_authorized = false;
                continuation = None;
                if let Some(reason) = goal.limit_reached() {
                    goal.status = GoalStatus::Limited;
                    goal.stop_reason = Some(reason);
                    goal.continuation_authorized = false;
                }
                goal.updated_at_ms = now;
            }
            GoalAction::Pause { goal_id } => {
                let goal = selected(&mut current, *goal_id)?;
                ensure!(goal.status != GoalStatus::Complete, "goal already complete");
                goal.status = GoalStatus::Paused;
                goal.stop_reason = Some(GoalStopReason::UserPaused);
                goal.continuation_authorized = false;
                goal.updated_at_ms = now;
                continuation = None;
            }
            GoalAction::Resume { goal_id } => {
                ensure_idle(&tx, guard.session_id)?;
                let goal = selected(&mut current, *goal_id)?;
                ensure!(goal.status != GoalStatus::Complete, "goal already complete");
                ensure!(
                    goal.limit_reached().is_none(),
                    "goal limit reached; edit limits before resuming"
                );
                ensure!(
                    goal.usage.unmeasured_runs == 0,
                    "goal usage is incomplete; explicitly replace the goal with a new budget"
                );
                goal.status = GoalStatus::Active;
                goal.stop_reason = None;
                goal.continuation_authorized = true;
                goal.updated_at_ms = now;
                continuation = Some(authority);
            }
            GoalAction::Clear { goal_id } => {
                ensure_idle(&tx, guard.session_id)?;
                selected(&mut current, *goal_id)?;
                current.goal = None;
                continuation = None;
            }
        }
        current.revision = current
            .revision
            .checked_add(1)
            .context("goal revision overflow")?;
        update_session(&tx, &saved)?;
        persist(&tx, guard.session_id, &current, continuation.as_ref())?;
        let receipt = json!({"command_id":command_id,"status":"applied","revision":saved.revision+1,"goal_revision":current.revision,"goal_id":current.goal.as_ref().map(|g|g.id)});
        let count: i64 = tx.query_row("SELECT count(*) FROM process_commands", [], |r| r.get(0))?;
        ensure!(
            count < MAX_COMMANDS,
            "process command receipt capacity reached"
        );
        tx.execute(
            "INSERT INTO process_commands VALUES(?1,?2,?3)",
            params![
                command_id.to_string(),
                request,
                serde_json::to_string(&receipt)?
            ],
        )?;
        Journal::append_public_command(&tx, guard.session_id, *command_id)?;
        commit(tx, &self.commit_fence)?;
        Ok(receipt)
    }
}

fn selected(snapshot: &mut GoalSnapshot, id: Uuid) -> Result<&mut Goal> {
    let goal = snapshot.goal.as_mut().context("no goal exists")?;
    ensure!(!id.is_nil() && goal.id == id, "goal identity conflict");
    Ok(goal)
}

fn ensure_idle(db: &Connection, session: Uuid) -> Result<()> {
    let active: u64 = db.query_row(
        "SELECT count(*) FROM runs WHERE session_id=?1 AND active=1",
        [session.to_string()],
        |r| r.get(0),
    )?;
    ensure!(active == 0, "goal change requires idle voyage");
    let reserved: u64 = db.query_row(
        "SELECT count(*) FROM process_goal_turns WHERE session_id=?1 AND state='reserved'",
        [session.to_string()],
        |r| r.get(0),
    )?;
    ensure!(
        reserved == 0,
        "goal continuation admission remains unresolved"
    );
    let cleanup: u64 = db.query_row("SELECT count(*) FROM local_cleanup_obligations WHERE session_id=?1 AND confirmation IS NULL",[session.to_string()],|r|r.get(0))?;
    ensure!(cleanup == 0, "goal continuation requires observed cleanup");
    ensure!(
        cleanup_ready(db, session)?,
        "goal continuation has unresolved retained effects"
    );
    Ok(())
}

fn cleanup_ready(db: &Connection, session: Uuid) -> Result<bool> {
    for (table, query) in [
        (
            "process_goal_allocations",
            "SELECT count(*) FROM process_goal_allocations a JOIN process_goal_requests q USING(request_id) JOIN commands c ON c.id=q.command_id JOIN runs r ON r.id=c.run_id WHERE r.session_id=?1 AND a.cleanup_observed=0",
        ),
        (
            "local_cleanup_obligations",
            "SELECT count(*) FROM local_cleanup_obligations WHERE session_id=?1 AND (confirmation IS NULL OR confirmation!='observed')",
        ),
        (
            "process_session_resources",
            "SELECT count(*) FROM process_session_resources WHERE session_id=?1 AND state IN ('cleanup_unknown','retained_unknown','operator_attested')",
        ),
        (
            "process_retained_cleanup",
            "SELECT count(*) FROM process_retained_cleanup WHERE session_id=?1 AND (confirmation IS NULL OR confirmation!='observed')",
        ),
    ] {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
            [table],
            |r| r.get(0),
        )?;
        if exists && db.query_row(query, [session.to_string()], |r| r.get::<_, u64>(0))? > 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
