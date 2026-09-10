//! Durable managed input. No default authority grant or raw channel is exposed.
use super::*;
use crate::attachment::journal::{
    SteeringActor, SteeringAdmission, SteeringOutcome, SteeringRejection,
};

/// Recheck current local authorization, without reusing a stored positive grant.
/// Called under the owner mutex: implementations must be bounded/non-reentrant.
/// Network/frontends still authorize every other action at their own boundary.
pub trait SteeringAuthorization: Send + Sync {
    fn authorize(&self, actor: SteeringActor, session_id: Uuid, run_id: Uuid)
    -> anyhow::Result<()>;
}
impl<F> SteeringAuthorization for F
where
    F: Fn(SteeringActor, Uuid, Uuid) -> anyhow::Result<()> + Send + Sync,
{
    fn authorize(
        &self,
        actor: SteeringActor,
        session_id: Uuid,
        run_id: Uuid,
    ) -> anyhow::Result<()> {
        self(actor, session_id, run_id)
    }
}
/// Clones retain the run token/fence; drop them before admitting a later turn.
#[derive(Clone)]
pub struct ManagedSteeringHandle {
    checkpoint: ManagedRunCheckpoint,
    sender: SteeringSender,
}
impl RunOwner {
    /// Must be configured explicitly before executing this turn. The callback
    /// consults current authority both at submission and before dispatch checkpoints.
    pub fn enable_steering(
        &mut self,
        authority: Arc<dyn SteeringAuthorization>,
    ) -> anyhow::Result<ManagedSteeringHandle> {
        self.enable_steering_with_clock(authority, Arc::new(SystemClock))
    }
    pub(super) fn enable_steering_with_clock(
        &mut self,
        authority: Arc<dyn SteeringAuthorization>,
        clock: Arc<dyn RuntimeClock>,
    ) -> anyhow::Result<ManagedSteeringHandle> {
        anyhow::ensure!(self.input.is_some(), "turn already executing");
        anyhow::ensure!(
            self.token
                .steering_authority
                .set((authority, clock))
                .is_ok(),
            "steering authority already configured"
        );
        Ok(ManagedSteeringHandle {
            checkpoint: self.checkpoint(),
            sender: self.steering_sender.clone(),
        })
    }
}
impl ManagedSteeringHandle {
    /// Commit queue evidence before delivery. Exact retries observe durable state,
    /// never resend. Failed rejection persistence poisons further dispatch/acceptance.
    pub async fn submit(&self, request: SteeringAdmission) -> anyhow::Result<SteeringOutcome> {
        self.submit_after_queue(request, || {}).await
    }
    pub(super) async fn submit_after_queue(
        &self,
        request: SteeringAdmission,
        after_queue: impl FnOnce() + Send + 'static,
    ) -> anyhow::Result<SteeringOutcome> {
        let checkpoint = self.checkpoint.clone();
        let sender = self.sender.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = checkpoint
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            anyhow::ensure!(
                store.run_id == checkpoint.token.run_id
                    && request.run_id == store.run_id
                    && request.session_id == store.session_id,
                "steering handle belongs to another turn"
            );
            anyhow::ensure!(
                !checkpoint.token.poisoned.load(Ordering::SeqCst),
                "steering persistence uncertain"
            );
            let (authority, clock) = checkpoint
                .token
                .steering_authority
                .get()
                .ok_or_else(|| anyhow::anyhow!("steering authority unavailable"))?;
            authority.authorize(request.actor, request.session_id, request.run_id)?;
            let Store { journal, guard, .. } = &mut *store;
            let mut outcome =
                journal.queue_steering_with_clock(guard, &request, || clock.now_ms())?;
            if !outcome.duplicate {
                after_queue();
                if let Err(error) = sender.try_send_message(outcome.record.queued_message()) {
                    let reason = match error {
                        crate::agent::SteeringError::Full(_) => SteeringRejection::QueueFull,
                        _ => SteeringRejection::Closed,
                    };
                    match journal.reject_steering(guard, request.run_id, request.receipt_id, reason)
                    {
                        Ok(record) => outcome.record = record,
                        Err(error) => {
                            checkpoint.token.poisoned.store(true, Ordering::SeqCst);
                            return Err(error);
                        }
                    }
                }
            }
            if !outcome.duplicate {
                checkpoint.token.title_input.notify_one();
            }
            Ok(outcome)
        })
        .await?
    }
}
pub(super) fn authorize(store: &Store, token: &TurnToken) -> anyhow::Result<()> {
    anyhow::ensure!(
        !token.poisoned.load(Ordering::SeqCst),
        "steering persistence uncertain"
    );
    for actor in store.journal.steering_actors(store.run_id)? {
        let (authority, _) = token
            .steering_authority
            .get()
            .ok_or_else(|| anyhow::anyhow!("steering authority unavailable"))?;
        authority.authorize(actor, store.session_id, store.run_id)?;
    }
    Ok(())
}
