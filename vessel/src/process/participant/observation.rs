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
        if cancel {
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
                if cancel {
                    assignment.observation.state = "cancelled".into();
                    assignment.observation.cleanup_observed = true;
                    store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
                }
                return Ok(serde_json::to_value(assignment.observation)?);
            }
        };
        let directory = registry::directory(&self.directory, registration.session_id);
        let snapshot = match self
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
        if let Some(run_id) = snapshot["run"]["run_id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
        {
            assignment.observation.run_id = Some(run_id);
            let state = snapshot["run"]["state"].as_str().unwrap_or("unknown");
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
        } else {
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
                }
            }
        }
        store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
        Ok(serde_json::to_value(assignment.observation)?)
    }
}
