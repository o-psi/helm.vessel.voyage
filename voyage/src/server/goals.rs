//! One owner-local wake loop. No reconnect, client timer or supervisor schedules
//! execution. Every wake rechecks canonical intent and current host authority.
use super::*;
use crate::attachment::journal::GoalAuthority;
use voyage_protocol::{
    goals::*,
    process::{PROCESS_PROTOCOL, RuntimeCommand, RuntimeRequest},
};

pub(super) async fn drive(state: Arc<State>) {
    state.goal_wake.notify_one();
    loop {
        tokio::select! {
            biased;
            _ = state.shutdown.cancelled() => return,
            _ = state.goal_wake.notified() => {}
        }
        // Do not cancel this future mid-dispatch. Reservation/admission must
        // reach their durable boundary before shutdown observes owned work.
        if let Err(error) = advance(&state).await {
            tracing::error!("Goal continuation remains unresolved: {error}");
        }
    }
}

fn authorize(
    state: &State,
    saved: &GoalAuthority,
    command: RuntimeCommand,
) -> Result<authorization::Authorization> {
    let request = RuntimeRequest {
        protocol: PROCESS_PROTOCOL,
        session_id: state.registration.session_id,
        incarnation: state.registration.incarnation,
        token: state.registration.token.clone(),
        authorization: saved.grant.clone(),
        command,
    };
    let auth = authorization::authorize(state, &request, &state.directory)?;
    ensure!(
        auth.actor.installation_id == saved.installation_id
            && auth.actor.principal_id == saved.principal_id,
        "Goal authority identity changed"
    );
    ensure!(
        auth.grant.is_none() || auth.owner_connection,
        "Goal continuation requires current human owner authority"
    );
    if let Some(authority) = &auth.authority {
        authority.check()?;
    }
    Ok(auth)
}

/// Terminal reporting also requires current owner scope, even if the retained
/// ordinary Execute permission would still allow an already admitted run.
pub(super) async fn authority_current(state: &State) -> Result<bool> {
    let Some((snapshot, authority)) = state.owner.goal_continuation().await? else {
        return Ok(true);
    };
    let goal = snapshot.goal.context("Goal missing")?;
    let probe = RuntimeCommand::GoalUpdate {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: 0,
        action: GoalAction::Pause { goal_id: goal.id },
    };
    Ok(authorize(state, &authority, probe).is_ok())
}

