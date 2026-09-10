//! Suspension is a clean lifecycle transition, never cancellation of active work.
use super::*;

pub(super) async fn suspend(state: &Arc<State>) -> Result<()> {
    // Observation tasks run independently of these short admission/dispatch gates.
    state.cleanup.advance(1).await?;
    let _requests = state.requests.write().await;
    let _admission = state.admission.lock().await;
    if state.shutdown.is_cancelled() || state.active.lock().await.is_some() {
        return Ok(());
    }
    ensure!(
        !state.workflows.pending().await,
        "private turn preparation remains pending"
    );
    ensure!(
        !state.browser.blocks_suspension()?,
        "local browser lease or cleanup remains active"
    );
    let snapshot = state.owner.process_snapshot().await?;
    ensure!(
        snapshot["pending_cleanup_run"].is_null(),
        "run cleanup remains unconfirmed"
    );
    ensure!(
        !matches!(
            snapshot["run"]["state"].as_str(),
            Some("accepted" | "running")
        ),
        "turn is still active"
    );
    state.controls.shutdown_retained(&state.owner).await?;
    ensure!(
        state
            .owner
            .session_resources()
            .await?
            .as_array()
            .is_some_and(Vec::is_empty),
        "session resources remain unresolved"
    );
    state
        .suspend_requested
        .store(true, std::sync::atomic::Ordering::Release);
    state.shutdown.cancel();
    Ok(())
}
