//! Explicit local recovery evidence. Appending unknown outcomes never dispatches
//! effects or changes a run's terminal classification. Remote exposure needs its
//! own authority and session-change projection; these IDs are only attribution.
use super::*;
use std::collections::BTreeSet;

pub(super) const SCHEMA: &str = "CREATE TABLE local_tool_reconciliations(run_id TEXT PRIMARY KEY REFERENCES runs(id), session_id TEXT NOT NULL REFERENCES sessions(id), record TEXT NOT NULL);";
const MAX_CALLS: usize = 128;
const MAX_CALL_ID: usize = 1024;
const MAX_RECEIPT: usize = 256 * 1024;
const UNKNOWN_RESULT: &str = "Tool execution was interrupted; its outcome is unknown. This tool was not retried. This record does not establish success or rollback of any effects.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalReconcileRequest {
    pub session_id: Uuid,
    pub run_id: Uuid,
    pub installation_id: Uuid,
    pub principal_id: Uuid,
    pub expected_revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LocalReconcileOutcome {
    pub duplicate: bool,
    pub revision: u64,
    pub tool_call_ids: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    request: LocalReconcileRequest,
    revision: u64,
    tool_call_ids: Vec<String>,
}
fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= MAX_CALL_ID && !id.chars().any(char::is_control)
}
// Refuse ambiguous history instead of fabricating retroactive success or inserting
// messages into its immutable prefix. Resolved legacy orphan results are tolerated
// only before a new unresolved block, as with admission's historical compaction.
fn unresolved(messages: &[Message]) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut resolved = BTreeSet::new();
    let mut pending = Vec::new();
    for message in messages {
        ensure!(
            (message.role == Role::Assistant || message.tool_calls.is_empty())
                && (message.role == Role::Tool || message.tool_call_id.is_none()),
            "invalid tool message shape"
        );
        match message.role {
            Role::Assistant => {
                ensure!(
                    pending.is_empty(),
                    "cannot append outcomes after later assistant history"
                );
                // Provider call IDs belong to one response, not the entire
                // conversation. Completed historical batches cannot make a
                // fresh interrupted call with the same ID unrecoverable.
                seen.clear();
                resolved.clear();
                for call in &message.tool_calls {
                    ensure!(
                        valid_id(&call.id)
                            && !resolved.contains(call.id.as_str())
                            && seen.insert(call.id.as_str()),
                        "invalid or ambiguous tool call identity"
                    );
                    pending.push(call.id.clone());
                    ensure!(pending.len() <= MAX_CALLS, "too many unresolved tool calls");
                }
            }
            Role::Tool => {
                let id = message
                    .tool_call_id
                    .as_deref()
                    .context("tool result lacks identity")?;
                ensure!(
                    valid_id(id) && resolved.insert(id),
                    "invalid or duplicate tool result identity"
                );
                if let Some(index) = pending.iter().position(|pending| pending == id) {
                    pending.remove(index);
                } else {
                    ensure!(
                        pending.is_empty() && !seen.contains(id),
                        "tool result does not match unresolved call"
                    );
                }
            }
            _ => ensure!(
                pending.is_empty(),
                "cannot append outcomes after later conversation history"
            ),
        }
    }
    ensure!(!pending.is_empty(), "no unresolved tool calls to reconcile");
    Ok(pending)
}

