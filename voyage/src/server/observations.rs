use super::*;
use serde_json::Value;
pub(super) async fn observe(
    state: &Arc<State>,
    after: u64,
    limit: u32,
    wait_ms: u32,
    projection: Option<&str>,
) -> Result<Value> {
    ensure!(wait_ms <= 10_000, "event wait exceeds ten seconds");
    ensure!(
        matches!(
            projection,
            None | Some("public-v1") | Some(voyage_protocol::live_events::PROJECTION)
        ),
        "unsupported event projection"
    );
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(u64::from(wait_ms));
    loop {
        let events = if projection == Some(voyage_protocol::live_events::PROJECTION) {
            state.owner.live_observations(after, limit).await?
        } else {
            state.owner.observations(after, limit).await?
        };
        if events["replay_gap"] == true
            || events["events"]
                .as_array()
                .is_some_and(|events| !events.is_empty())
            || tokio::time::Instant::now() >= deadline
        {
            return Ok(events);
        }
        tokio::select! {_=state.shutdown.cancelled()=>return Ok(events),_=tokio::time::sleep(std::time::Duration::from_millis(100))=>{}}
    }
}
