//! Refusal becomes definite only when owner-serialized reads prove no admission exists.
use super::*;
use serde_json::{Value, json};
impl Journal {
    pub(crate) fn reject_unadmitted(
        &mut self,
        guard: &ExecutionGuard,
        id: Uuid,
        principal: Uuid,
        reason: &str,
    ) -> Result<Option<Value>> {
        self.check_guard(guard, guard.session_id)?;
        if self.process_receipt(id)?.is_some() {
            return Ok(None);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE id=?1 UNION ALL SELECT 1 FROM steering WHERE id=?1 UNION ALL SELECT 1 FROM process_commands WHERE id=?1)",[id.to_string()],|r|r.get(0))?;
        if existing {
            return Ok(None);
        }
        let (actor, request): (String, Option<String>) = tx.query_row(
            "SELECT principal,request FROM process_command_bindings WHERE id=?1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(actor == principal.to_string(), "rejection actor mismatch");
        let Some(request) = request else {
            return Ok(None);
        };
        let reason: String = reason
            .chars()
            .filter(|ch| !ch.is_control())
            .take(512)
            .collect();
        let receipt = json!({"command_id":id,"status":"rejected","reason":reason});
        tx.execute(
            "INSERT INTO process_commands VALUES(?1,?2,?3)",
            params![id.to_string(), request, serde_json::to_string(&receipt)?],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(Some(receipt))
    }
}
