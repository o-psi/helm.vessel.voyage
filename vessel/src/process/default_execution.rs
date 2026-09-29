//! A public configured start selects an administrator-provisioned ordinary
//! identity. Configuration and provider state are opened by the dropped runtime,
//! never by the root supervisor's login account.
use super::{database, service::Supervisor};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use voyage_protocol::{
    execution_identity::{AuthorityClass, ConfiguredExecutionIdentity, ExecutionBinding},
    process::{ProcessPeerUids, VesselCommand},
};

pub(super) const DEFAULT_EXECUTION_RECORD: &str = "default-execution.json";

fn validate_default(identity: &ConfiguredExecutionIdentity) -> Result<()> {
    ensure!(
        identity.enabled
            && identity.authority == AuthorityClass::Ordinary
            && identity.uid != 0
            && identity.gid != 0
            && !identity.supplementary_groups.contains(&0)
            && !identity.supplementary_groups.contains(&identity.gid),
        "default execution requires an enabled ordinary account without root groups"
    );
    super::launch::validate_identity(identity)
}

fn protected_default(root: &Path) -> Result<ConfiguredExecutionIdentity> {
    let control = voyage_storage::protected_linux::RootDirectory::open(root)?;
    let identity: ConfiguredExecutionIdentity =
        serde_json::from_slice(&control.read(DEFAULT_EXECUTION_RECORD.as_ref(), 16384)?)?;
    validate_default(&identity)?;
    Ok(identity)
}

fn binding(identity: &ConfiguredExecutionIdentity, session: Uuid) -> Result<ExecutionBinding> {
    // These digests identify the ordinary launch selection. They are not an
    // administrator review or a claim to have read user-owned provider secrets.
    let identity_bytes = serde_json::to_vec(identity)?;
    let mut host = Sha256::new();
    host.update(b"voyage/ordinary-host-identity/v1\0");
    host.update(&identity_bytes);
    let mut policy = Sha256::new();
    policy.update(b"voyage/configured-ordinary-start/v1\0");
    policy.update(&identity_bytes);
    Ok(ExecutionBinding {
        session_id: session,
        incarnation: Uuid::new_v4(),
        identity: identity.identity.clone(),
        account_context: identity.account_context.clone(),
        peer_uids: ProcessPeerUids {
            supervisor: 0,
            runtime: identity.uid,
        },
        administrator_grant_id: None,
        host_identity_digest: format!("{:x}", host.finalize()),
        policy_digest: format!("{:x}", policy.finalize()),
    })
}

impl Supervisor {
    pub(super) async fn start_default_bound(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: PathBuf,
    ) -> Result<serde_json::Value> {
        ensure!(unsafe { libc::geteuid() } == 0, "root supervisor required");
        ensure!(
            !command_id.is_nil()
                && !session_id.is_nil()
                && workspace.is_absolute()
                && config_path.is_absolute(),
            "invalid configured system creation"
        );
        let lock = self.bound_creation_lock(session_id).await?;
        let _guard = lock.lock().await;
        let command = VesselCommand::StartConfigured {
            command_id,
            session_id,
            workspace: workspace.clone(),
            config_path: config_path.clone(),
        };
        ensure!(
            !super::start::resolution_record(
                &self.directory,
                "intent",
                command_id,
                &command,
                false,
            )
            .await?,
            "start command was fenced as not admitted"
        );
        // Reuse the binding from exact durable admission, even after the default
        // file changes. Never regenerate an incarnation or relaunch a receipt.
        let selected =
            if super::registry::command_record(&self.directory, command_id, &command, false).await?
            {
                database::execution_binding(&self.directory, session_id)
                    .await?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "bound creation outcome unconfirmed; inspect retained admission"
                        )
                        .context(super::routing::OutcomeUnknown)
                    })?
            } else {
                let identity = protected_default(&self.directory)?;
                database::store_identity(&self.directory, &identity).await?;
                let selected =
                    database::configured_identity(&self.directory, &identity.identity).await?;
                ensure!(
                    selected == identity && protected_default(&self.directory)? == identity,
                    "default execution identity changed"
                );
                binding(&identity, session_id)?
            };
        self.start_bound_configured_locked(command_id, session_id, workspace, config_path, selected)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voyage_protocol::execution_identity::{AccountContextRef, IdentityRef};

    fn ordinary() -> ConfiguredExecutionIdentity {
        ConfiguredExecutionIdentity {
            identity: IdentityRef {
                id: Uuid::new_v4(),
                revision: 1.try_into().unwrap(),
            },
            label: "Explicit account".into(),
            user_name: "fixture".into(),
            uid: 1000,
            gid: 1000,
            supplementary_groups: vec![],
            home: "/home/fixture".into(),
            account_context: AccountContextRef {
                id: Uuid::new_v4(),
                revision: 1.try_into().unwrap(),
            },
            authority: AuthorityClass::Ordinary,
            enabled: true,
        }
    }

    #[test]
    fn default_cannot_select_administrator_or_root_groups() {
        for change in 0..5 {
            let mut identity = ordinary();
            match change {
                0 => identity.authority = AuthorityClass::Administrator,
                1 => identity.uid = 0,
                2 => identity.gid = 0,
                3 => identity.supplementary_groups.push(0),
                _ => identity.enabled = false,
            }
            assert!(
                validate_default(&identity)
                    .unwrap_err()
                    .to_string()
                    .contains("enabled ordinary")
            );
        }
    }

    #[test]
    fn ordinary_binding_has_separate_account_namespace_and_no_admin_grant() {
        let identity = ordinary();
        let session = Uuid::new_v4();
        let selected = binding(&identity, session).unwrap();
        assert_eq!(selected.identity, identity.identity);
        assert_eq!(selected.account_context, identity.account_context);
        assert_eq!(selected.peer_uids.runtime, 1000);
        assert_eq!(selected.peer_uids.supervisor, 0);
        assert!(selected.administrator_grant_id.is_none());
        assert_eq!(selected.session_id, session);
        assert!(!selected.incarnation.is_nil());
        let mut changed = identity.clone();
        changed.account_context.id = Uuid::new_v4();
        let other = binding(&changed, session).unwrap();
        assert_ne!(selected.host_identity_digest, other.host_identity_digest);
        assert_ne!(selected.policy_digest, other.policy_digest);
    }

    #[tokio::test]
    async fn ordinary_supervisor_cannot_admit_default_bound_start() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let fixture = super::super::test_support::Fixture::new();
        let supervisor = fixture.supervisor().await;
        let session = Uuid::new_v4();
        let error = supervisor
            .start_default_bound(
                Uuid::new_v4(),
                session,
                fixture.0.clone(),
                fixture.0.join("config.toml"),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("root supervisor required"));
        assert!(database::registration(&fixture.0, session).await.is_err());
    }
}
