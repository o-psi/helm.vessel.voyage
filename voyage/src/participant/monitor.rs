use super::*;
/// Observation only after acceptance; cancellation is an explicit owned-run signal.
pub(super) fn spawn(
    parent: Arc<Parent>,
    endpoint: ParticipantEndpoint,
    id: Uuid,
    cancel: tokio_util::sync::CancellationToken,
) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3600);
        let mut cancellation_sent = false;
        while tokio::time::Instant::now() < deadline {
            if let Ok(prior) = parent.owner.assignment_result(parent.run_id, id).await
                && prior["cleanup_observed"] == true
            {
                return;
            }
            let request_cancel = cancel.is_cancelled() && !cancellation_sent;
            if let Ok(observation) = parent.observe(&endpoint, id, request_cancel).await {
                if request_cancel {
                    cancellation_sent = true;
                }
                if observation.cleanup_observed {
                    return;
                }
            }
            // Authority loss/partition retains the canonical unresolved obligation.
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}
