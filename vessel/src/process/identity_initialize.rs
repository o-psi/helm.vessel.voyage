//! Ordinary initialization owns intent before copying/capturing and never opens
//! an ordinary journal or credential file in the root supervisor.
use super::{
    accounts::Scope,
    database,
    identity_accounts::{self, Selection},
    service::Supervisor,
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use uuid::Uuid;
use voyage_protocol::{
    execution_identity::AuthorityClass,
    identity_helper::{IdentityHelperOperation, IdentityHelperResponse},
    process::*,
};

impl Supervisor {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn start_ordinary_initialized(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        base: Option<PathBuf>,
        initialize: Option<RuntimeInitialization>,
        command: VesselCommand,
        scope: Scope,
    ) -> Result<serde_json::Value> {
        ensure!(
            !command_id.is_nil() && !session_id.is_nil(),
            "invalid initialized creation identity"
        );
        let workspace = std::fs::canonicalize(workspace)?;
        ensure!(
            workspace.is_dir(),
            "initialized workspace must be a directory"
        );
        let right = if matches!(
            (&scope, &initialize),
            (
                Scope::Session(_),
                Some(RuntimeInitialization::Participant { .. })
            )
        ) {
            ProcessRight::Execute
        } else if matches!(&scope, Scope::Session(_)) {
            ProcessRight::Lifecycle
        } else {
            ProcessRight::Create
        };
        scope.check(&self.directory, &workspace, right)?;
        ensure!(
            !matches!(
                (&initialize, &scope),
                (
                    Some(RuntimeInitialization::Participant { .. }),
                    Scope::Owner
                )
            ),
            "participant initialization requires its source grant"
        );
        let directory =
            super::runtime_storage::planned_bound_directory(&self.directory, session_id)?;
        let config_path = directory.join("launch.json");
        let lock = self.bound_creation_lock(session_id).await?;
        let _guard = lock.lock().await;
        ensure!(
            !super::start::resolution_record(
                &self.directory,
                "intent",
                command_id,
                &command,
                false
            )
            .await?,
            "initialized creation was fenced as not admitted"
        );
        if super::registry::command_record(&self.directory, command_id, &command, false).await? {
            let binding = database::execution_binding(&self.directory, session_id)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("initialized creation binding unconfirmed")
                        .context(super::routing::OutcomeUnknown)
                })?;
            let previous = database::registration(&self.directory, session_id).await?;
            return self
                .start_bound_initialized_request_locked(
                    command_id,
                    session_id,
                    workspace,
                    config_path,
                    binding,
                    previous.initialize,
                    command,
                )
                .await;
        }
        ensure!(
            !self.registrations.lock().await?.contains_key(&session_id),
            "initialized voyage already reserved"
        );
        let source = if let Some(RuntimeInitialization::Branch {
            source_session_id, ..
        }) = &initialize
        {
            Some(database::registration(&self.directory, *source_session_id).await?)
        } else {
            None
        };
        let identity = if let Some(source) = &source {
            let identity = database::bound_observer_identity(&self.directory, source).await?;
            ensure!(
                identity.authority == AuthorityClass::Ordinary && identity.uid != 0,
                "branch cannot inherit administrator execution identity"
            );
            identity
        } else {
            let identity = super::default_execution::protected_default(&self.directory)?;
            database::store_identity(&self.directory, &identity).await?;
            ensure!(
                database::configured_identity(&self.directory, &identity.identity).await?
                    == identity,
                "ordinary initialization identity changed"
            );
            identity
        };
        ensure!(
            identity.enabled
                && identity.uid != 0
                && identity.gid != 0
                && identity.authority == AuthorityClass::Ordinary
                && !identity.supplementary_groups.contains(&0),
            "initialization requires independently validated ordinary identity"
        );
        super::launch::validate_identity(&identity)?;
        let intent = serde_json::to_vec(
            &serde_json::json!({"schema":1,"command":command,"initialize":initialize,"identity":identity.identity,"account_context":identity.account_context,"actor":scope.actor(&workspace),"base_config_path":base}),
        )?;
        ensure!(
            intent.len() <= 65_536,
            "initialized creation intent exceeds bounds"
        );
        let control = voyage_storage::protected_linux::RootDirectory::open(&self.directory)?
            .create_child("initialization-launch".as_ref())?;
        let entry = format!("{command_id}.json");
        match control.read(entry.as_ref(), 65_536) {
            Ok(prior) => ensure!(prior == intent, "initialized creation intent changed"),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                ensure!(
                    std::fs::read_dir(self.directory.join("initialization-launch"))?
                        .take(4097)
                        .count()
                        < 4096,
                    "initialized creation retention capacity reached"
                );
                control.publish_new(entry.as_ref(), &intent, 65_536)?;
            }
            Err(error) => return Err(error),
        }
        let request_digest = format!("{:x}", Sha256::digest(&intent));
        let directory = super::runtime_storage::provision_bound_directory(
            &self.directory,
            session_id,
            identity.uid,
            identity.gid,
        )?;
        let mut initialize = initialize;
        if let Some(RuntimeInitialization::Transfer {
            artifact_path,
            sha256,
            ..
        }) = &mut initialize
        {
            *artifact_path = super::runtime_storage::stage_initialization_artifact(
                &self.directory,
                session_id,
                identity.uid,
                identity.gid,
                artifact_path,
                sha256,
            )
            .map_err(|error| error.context(super::routing::OutcomeUnknown))?;
        }
        let projection = self.identity_account_scope(&scope, &workspace)?;
        let operation = match &initialize {
            Some(initialize) => IdentityHelperOperation::CaptureInitialization {
                scope: projection,
                directory,
                request_digest,
                base_config_path: base,
                initialize: initialize.clone(),
            },
            None => IdentityHelperOperation::CaptureLaunch {
                scope: projection,
                directory,
                request_digest,
                base_config_path: base,
                account: None,
                settings: Default::default(),
            },
        };
        let selection = source.as_ref().map_or(
            Selection::InitializationDefault(&identity),
            Selection::Bound,
        );
        let captured = identity_accounts::run_owned_helper(
            &self.directory,
            &self.binary,
            &scope,
            &workspace,
            right,
            operation,
            selection,
        )
        .await
        .map_err(|error| error.context(super::routing::OutcomeUnknown))?;
        let IdentityHelperResponse::Value { value } = captured else {
            anyhow::bail!(
                anyhow::anyhow!("initialized configuration unavailable")
                    .context(super::routing::OutcomeUnknown)
            );
        };
        let path: PathBuf = serde_json::from_value(value["config_path"].clone())?;
        let digest = value["config_digest"]
            .as_str()
            .context("initialized config digest missing")?;
        ensure!(path == config_path, "initialized launch path changed");
        super::identity_start::pin_launch(&self.directory, command_id, session_id, &path, digest)?;
        scope.check(&self.directory, &workspace, right)?;
        if let Some(source) = &source {
            ensure!(
                database::bound_observer_identity(&self.directory, source).await? == identity,
                "branch source identity changed before admission"
            );
        } else {
            ensure!(
                super::default_execution::protected_default(&self.directory)? == identity,
                "ordinary initialization destination changed before admission"
            );
        }
        let binding = super::default_execution::binding(&identity, session_id)?;
        self.start_bound_initialized_request_locked(
            command_id, session_id, workspace, path, binding, initialize, command,
        )
        .await
    }
}
