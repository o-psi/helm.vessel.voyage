//! Bind model-requested child execution to a durable parent allocation. This
//! observer only reads exact receipts; it never repeats uncertain submissions.
use super::*;
use voyage_protocol::execution_budget::{ExecutionBudget, ExecutionUsage};

pub(super) async fn prepare(
    action: &Action,
    transport: &transport::Transport,
    launch: Option<&crate::Config>,
) -> Result<Option<ExecutionBudget>, ToolError> {
    let Some(meter) = launch.and_then(|config| config.goal_meter.as_ref()) else {
        return Ok(None);
    };
    let (command_id,session_id)=match action {
        Action::Create {command_id,session_id,..}|Action::Submit {command_id,session_id,..}=>(*command_id,*session_id),
        Action::Steer {..}=>return Err(ToolError::Denied("Goal delegation requires a bounded Submit; steering cannot establish a child execution budget".into())),
        _=>return Ok(None),
    };
    let caps = transport.exchange(VesselCommand::Capabilities).await?;
    if !caps["features"]
        .as_array()
        .is_some_and(|features| features.iter().any(|feature| feature == "execution_budget"))
    {
        return Err(failed(
            "target Vessel does not support bounded execution; upgrade before delegating Goal work",
        ));
    }
    let destination: Uuid = serde_json::from_value(caps["vessel_id"].clone())
        .map_err(|_| failed("target Vessel identity unavailable"))?;
    if matches!(action, Action::Submit { .. }) {
        let snapshot = voyage(transport, session_id, None, VoyageCommand::Snapshot).await?;
        if snapshot.get("revision").and_then(Value::as_u64).is_none()
            || snapshot.get("run").is_none()
            || snapshot.get("pending_cleanup_run").is_none()
            || matches!(
                snapshot["run"]["state"].as_str(),
                Some("accepted" | "running")
            )
            || !snapshot["pending_cleanup_run"].is_null()
        {
            return Err(failed(
                "Goal child submission requires observed idle state and cleanup",
            ));
        }
    }
    let budget = meter
        .allocate(command_id, destination, session_id)
        .await
        .map_err(|_| failed("Goal child allocation refused or uncertain"))?;
    if !meter.observe_allocation_once(command_id) {
        return Ok(Some(budget));
    }
    let transport = transport.clone();
    let meter = meter.clone();
    let observed = budget.clone();
    // Launch observation before the first possible external effect. Timeouts and
    // cancellation leave a durable pending allocation, never a zero-cost refund.
    tokio::spawn(async move {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(u128::MAX);
        let remaining = u128::from(observed.expires_at_ms)
            .saturating_add(5000)
            .saturating_sub(now)
            .min(u128::from(86_405_000u64)) as u64;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(remaining);
        loop {
            let result = tokio::time::timeout_at(
                deadline,
                voyage(
                    &transport,
                    session_id,
                    None,
                    VoyageCommand::Receipt { command_id },
                ),
            )
            .await;
            if let Ok(Ok(result)) = result
                && let Some(value) = result
                    .get("execution_usage")
                    .filter(|value| !value.is_null())
                && let Ok(usage) = serde_json::from_value::<ExecutionUsage>(value.clone())
            {
                if usage.budget != observed {
                    return;
                }
                if meter.settle_allocation(destination, usage).await.is_ok() {
                    return;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep_until(
                (tokio::time::Instant::now() + Duration::from_secs(1)).min(deadline),
            )
            .await;
        }
    });
    Ok(Some(budget))
}

#[cfg(all(test, unix))]
mod tests;
