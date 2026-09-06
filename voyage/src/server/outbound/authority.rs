use super::*;
#[derive(Debug)]
pub(super) struct Authority {
    pub(super) lease: ConnectionLease,
    pub(super) consent: Arc<RemoteGrantObserver>,
}
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> Result<()> {
        ensure!(
            self.lease.is_active(),
            "foreground connection authority unavailable"
        );
        self.consent.check()
    }
}
/// Dispatch combines the still-live remote grant with the locally selected
/// policy snapshot. Remote observation/cancellation require the current lease and
/// durable grant; a changed local profile does not obstruct them. Local cleanup
/// and recovery remain available independently after withdrawal.
#[derive(Debug)]
pub(super) struct DispatchAuthority {
    pub(super) lease: Arc<Authority>,
    pub(super) policy: Policy,
}
impl crate::policy::ExecutionAuthority for DispatchAuthority {
    fn check(&self) -> Result<()> {
        self.lease.check()?;
        self.policy.check_current()
    }
}
pub(super) fn features() -> Features {
    Features::new(vec![
        Feature::SequencedEvents,
        Feature::Replay,
        Feature::ToolActivity,
        Feature::Usage,
        Feature::ManagedExecution,
    ])
    .expect("fixed features")
}
pub(super) fn public_reply(mut reply: Reply, redactor: &Redactor) -> Reply {
    match &mut reply {
        Reply::ExecutionSnapshot { session, .. } | Reply::Session { session } => {
            session.name = redactor.redact(&session.name);
            session.model = redactor.redact(&session.model);
        }
        Reply::Sessions { sessions } => {
            for session in sessions {
                session.name = redactor.redact(&session.name);
                session.model = redactor.redact(&session.model);
            }
        }
        _ => {}
    }
    reply
}