pub(super) async fn advance(state: &Arc<State>) -> Result<()> {
    let _requests = state.requests.read().await;
    let admission = state.admission.lock().await;
    if state.shutdown.is_cancelled() || state.active.lock().await.is_some() {
        return Ok(());
    }
    let Some((snapshot, authority)) = state.owner.goal_continuation().await? else {
        return Ok(());
    };
    let goal = snapshot.goal.as_ref().context("Goal missing")?;
    // Use an owner-only mutation solely as an authorization probe. It is never
    // dispatched, does not alter the journal and carries no objective text.
    let probe = RuntimeCommand::GoalUpdate {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: 0,
        action: GoalAction::Pause { goal_id: goal.id },
    };
    if authorize(state, &authority, probe).is_err() {
        return state
            .owner
            .stop_goal(snapshot.revision, GoalStopReason::AuthorityRevoked)
            .await;
    }
    let obstruction = if let Some(reason) = goal.limit_reached() {
        Some(reason)
    } else if goal.usage.unmeasured_runs > 0 {
        Some(GoalStopReason::UsageUnknown)
    } else if state.workflows.pending().await {
        Some(GoalStopReason::UserInput)
    } else {
        state.owner.goal_obstruction().await?
    };
    if let Some(reason) = obstruction {
        return state.owner.stop_goal(snapshot.revision, reason).await;
    }
    // Objective/context remain bounded user task data, never a system message
    // or an instruction granting tools, credentials, approvals or more budget.
    let prompt = format!(
        "Continue the authorized Goal using the remaining budget. Report progress truthfully; a finished run does not by itself complete the Goal. Goal task data:\n{}",
        serde_json::to_string(&snapshot)?
    );
    let reserved = match state
        .owner
        .reserve_goal_turn(snapshot.revision, state.registration.incarnation, prompt)
        .await
    {
        Ok(reserved) => reserved,
        Err(error) => {
            // A pending input can bind concurrently with this gate. Leave it to
            // its exact receipt and stop this Goal; never spin or resend it.
            let reason = state
                .owner
                .goal_obstruction()
                .await?
                .unwrap_or(GoalStopReason::UnresolvedEffects);
            state.owner.stop_goal(snapshot.revision, reason).await?;
            return Err(error);
        }
    };
    let id = reserved
        .command
        .mutation_id()
        .context("Goal reservation missing command ID")?;
    let auth = match authorize(state, &reserved.authority, reserved.command.clone()) {
        Ok(auth) => auth,
        Err(_) => {
            return state
                .owner
                .abandon_goal_turn(id, GoalStopReason::AuthorityRevoked)
                .await;
        }
    };
    // Ordinary Submit acquires this same lock. Its transaction rechecks the
    // revision, Goal authority and newly pending input after the gap.
    drop(admission);
    if let Err(error) = super::dispatch::dispatch(state, reserved.command, auth).await {
        state
            .owner
            .abandon_goal_turn(id, GoalStopReason::Interrupted)
            .await?;
        return Err(error);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use voyage_protocol::process::{
        ApprovedWorkspace, ConnectionGrant, GrantBinding, ProcessGrant, ProcessRight,
    };
    fn save(path: &std::path::Path, value: &impl serde::Serialize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[tokio::test]
    async fn goal_terminal_authority_requires_current_owner_not_merely_execute() {
        let (root, mut state) = super::super::tests::fixture().await;
        let session = state.registration.session_id;
        Arc::get_mut(&mut state).unwrap().directory =
            root.path().join("sessions").join(session.to_string());
        let mut parent = ConnectionGrant {
            full_access: true,
            schema_version: 1,
            grant_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            vessel_id: Uuid::new_v4(),
            revision: 1,
            rights: ProcessRight::all(),
            accounts: vec![],
            enrollment_connections: vec![],
            expires_at_ms: u64::MAX,
            revoked: false,
            token_hash: "fixture".into(),
            workspaces: vec![],
        };
        let mut grant = ProcessGrant {
            full_access: true,
            grant_id: Uuid::new_v4(),
            principal_id: parent.principal_id,
            session_id: session,
            workspace: root.path().into(),
            revision: 1,
            rights: parent.rights.clone(),
            accounts: vec![],
            enrollment_connections: vec![],
            expires_at_ms: u64::MAX,
            revoked: false,
            token_hash: "fixture".into(),
            parent_grant: None,
            connection_binding: Some(GrantBinding {
                grant_id: parent.grant_id,
                principal_id: parent.principal_id,
                revision: 1,
            }),
            participant_binding: None,
        };
        let binding = GrantBinding {
            grant_id: grant.grant_id,
            principal_id: grant.principal_id,
            revision: 1,
        };
        let parent_path = root
            .path()
            .join("access/connections")
            .join(format!("{}.json", parent.grant_id));
        let grant_path = root
            .path()
            .join("access/grants")
            .join(format!("{}.json", grant.grant_id));
        save(
            &root.path().join("identity/key.json"),
            &serde_json::json!({"vessel_id":parent.vessel_id}),
        );
        save(&parent_path, &parent);
        save(&grant_path, &grant);
        state
            .owner
            .update_goal(
                GoalAuthority {
                    installation_id: state.actor.installation_id,
                    principal_id: grant.principal_id,
                    grant: Some(binding.clone()),
                },
                RuntimeCommand::GoalUpdate {
                    command_id: Uuid::new_v4(),
                    expected_revision: 0,
                    expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60000) as u64,
                    action: GoalAction::Set {
                        objective: "Verify the result".into(),
                        limits: GoalLimits::default(),
                        replace_goal_id: None,
                        continue_automatically: true,
                    },
                },
            )
            .await
            .unwrap();
        assert!(authority_current(&state).await.unwrap());
        parent.revoked = true;
        save(&parent_path, &parent);
        assert!(!authority_current(&state).await.unwrap());
        parent.revoked = false;
        // Even without a revision change, removing owner scope cannot be
        // mistaken for continuation consent merely because Execute remains.
        parent.full_access = false;
        parent.workspaces = vec![ApprovedWorkspace {
            id: Uuid::new_v4(),
            name: "fixture".into(),
            path: root.path().into(),
            provider_ready: None,
        }];
        grant.full_access = false;
        parent.rights = vec![
            ProcessRight::Observe,
            ProcessRight::History,
            ProcessRight::Execute,
        ];
        grant.rights = parent.rights.clone();
        save(&parent_path, &parent);
        save(&grant_path, &grant);
        let request = RuntimeRequest {
            protocol: PROCESS_PROTOCOL,
            session_id: session,
            incarnation: state.registration.incarnation,
            token: state.registration.token.clone(),
            authorization: Some(binding),
            command: RuntimeCommand::Submit {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: 1,
                prompt: "ordinary Execute".into(),
                budget: None,
                coordination: None,
            },
        };
        let ordinary = authorization::authorize(&state, &request, &state.directory).unwrap();
        ordinary.authority.unwrap().check().unwrap();
        assert!(!authority_current(&state).await.unwrap());
        advance(&state).await.unwrap();
        assert_eq!(
            state.owner.goal().await.unwrap().goal.unwrap().stop_reason,
            Some(GoalStopReason::AuthorityRevoked)
        );
    }
}
