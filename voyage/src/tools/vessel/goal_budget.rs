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
            if let Ok(Ok(result)) = &result
                && result["status"] == "not_admitted"
                && result["command_id"] == command_id.to_string()
                && meter
                    .close_allocation(destination, command_id, result.clone())
                    .await
                    .is_ok()
            {
                return;
            }
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

/// Explicit bounded recovery uses only current host-configured authenticated
/// routes. Missing routes/rights remain pending; no original mutation is sent.
pub(crate) async fn reconcile_goal_allocations(
    owner: &crate::attachment::runtime::ManagedSessionOwner,
    config: &crate::Config,
    offset: u64,
    limit: u32,
    fence_children: bool,
) -> anyhow::Result<Value> {
    let allocations = owner.goal_allocations(offset, limit).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let local = config
        .vessel_context
        .as_ref()
        .map(|c| c.directory.as_path())
        .or(config.vessel.local_directory.as_deref())
        .unwrap_or(Path::new(""));
    let mut routes = Vec::new();
    if config.vessel.enabled {
        if let Ok(route) = transport::Transport::open(local, None) {
            routes.push(route);
        }
        for path in config.vessel.remotes.values() {
            if let Ok(route) = transport::Transport::open(local, Some(path)) {
                routes.push(route);
            }
        }
    }
    let mut matched = Vec::new();
    for route in routes {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        if let Ok(Ok(caps)) =
            tokio::time::timeout_at(deadline, route.exchange(VesselCommand::Capabilities)).await
            && let Ok(target) = serde_json::from_value::<Uuid>(caps["vessel_id"].clone())
        {
            matched.push((target, route));
        }
    }
    let mut results = Vec::new();
    for allocation in &allocations {
        if allocation.closed {
            results.push(json!({"command_id":allocation.budget.command_id,"observed":true,"non_admission":true,"usage_updated":false}));
            continue;
        }
        if allocation.gated
            && allocation.dispatch.is_none()
            && let Ok(changed) = owner
                .close_goal_allocation(allocation.destination, allocation.budget.command_id, None)
                .await
        {
            results.push(json!({"command_id":allocation.budget.command_id,"observed":true,"non_admission":true,"usage_updated":changed}));
            continue;
        }
        let mut changed = Some(false);
        let mut observed = false;
        for (target, route) in matched
            .iter()
            .filter(|(target, _)| *target == allocation.destination)
        {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            let command = match (&allocation.dispatch, fence_children) {
                (
                    Some(crate::provider::goal_meter::AllocationDispatch::Voyage { command }),
                    true,
                ) => VoyageCommand::Resolve {
                    command_id: allocation.budget.command_id,
                    original: Some(command.clone()),
                },
                _ => VoyageCommand::Receipt {
                    command_id: allocation.budget.command_id,
                },
            };
            if let Ok(Ok(result)) = tokio::time::timeout_at(
                deadline,
                voyage(route, allocation.budget.session_id, None, command),
            )
            .await
            {
                if result["status"] == "not_admitted"
                    && result["command_id"] == allocation.budget.command_id.to_string()
                {
                    if let Ok(updated) = owner
                        .close_goal_allocation(*target, allocation.budget.command_id, Some(result))
                        .await
                    {
                        observed = true;
                        changed = Some(updated);
                        break;
                    }
                    continue;
                }
                if let Ok(usage) =
                    serde_json::from_value::<ExecutionUsage>(result["execution_usage"].clone())
                    && usage.budget == allocation.budget
                {
                    let supplemental = match result
                        .get("execution_usage_observed")
                        .filter(|v| !v.is_null())
                    {
                        Some(value) => match serde_json::from_value(value.clone()) {
                            Ok(value) => Some(value),
                            Err(_) => continue,
                        },
                        None => None,
                    };
                    if let Ok(updated) = owner
                        .reconcile_goal_allocation(*target, usage, supplemental)
                        .await
                    {
                        observed = true;
                        changed = Some(updated);
                        break;
                    }
                }
            }
        }
        // Participant grants observe their exact parent assignment rather than
        // acquiring child History rights through a generic Vessel route.
        if !observed {
            for endpoint in config
                .participants
                .iter()
                .filter(|p| p.participant_vessel_id == allocation.destination)
            {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                if let Ok(Ok(result)) = tokio::time::timeout_at(
                    deadline,
                    crate::participant::reconcile(
                        owner.clone(),
                        allocation.budget.parent_run_id,
                        allocation.budget.command_id,
                        &endpoint.name,
                        fence_children,
                        config,
                    ),
                )
                .await
                {
                    observed = result["execution_usage"].is_object()
                        || result["result"]["execution_usage"].is_object()
                        || result["admission_closed"] == true;
                    if observed {
                        changed = None; // Existing assignment API does not return an import delta.
                        break;
                    }
                }
            }
        }
        results.push(json!({"command_id":allocation.budget.command_id,"observed":observed,"usage_updated":changed}));
    }
    Ok(
        json!({"allocations":results,"next_offset":(allocations.len()==limit as usize).then_some(offset.saturating_add(u64::from(limit))),"effects_replayed":false,"continuation_restored":false}),
    )
}

pub(super) async fn submit(
    t: &transport::Transport,
    session: Uuid,
    launch: Option<&crate::Config>,
    command: VoyageCommand,
) -> Result<Value, ToolError> {
    if let Some(meter) = launch.and_then(|c| c.goal_meter.as_ref()) {
        meter
            .prepare_dispatch(crate::provider::goal_meter::AllocationDispatch::Voyage {
                command: Box::new(command.clone()),
            })
            .await
            .map_err(|_| failed("Goal child dispatch could not be durably recorded"))?;
    }
    voyage(t, session, None, command).await
}

#[cfg(all(test, unix))]
#[path = "goal_budget/owned_delegation_journey_standard_tests.rs"]
mod owned_delegation_journey_standard_tests;
