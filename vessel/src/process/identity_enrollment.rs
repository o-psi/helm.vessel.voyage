//! The supervisor retains one bounded owner of each original enrollment attempt.
//! Provider state and private codes remain in the selected ordinary identity.
use super::{
    accounts::Scope,
    identity_accounts::{Selection, projection, run_owned_helper, selected},
    service::Supervisor,
};
use anyhow::{Result, ensure};
use std::{path::Path, time::Duration};
use tokio::sync::watch;
use uuid::Uuid;
use voyage_protocol::{
    accounts::{EnrollmentRequest, EnrollmentState, EnrollmentStatus},
    identity_helper::*,
    process::ProcessRight,
};

pub(super) type Started = Option<std::result::Result<EnrollmentStatus, String>>;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedIntent {
    schema: u32,
    request: EnrollmentRequest,
    identity: voyage_protocol::execution_identity::IdentityRef,
    account_context: voyage_protocol::execution_identity::AccountContextRef,
}

fn current_actor_scope(root: &Path, request: &EnrollmentRequest) -> Result<Scope> {
    if request.actor.principal == "owner" {
        return Ok(Scope::Owner);
    }
    let parts: Vec<_> = request.actor.principal.split(':').collect();
    ensure!(
        parts.len() == 4 && parts[0] == "grant",
        "retained actor unavailable"
    );
    let id = Uuid::parse_str(parts[1])?;
    let principal = Uuid::parse_str(parts[2])?;
    let revision = parts[3].parse::<u64>()?;
    let path = super::access::store::connection_path(root, id);
    if path.try_exists()? {
        let grant: voyage_protocol::process::ConnectionGrant = super::access::store::load(&path)?;
        ensure!(
            grant.principal_id == principal && grant.revision == revision,
            "retained actor changed"
        );
        Ok(Scope::Connection(grant))
    } else {
        let grant: voyage_protocol::process::ProcessGrant =
            super::access::store::load(&super::access::store::grant_path(root, id))?;
        ensure!(
            grant.principal_id == principal && grant.revision == revision,
            "retained actor changed"
        );
        Ok(Scope::Session(grant))
    }
}

async fn operation(
    root: &Path,
    binary: &Path,
    scope: &Scope,
    workspace: &Path,
    identity: &voyage_protocol::execution_identity::ConfiguredExecutionIdentity,
    request: IdentityEnrollmentOperation,
) -> Result<serde_json::Value> {
    let request = IdentityHelperOperation::Enrollment {
        scope: projection(scope, root, workspace)?,
        operation: request,
    };
    match run_owned_helper(
        root,
        binary,
        scope,
        workspace,
        ProcessRight::AccountEnroll,
        request,
        Selection::FrozenDefault(identity),
    )
    .await?
    {
        IdentityHelperResponse::Value { value } => Ok(value),
        _ => anyhow::bail!("identity enrollment unavailable"),
    }
}

