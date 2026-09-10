//! Positive evidence for failures before constructing any execution resources.
use super::*;
impl Journal {
    pub(crate) fn has_cleanup_attestation(&self, session: Uuid) -> Result<bool> {
        let local:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM local_cleanup_obligations WHERE session_id=?1 AND confirmation='operator_attested')",[session.to_string()],|r|r.get(0))?;
        let resource:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM process_session_resources WHERE session_id=?1 AND state='operator_attested')",[session.to_string()],|r|r.get(0))?;
        let retained: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM process_retained_cleanup WHERE session_id=?1 AND confirmation='operator_attested')", [session.to_string()], |r| r.get(0))?;
        Ok(local || resource || retained)
    }
    pub(crate) fn startup_cleanup_clear(&self, guard: &ExecutionGuard) -> Result<bool> {
        self.check_guard(guard, guard.session_id)?;
        let active: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND active=1)",
            [guard.session_id.to_string()],
            |r| r.get(0),
        )?;
        Ok(!active
            && catalogue::pending_cleanup(&self.connection, guard.session_id)?.is_none()
            && session_resources::pending(&self.connection, guard.session_id)? == 0)
    }
}
