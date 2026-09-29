use super::*;
use crate::process::routing;
impl Supervisor {
    pub(crate) async fn observe_assignment(
        &self,
        grant: &ProcessGrant,
        id: Uuid,
        cancel: bool,
    ) -> Result<serde_json::Value> {
        ensure!(
            grant.rights.contains(&ProcessRight::History)
                && (!cancel || grant.rights.contains(&ProcessRight::Cancel)),
            "assignment observation or cancellation denied"
        );
        let lock = self.assignment_lock(id).await?;
        let _assignment = lock.lock().await;
        let path = assignment_path(&self.directory, id);
        let mut assignment: Assignment = store::load_bounded(&path, 2 * 1024 * 1024)?;
        ensure!(
            assignment.principal_id == grant.principal_id
                && assignment.request.parent_session_id == grant.session_id,
            "assignment scope denied"
        );
        if assignment.observation.cleanup_observed {
            return Ok(serde_json::to_value(assignment.observation)?);
        }
        // Recover legacy intent from either retained cancellation command or the
        // durable revoked grant, even if older polling overwrote the public state.
        let child_path = store::grant_path(&self.directory, assignment.child_grant_id);
        let revoked = if child_path.exists() {
            store::load::<ProcessGrant>(&child_path)?.revoked
        } else {
            false
        };
        let cancel = cancel
            || assignment.cancellation_requested
            || assignment.cancel.is_some()
            || assignment.observation.state == "cancellation_requested"
            || revoked;
        if cancel {
            assignment.cancellation_requested = true;
            store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
            // Fence future admission before asking a running child to stop. The
            // assignment mutex serializes this with every retry of Assign.
            let child_path = store::grant_path(&self.directory, assignment.child_grant_id);
            if child_path.exists() {
                let mut child: ProcessGrant = store::load(&child_path)?;
                child.revoked = true;
                store::save(&child_path, &child)?;
            }
            assignment.observation.state = "cancellation_requested".into();
            store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
        }
        let registration = match self
            .registration(assignment.observation.child_session_id)
            .await
        {
            Ok(registration) => registration,
            Err(_) => {
                // An unavailable registration is not positive evidence of absence
                // or cleanup, even when cancellation was requested.
                assignment.observation.state = "cleanup_unknown".into();
                assignment.observation.cleanup_observed = false;
                store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
                return Ok(serde_json::to_value(assignment.observation)?);
            }
        };
        let directory = registry::directory(&self.directory, registration.session_id);
        let mut snapshot = match self
            .forward_current(&directory, &registration, RuntimeCommand::Snapshot, None)
            .await
        {
            Ok(response) if response.error.is_none() => response.result,
            _ => {
                assignment.observation.state = "cleanup_unknown".into();
                store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
                return Ok(serde_json::to_value(assignment.observation)?);
            }
        };
        // A terminal child's own bounded reconciliation can observe descendants;
        // it cannot resume the Goal or redispatch their work.
        if assignment.request.budget.is_some()
            && matches!(
                snapshot["run"]["state"].as_str(),
                Some("completed" | "cancelled" | "incomplete" | "failed" | "interrupted")
            )
            && snapshot["execution_usage"].is_object()
            && snapshot["execution_usage"]["cleanup_observed"] != true
            && snapshot["execution_usage_observed"]["cleanup_observed"] != true
        {
            let _ = self
                .forward_current(
                    &directory,
                    &registration,
                    RuntimeCommand::GoalReconcile {
                        offset: 0,
                        limit: 128,
                        fence_children: false,
                    },
                    None,
                )
                .await;
            if let Ok(response) = self
                .forward_current(&directory, &registration, RuntimeCommand::Snapshot, None)
                .await
                && response.error.is_none()
            {
                snapshot = response.result;
            }
        }
        if let Some(run_id) = snapshot["run"]["run_id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
        {
            assignment.observation.run_id = Some(run_id);
            let state = snapshot["run"]["state"].as_str().unwrap_or("unknown");
            if let Some(budget) = &assignment.request.budget
                && let Some(value) = snapshot.get("execution_usage").filter(|v| !v.is_null())
            {
                let base: voyage_protocol::execution_budget::ExecutionUsage =
                    serde_json::from_value(value.clone())?;
                ensure!(
                    base.budget == *budget
                        && base.session_id == registration.session_id
                        && base.run_id == run_id,
                    "participant usage receipt attribution mismatch"
                );
                ensure!(
                    assignment
                        .observation
                        .execution_usage
                        .as_ref()
                        .is_none_or(|prior| prior == &base),
                    "participant usage receipt changed"
                );
                let observed = snapshot
                    .get("execution_usage_observed")
                    .filter(|v| !v.is_null())
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()?
                    .unwrap_or_else(|| base.clone());
                ensure!(
                    observed.observes(&base)
                        && assignment
                            .observation
                            .execution_usage_observed
                            .as_ref()
                            .is_none_or(|prior| observed.observes(prior)),
                    "participant observed usage regressed"
                );
                assignment.observation.execution_usage = Some(base);
                assignment.observation.execution_usage_observed = Some(observed);
            }

            if cancel && matches!(state, "accepted" | "running") {
                let command = assignment
                    .cancel
                    .get_or_insert_with(|| RuntimeCommand::Cancel {
                        command_id: Uuid::new_v4(),
                        expected_revision: snapshot["revision"].as_u64().unwrap_or(0),
                        expires_at_ms: store::now().unwrap_or(0).saturating_add(300000),
                        run_id,
                    })
                    .clone();
                store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
                let _ = self
                    .forward_current(&directory, &registration, command, None)
                    .await?;
                assignment.observation.state = "cancellation_requested".into();
            } else {
                assignment.observation.state = state.to_owned();
                if matches!(
                    state,
                    "completed" | "cancelled" | "incomplete" | "failed" | "interrupted"
                ) && snapshot["pending_cleanup_run"].is_null()
                    && (assignment.request.budget.is_none()
                        || assignment
                            .observation
                            .execution_usage_observed
                            .as_ref()
                            .is_some_and(|usage| usage.cleanup_observed))
                {
                    assignment.observation.cleanup_observed = true;
                    if assignment.observation.result.is_none() {
                        assignment.observation.result = Some(snapshot.clone());
                    }
                    // Child result and cleanup evidence are durable before its process is stopped.
                    store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
                    let _ = self
                        .forward_current(&directory, &registration, RuntimeCommand::Stop, None)
                        .await;
                }
            }
        } else if snapshot.get("run").is_some_and(serde_json::Value::is_null)
            && snapshot["revision"].is_u64()
        {
            let binding: ParticipantBinding = store::load(&binding_path(
                &self.directory,
                assignment.request.binding_id,
            ))?;
            if cancel || binding.cancel_existing {
                // No admitted run plus closed binding prevents a later assignment dispatch.
                if cancel || binding.cancel_existing {
                    let _ = self
                        .forward_current(&directory, &registration, RuntimeCommand::Stop, None)
                        .await;
                    let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                        loop {
                            if routing::inspect(&directory, &registration).await.state
                                == ProcessState::Stopped
                            {
                                return;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                    })
                    .await
                    .is_ok();
                    assignment.observation.state = "cancelled".into();
                    assignment.observation.cleanup_observed = stopped;
                    assignment.observation.admission_closed =
                        stopped && assignment.request.budget.is_some();
                }
            }
        }
        store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
        Ok(serde_json::to_value(assignment.observation)?)
    }
}