impl Supervisor {
    pub(super) async fn resume_identity_enrollments(&self) -> Result<()> {
        let root = voyage_storage::protected_linux::RootDirectory::open(&self.directory)?;
        let directory = match root.child("identity-enrollments".as_ref()) {
            Ok(directory) => directory,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        for (count, entry) in
            std::fs::read_dir(self.directory.join("identity-enrollments"))?.enumerate()
        {
            ensure!(count < 4096, "retained enrollment directory exceeds bound");
            let entry = entry?;
            let name = entry.file_name();
            if !name.as_encoded_bytes().ends_with(b".json") {
                continue;
            }
            let intent: RetainedIntent = serde_json::from_slice(&directory.read(&name, 4096)?)?;
            ensure!(
                intent.schema == 1 && !intent.request.enrollment_id.is_nil(),
                "retained enrollment identity invalid"
            );
            let resumed = async {
                let scope = current_actor_scope(&self.directory, &intent.request)?;
                let workspace = std::path::PathBuf::from(&intent.request.actor.workspace);
                let identity = selected(&scope, &self.directory).await?;
                ensure!(
                    identity.identity == intent.identity
                        && identity.account_context == intent.account_context,
                    "retained enrollment execution identity changed"
                );
                // Resolve is observation/negative admission, never an automatic
                // repeat of device-start or an uncertain exchange effect.
                self.track_identity_enrollment(scope, workspace, intent.request, true)
                    .await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if resumed.is_err() {
                continue;
            }
        }
        Ok(())
    }

    pub(super) async fn start_identity_enrollment(
        &self,
        scope: Scope,
        workspace: std::path::PathBuf,
        request: EnrollmentRequest,
    ) -> Result<serde_json::Value> {
        let mut receiver = self
            .track_identity_enrollment(scope, workspace, request, false)
            .await?;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Some(result) = receiver.borrow().clone() {
                    return result;
                }
                if receiver.changed().await.is_err() {
                    return Err(
                        "identity enrollment outcome unavailable; check the retained attempt"
                            .into(),
                    );
                }
            }
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!("identity enrollment start remains pending; check the retained attempt")
        })?;
        Ok(serde_json::to_value(result.map_err(anyhow::Error::msg)?)?)
    }
    async fn track_identity_enrollment(
        &self,
        scope: Scope,
        workspace: std::path::PathBuf,
        request: EnrollmentRequest,
        resume: bool,
    ) -> Result<watch::Receiver<Started>> {
        ensure!(
            !request.command_id.is_nil()
                && !request.enrollment_id.is_nil()
                && request.command_id != request.enrollment_id,
            "invalid enrollment command identity"
        );
        ensure!(
            [&request.alias, &request.label]
                .iter()
                .all(|text| !text.trim().is_empty()
                    && text.len() <= 256
                    && !text.chars().any(char::is_control)),
            "invalid enrollment display text"
        );
        scope.check(&self.directory, &workspace, ProcessRight::AccountEnroll)?;
        ensure!(
            request.actor == scope.actor(&workspace)
                && scope.connection_allowed(request.connection_id),
            "enrollment scope refused"
        );
        let identity = selected(&scope, &self.directory).await?;
        let control = voyage_storage::protected_linux::RootDirectory::open(&self.directory)?
            .create_child("identity-enrollments".as_ref())?;
        let name = format!("{}.json", request.enrollment_id);
        let bytes = serde_json::to_vec(
            &serde_json::json!({"schema":1,"request":request,"identity":identity.identity,"account_context":identity.account_context}),
        )?;
        ensure!(bytes.len() <= 4096, "enrollment intent exceeds bounds");
        match control.read(name.as_ref(), 4096) {
            Ok(prior) => ensure!(prior == bytes, "identity enrollment request changed"),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                ensure!(
                    std::fs::read_dir(self.directory.join("identity-enrollments"))?
                        .take(4097)
                        .count()
                        < 4096,
                    "identity enrollment retention capacity reached"
                );
                control.publish_new(name.as_ref(), &bytes, 4096)?;
            }
            Err(error) => return Err(error),
        }
        let mut workers = self.enrollment_workers.lock().await;
        workers.retain(|_, worker| !worker.is_finished());
        let mut starts = self.identity_enrollment_starts.lock().await;
        starts.retain(|id, _| workers.contains_key(id));
        let receiver = if let Some(receiver) = starts.get(&request.enrollment_id) {
            receiver.clone()
        } else {
            ensure!(
                workers.len() < 64,
                "identity enrollment worker capacity reached"
            );
            let (sender, receiver) = watch::channel(None);
            let root = self.directory.clone();
            let binary = self.binary.clone();
            let id = request.enrollment_id;
            // Retain ownership before the task can start or contact a provider.
            starts.insert(id, receiver.clone());
            workers.insert(id,tokio::spawn(async move {
                let first=if resume {IdentityEnrollmentOperation::Resolve{request}} else {IdentityEnrollmentOperation::Start{request}};
                let started=operation(&root,&binary,&scope,&workspace,&identity,first).await
                    .and_then(|value|Ok(serde_json::from_value::<EnrollmentStatus>(value)?));
                match started {
                    Ok(status)=>{
                        let _=sender.send(Some(Ok(status.clone())));
                        let mut status=status;
                        let deadline=tokio::time::Instant::now()+Duration::from_secs(15*60);
                        while matches!(status.state,EnrollmentState::Starting|EnrollmentState::Pending|EnrollmentState::Exchanging) && tokio::time::Instant::now()<deadline {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            let driven=operation(&root,&binary,&scope,&workspace,&identity,IdentityEnrollmentOperation::Drive{enrollment_id:id}).await;
                            let Ok(value)=driven else{break;};
                            let Ok(next)=serde_json::from_value(value) else{break;};status=next;
                        }
                    },
                    Err(_)=>{let _=sender.send(Some(Err("identity enrollment outcome unavailable; check the retained attempt".into())));},
                }
            }));
            receiver
        };
        drop(starts);
        drop(workers);
        Ok(receiver)
    }
}