impl Journal {
    /// Explicit operator-selected recovery. Requires the exact local actor, guarded
    /// terminal latest run, resolved cleanup, and original expected revision. New
    /// error results report unknown effects, never success/rollback or replay.
    /// Exact immutable retries are observations even after a later run/revision.
    pub fn reconcile_local_tools(
        &mut self,
        guard: &ExecutionGuard,
        request: &LocalReconcileRequest,
    ) -> Result<LocalReconcileOutcome> {
        self.reconcile_tools_authorized(guard, request, None)
    }
    /// Local-only recovery of a dedicated remote run. The receipt records the
    /// verified installation actor, not an impersonated remote principal.
    pub fn reconcile_remote_tools(
        &mut self,
        guard: &ExecutionGuard,
        request: &LocalReconcileRequest,
        binding: &super::remote::RemoteBinding,
    ) -> Result<LocalReconcileOutcome> {
        self.reconcile_tools_authorized(guard, request, Some(binding))
    }
    fn reconcile_tools_authorized(
        &mut self,
        guard: &ExecutionGuard,
        request: &LocalReconcileRequest,
        binding: Option<&super::remote::RemoteBinding>,
    ) -> Result<LocalReconcileOutcome> {
        self.check_guard(guard, request.session_id)?;
        ensure!(
            self.opened_schema >= 6,
            "explicit quiescent journal upgrade required"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let run = read_run(&tx, request.run_id)?;
        if let Some(binding) = binding {
            super::remote::require(&tx, binding, request.session_id)?;
            ensure!(
                run.session_id == request.session_id
                    && run.machine_id == binding.machine_id
                    && run.principal_id == binding.owner_id
                    && request.installation_id == binding.local_installation_id
                    && request.principal_id == binding.local_principal_id,
                "remote recovery attribution mismatch"
            );
        } else {
            ensure!(
                run.session_id == request.session_id
                    && run.machine_id == request.installation_id
                    && run.principal_id == request.principal_id,
                "reconciliation actor or run mismatch"
            );
        }
        let existing: Option<(String, String)> = tx.query_row(
            "SELECT session_id,record FROM local_tool_reconciliations WHERE run_id=?1 AND length(CAST(record AS BLOB))<=?2",
            params![request.run_id.to_string(), MAX_RECEIPT], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        if let Some((session_id, record)) = existing {
            let receipt: Receipt = serde_json::from_str(&record)?;
            ensure!(
                receipt.request == *request && session_id == request.session_id.to_string(),
                "reconciliation identity reused with different request"
            );
            ensure!(
                request.expected_revision.checked_add(1) == Some(receipt.revision)
                    && receipt.revision <= i64::MAX as u64
                    && !receipt.tool_call_ids.is_empty()
                    && receipt.tool_call_ids.len() <= MAX_CALLS
                    && receipt.tool_call_ids.iter().all(|id| valid_id(id))
                    && receipt.tool_call_ids.iter().collect::<BTreeSet<_>>().len()
                        == receipt.tool_call_ids.len(),
                "invalid reconciliation receipt"
            );
            return Ok(LocalReconcileOutcome {
                duplicate: true,
                revision: receipt.revision,
                tool_call_ids: receipt.tool_call_ids,
            });
        }
        // Oversized stored evidence must fail closed rather than masquerading as
        // absent and reaching a unique constraint after an attempted rewrite.
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_tool_reconciliations WHERE run_id=?1)",
            [request.run_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(!exists, "reconciliation receipt exceeds limit");
        ensure!(
            !matches!(run.state, RunState::Accepted | RunState::Running),
            "reconciliation requires a terminal run"
        );
        let latest: String = tx.query_row("SELECT json_extract(event,'$.run_id') FROM events WHERE session_id=?1 ORDER BY sequence DESC LIMIT 1", [request.session_id.to_string()], |r| r.get(0))?;
        ensure!(
            latest == request.run_id.to_string(),
            "reconciliation requires the latest run"
        );
        let confirmation: Option<String> = tx.query_row("SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?1 AND session_id=?2 AND installation_id=?3 AND principal_id=?4", params![request.run_id.to_string(),request.session_id.to_string(),run.machine_id.to_string(),run.principal_id.to_string()], |r| r.get(0))?;
        ensure!(
            confirmation
                .as_deref()
                .is_some_and(|value| matches!(value, "observed" | "operator_attested")),
            "reconciliation requires completed cleanup"
        );
        ensure!(
            catalogue::pending_cleanup(&tx, request.session_id)?.is_none(),
            "session cleanup remains unconfirmed"
        );
        let mut current = read_session(&tx, request.session_id)?;
        ensure!(
            current.revision == request.expected_revision,
            "stale reconciliation revision"
        );
        let ids = unresolved(&current.session.messages)?;
        for id in &ids {
            current
                .session
                .messages
                .push(Message::tool_result(id, UNKNOWN_RESULT, false));
        }
        let revision = current
            .revision
            .checked_add(1)
            .context("revision overflow")?;
        let receipt = Receipt {
            request: request.clone(),
            revision,
            tool_call_ids: ids.clone(),
        };
        let record = serde_json::to_string(&receipt)?;
        ensure!(
            record.len() <= MAX_RECEIPT,
            "reconciliation receipt exceeds limit"
        );
        update_session(&tx, &current)?;
        tx.execute(
            "INSERT INTO local_tool_reconciliations VALUES(?1,?2,?3)",
            params![
                request.run_id.to_string(),
                request.session_id.to_string(),
                record
            ],
        )?;
        // Local snapshot revision changes; the terminal run/event stream remains
        // immutable. Remote operators explicitly inspect the current snapshot revision;
        // reconciliation never emits a successful tool result or replays an effect.
        commit(tx, &self.commit_fence)?;
        Ok(LocalReconcileOutcome {
            duplicate: false,
            revision,
            tool_call_ids: ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call() -> Message {
        let mut message = Message::new(Role::Assistant, "");
        message.tool_calls.push(crate::model::ToolCall {
            id: "reused".into(),
            name: "shell".into(),
            arguments: serde_json::json!({"command":"work"}),
        });
        message
    }

    #[test]
    fn reconcile_reused_identity_only_in_latest_unresolved_response() {
        let history = vec![
            call(),
            Message::tool_result("reused", "finished", true),
            Message::new(Role::User, "next run"),
            call(),
        ];
        assert_eq!(unresolved(&history).unwrap(), ["reused"]);
    }

    #[test]
    fn ambiguous_batches_and_history_after_unresolved_work_still_fail() {
        let mut duplicate = call();
        duplicate.tool_calls.push(duplicate.tool_calls[0].clone());
        assert!(unresolved(&[duplicate]).is_err());
        assert!(unresolved(&[call(), call()]).is_err());
        assert!(unresolved(&[call(), Message::new(Role::User, "later input")]).is_err());
        assert!(unresolved(&[call(), Message::tool_result("other", "wrong", true)]).is_err());
    }
}
