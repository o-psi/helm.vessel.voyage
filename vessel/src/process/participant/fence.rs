//! Positive no-admission evidence for cancellation crossing an uncertain network.
use super::*;
impl Supervisor {
    pub(crate) async fn fence_assignment(
        &self,
        grant: &ProcessGrant,
        request: AssignmentRequest,
    ) -> Result<serde_json::Value> {
        ensure!(
            grant.rights.contains(&ProcessRight::History)
                && grant.rights.contains(&ProcessRight::Cancel),
            "assignment cancellation denied"
        );
        ensure!(
            !request.assignment_id.is_nil()
                && request.assignment_id != request.parent_session_id
                && request.parent_session_id == grant.session_id,
            "assignment parent scope mismatch"
        );
        let lock = self.assignment_lock(request.assignment_id).await?;
        let serial = lock.lock().await;
        initialize(&self.directory)?;
        let path = assignment_path(&self.directory, request.assignment_id);
        if path.exists() {
            let prior: Assignment = store::load_bounded(&path, 2 * 1024 * 1024)?;
            ensure!(
                identity::digest(&prior.request)? == identity::digest(&request)?
                    && prior.principal_id == grant.principal_id,
                "assignment cancellation payload conflict"
            );
            drop(serial);
            return self
                .observe_assignment(grant, request.assignment_id, true)
                .await;
        }
        let binding: ParticipantBinding =
            store::load(&binding_path(&self.directory, request.binding_id))?;
        ensure!(
            binding.principal_id == grant.principal_id
                && binding.parent_vessel_id == request.parent_vessel_id
                && binding.parent_session_id == request.parent_session_id,
            "assignment cancellation binding mismatch"
        );
        ensure!(
            std::fs::read_dir(root(&self.directory).join("assignments"))?
                .take(4096)
                .count()
                < 4096,
            "assignment retention capacity exhausted"
        );
        let observation = AssignmentObservation {
            assignment_id: request.assignment_id,
            participant_vessel_id: identity::public(&self.directory)?.vessel_id,
            parent_session_id: request.parent_session_id,
            parent_run_id: request.parent_run_id,
            child_session_id: request.assignment_id,
            child_incarnation: None,
            run_id: None,
            state: "cancelled".into(),
            cleanup_observed: true,
            result: None,
        };
        let assignment = Assignment {
            request,
            principal_id: grant.principal_id,
            source_grant: GrantBinding {
                grant_id: grant.grant_id,
                revision: grant.revision,
                principal_id: grant.principal_id,
            },
            child_grant_id: Uuid::new_v4(),
            start_command_id: Uuid::new_v4(),
            observation: observation.clone(),
            cancel: None,
        };
        // This immutable terminal record excludes every late exact Assign retry.
        store::save_bounded(&path, &assignment, 2 * 1024 * 1024)?;
        Ok(serde_json::to_value(observation)?)
    }
}
