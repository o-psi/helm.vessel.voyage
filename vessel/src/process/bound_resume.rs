//! Automatic ordinary wake requires both protected retirement and a fenced
//! positive suspension observation by the original executing UID.
use super::{accounts::Scope, database, guardian, service::Supervisor};
use anyhow::{Result, ensure};
use std::path::Path;
use uuid::Uuid;
use voyage_protocol::{execution_identity::*, process::*};

pub(super) fn retired(root: &Path, registration: &ProcessRegistration) -> bool {
    guardian::cleanup_observed(root, registration.session_id, registration.incarnation)
        .unwrap_or(false)
        || super::migration::dormant(root, registration.session_id, registration.incarnation)
            .unwrap_or(false)
        || super::execution_transition::dormant(
            root,
            registration.session_id,
            registration.incarnation,
        )
        .unwrap_or(false)
}

fn ordinary(
    identity: &ConfiguredExecutionIdentity,
    registration: &ProcessRegistration,
) -> Result<()> {
    ensure!(
        identity.enabled
            && identity.authority == AuthorityClass::Ordinary
            && identity.uid != 0
            && identity.gid != 0
            && !identity.supplementary_groups.contains(&0)
            && !matches!(
                registration.state,
                ProcessState::Stopped
                    | ProcessState::CleanupUnconfirmed
                    | ProcessState::Relinquished
            ),
        "automatic wake requires a current ordinary suspended identity"
    );
    Ok(())
}

fn may_wake(command: &RuntimeCommand) -> bool {
    !command.observes_suspended()
        && !matches!(
            command,
            RuntimeCommand::ExecuteTool { .. }
                | RuntimeCommand::Terminal { .. }
                | RuntimeCommand::Steer { .. }
                | RuntimeCommand::Respond { .. }
                | RuntimeCommand::Cancel { .. }
                | RuntimeCommand::HostBrowser { .. }
                | RuntimeCommand::HostBrowserDisconnected { .. }
                | RuntimeCommand::Relinquish { .. }
        )
}

pub(super) fn suspension_response(
    response: &RuntimeResponse,
    registration: &ProcessRegistration,
) -> Result<()> {
    ensure!(
        response.protocol == PROCESS_PROTOCOL
            && response.session_id == registration.session_id
            && response.incarnation == registration.incarnation
            && response.resumed_from.is_none()
            && response.error.is_none()
            && !response.outcome_unknown
            && response.result["session_id"] == registration.session_id.to_string()
            && response.result["incarnation"] == registration.incarnation.to_string()
            && response.result["suspended"] == true
            && response.result.get("pid") == Some(&serde_json::Value::Null),
        "positive fenced suspension observation unavailable"
    );
    Ok(())
}

fn scope(
    root: &Path,
    registration: &ProcessRegistration,
    binding: Option<&GrantBinding>,
    command: &RuntimeCommand,
) -> Result<()> {
    let Some(binding) = binding else {
        return Ok(());
    };
    let grant: ProcessGrant =
        super::access::store::load(&super::access::store::grant_path(root, binding.grant_id))?;
    ensure!(
        grant.grant_id == binding.grant_id
            && grant.principal_id == binding.principal_id
            && grant.revision == binding.revision
            && grant.session_id == registration.session_id,
        "wake scope binding changed"
    );
    let right = required_process_right(command)
        .or_else(|| {
            grant
                .full_access
                .then(|| owner_connection_right(command))
                .flatten()
        })
        .ok_or_else(|| anyhow::anyhow!("operation unavailable to scoped clients"))?;
    let current = Scope::Session(grant);
    current.check(root, &registration.workspace, right)?;
    if command.requires_browser_history() {
        current.check(root, &registration.workspace, ProcessRight::History)?;
    }
    Ok(())
}

impl Supervisor {
    pub(super) async fn resume_bound_locked(
        &self,
        previous: &ProcessRegistration,
        command: &RuntimeCommand,
        authorization: Option<&GrantBinding>,
    ) -> Result<()> {
        ensure!(
            may_wake(command),
            "retired live resources cannot wake a replacement owner"
        );
        let identity = database::bound_observer_identity(&self.directory, previous).await?;
        ordinary(&identity, previous)?;
        super::launch::validate_identity(&identity)?;
        scope(&self.directory, previous, authorization, command)?;
        ensure!(
            guardian::suspension_candidate(
                &self.directory,
                previous.session_id,
                previous.incarnation
            )
            .unwrap_or(false),
            "automatic wake requires protected successful suspension cleanup"
        );
        let directory = super::runtime_storage::directory(&self.directory, previous).await?;
        let health = self
            .observe_current(&directory, previous, RuntimeCommand::Health, None)
            .await?;
        suspension_response(&health, previous)?;
        // The original UID parses its saved account/configuration. Root receives
        // only the public snapshot's safe binding, never provider credentials.
        let snapshot = self
            .observe_current(&directory, previous, RuntimeCommand::Snapshot, None)
            .await?;
        ensure!(
            snapshot.error.is_none() && !snapshot.outcome_unknown,
            "saved account observation unavailable"
        );
        ensure!(
            snapshot.result["inference"].is_object(),
            "saved execution configuration unavailable"
        );
        let account: Option<voyage_protocol::accounts::AccountBinding> =
            serde_json::from_value(snapshot.result["inference"]["account"].clone())?;
        if let Some(account) = account {
            self.validate_identity_account_binding(previous, authorization, &account, false)
                .await?;
        }
        // Recheck after the private helpers and immediately before the exact
        // SQLite admission. No child-owned projection can select an identity.
        ensure!(
            database::bound_observer_identity(&self.directory, previous).await? == identity,
            "wake identity changed during preflight"
        );
        scope(&self.directory, previous, authorization, command)?;
        ensure!(
            guardian::suspension_candidate(
                &self.directory,
                previous.session_id,
                previous.incarnation
            )
            .unwrap_or(false),
            "suspension authority changed during wake"
        );
        let result = self
            .restart_bound_locked(Uuid::new_v4(), previous.session_id, previous.incarnation)
            .await?;
        if result["state"] != "live" {
            return Err(anyhow::anyhow!(
                "replacement admission did not become live; inspect retained incarnation"
            )
            .context(super::routing::OutcomeUnknown));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::Fixture;
    use super::*;
    use serde_json::json;

    #[test]
    fn suspended_health_must_be_exact_positive_and_complete() {
        let fixture = Fixture::new();
        let registration = fixture.registration();
        let original = RuntimeResponse {
            protocol: PROCESS_PROTOCOL,
            session_id: registration.session_id,
            incarnation: registration.incarnation,
            resumed_from: None,
            result: json!({"session_id":registration.session_id,"incarnation":registration.incarnation,
                "suspended":true,"pid":null}),
            error: None,
            outcome_unknown: false,
        };
        assert!(suspension_response(&original, &registration).is_ok());
        for (field, value) in [
            ("session_id", json!(Uuid::new_v4())),
            ("incarnation", json!(Uuid::new_v4())),
            ("suspended", json!(false)),
            ("suspended", json!(null)),
            ("pid", json!(1)),
        ] {
            let mut changed = original.clone();
            changed.result[field] = value;
            assert!(
                suspension_response(&changed, &registration).is_err(),
                "{field}"
            );
        }
        for field in ["session_id", "incarnation", "suspended", "pid"] {
            let mut changed = original.clone();
            changed.result.as_object_mut().unwrap().remove(field);
            assert!(
                suspension_response(&changed, &registration).is_err(),
                "missing {field}"
            );
        }
        let mut changed = original.clone();
        changed.outcome_unknown = true;
        assert!(suspension_response(&changed, &registration).is_err());
        let mut changed = original.clone();
        changed.error = Some("unconfirmed".into());
        assert!(suspension_response(&changed, &registration).is_err());
        let mut changed = original.clone();
        changed.resumed_from = Some(registration.incarnation);
        assert!(suspension_response(&changed, &registration).is_err());
        let mut changed = original.clone();
        changed.protocol += 1;
        assert!(suspension_response(&changed, &registration).is_err());
        let mut changed = original.clone();
        changed.session_id = Uuid::new_v4();
        assert!(suspension_response(&changed, &registration).is_err());
        let mut changed = original;
        changed.incarnation = Uuid::new_v4();
        assert!(suspension_response(&changed, &registration).is_err());
    }

    #[test]
    fn automatic_wake_never_elevates_or_revives_operator_retirement() {
        let fixture = Fixture::new();
        let mut registration = fixture.registration();
        registration.state = ProcessState::Live;
        let identity: ConfiguredExecutionIdentity = serde_json::from_value(json!({
            "identity":{"id":Uuid::new_v4(),"revision":1},"label":"ordinary fixture",
            "user_name":"offline-fixture","uid":1001,"gid":1001,"supplementary_groups":[],
            "home":"/fixture/home","account_context":{"id":Uuid::new_v4(),"revision":1},
            "authority":"ordinary","enabled":true
        }))
        .unwrap();
        assert!(ordinary(&identity, &registration).is_ok());
        for authority in [AuthorityClass::Administrator, AuthorityClass::Unknown] {
            let mut changed = identity.clone();
            changed.authority = authority;
            assert!(ordinary(&changed, &registration).is_err());
        }
        let mut changed = identity.clone();
        changed.enabled = false;
        assert!(ordinary(&changed, &registration).is_err());
        let mut changed = identity.clone();
        changed.uid = 0;
        assert!(ordinary(&changed, &registration).is_err());
        let mut changed = identity.clone();
        changed.gid = 0;
        assert!(ordinary(&changed, &registration).is_err());
        let mut changed = identity.clone();
        changed.supplementary_groups.push(0);
        assert!(ordinary(&changed, &registration).is_err());
        for state in [
            ProcessState::Stopped,
            ProcessState::CleanupUnconfirmed,
            ProcessState::Relinquished,
        ] {
            registration.state = state;
            assert!(ordinary(&identity, &registration).is_err());
        }
    }

    #[test]
    fn read_and_retired_live_resources_never_wake_an_owner() {
        for command in [
            RuntimeCommand::Health,
            RuntimeCommand::Snapshot,
            RuntimeCommand::History {
                offset: 0,
                limit: 10,
                expected_revision: None,
            },
            RuntimeCommand::Resolve {
                command_id: Uuid::new_v4(),
                original: None,
            },
            RuntimeCommand::Cancel {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: 1,
                run_id: Uuid::new_v4(),
            },
            RuntimeCommand::HostBrowserDisconnected {
                socket: voyage_protocol::host_browser::HostBrowserSocket {
                    socket_id: Uuid::new_v4(),
                },
            },
        ] {
            assert!(!may_wake(&command));
        }
    }
}
