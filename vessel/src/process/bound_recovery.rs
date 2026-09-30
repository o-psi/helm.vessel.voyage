//! Offline journal bookkeeping is separate from execution grant admission.
use super::{accounts::Scope, database, service::Supervisor};
use anyhow::{Context, Result, ensure};
use std::{process::Stdio, time::Duration};
use voyage_protocol::{
    execution_identity::{AuthorityClass, ConfiguredExecutionIdentity},
    identity_helper::*,
    process::*,
};

fn operator_changes(request: &BoundRecoveryRequest) -> bool {
    request.acknowledge_cleanup.is_some()
        || !request.acknowledge_resources.is_empty()
        || request.reconcile_tools.is_some()
}

fn validate_receipt(value: &serde_json::Value, request: &BoundRecoveryRequest) -> Result<()> {
    ensure!(
        value["session_id"] == request.session_id.to_string()
            && value["incarnation"] == request.incarnation.to_string()
            && value["command_id"] == request.command_id.to_string()
            && value["revision"].is_u64()
            && matches!(
                value["cleanup_disposition"].as_str(),
                Some("observed" | "operator_attested" | "unresolved_retained")
            )
            && value["restart_permitted"] == false
            && value["execution_authorized"] == false
            && value["local_process_retired"] == true,
        "offline recovery receipt mismatch"
    );
    ensure!(
        serde_json::to_vec(value)?.len() <= 32768
            && value
                .as_object()
                .is_some_and(|object| object.keys().all(|key| matches!(
                    key.as_str(),
                    "session_id"
                        | "incarnation"
                        | "command_id"
                        | "revision"
                        | "cleanup_disposition"
                        | "restart_permitted"
                        | "execution_authorized"
                        | "retained_run_ids"
                        | "retained_resource_ids"
                        | "last_run_id"
                        | "local_process_retired"
                ))),
        "unexpected offline metadata fields"
    );
    Ok(())
}
impl Supervisor {
    async fn recovery_authority(
        &self,
        scope: &Scope,
        registration: &ProcessRegistration,
        identity: &ConfiguredExecutionIdentity,
        request: &BoundRecoveryRequest,
    ) -> Result<()> {
        match scope {
            Scope::Session(_) => anyhow::bail!("scoped runtime cannot attest offline recovery"),
            Scope::Connection(grant) => {
                super::access::store::current_connection(&self.directory, grant)?;
                ensure!(
                    grant.full_access
                        && grant.rights.contains(&ProcessRight::History)
                        && grant.rights.contains(&ProcessRight::Lifecycle),
                    "offline recovery requires explicit current owner rights"
                );
                if identity.authority == AuthorityClass::Administrator {
                    database::execution_reviews::authority(&self.directory, grant).await?;
                }
            }
            Scope::Owner => {
                if identity.authority == AuthorityClass::Administrator {
                    ensure!(
                        !operator_changes(request),
                        "administrator attestations require enrolled current owner authority"
                    );
                }
            }
        }
        ensure!(
            database::bound_cleanup_identity(&self.directory, registration).await? == *identity,
            "offline identity changed"
        );
        super::launch::validate_identity(identity)?;
        ensure!(
            super::guardian::cleanup_observed(
                &self.directory,
                registration.session_id,
                registration.incarnation
            )
            .unwrap_or(false)
                || super::migration::dormant(
                    &self.directory,
                    registration.session_id,
                    registration.incarnation
                )
                .unwrap_or(false)
                || super::execution_transition::dormant(
                    &self.directory,
                    registration.session_id,
                    registration.incarnation
                )
                .unwrap_or(false),
            "offline recovery requires protected positive retirement"
        );
        Ok(())
    }
    pub(super) async fn recover_bound(
        &self,
        command: VesselCommand,
        scope: Scope,
    ) -> Result<serde_json::Value> {
        let VesselCommand::Recover {
            command_id,
            session_id,
            incarnation,
            acknowledge_cleanup,
            acknowledge_resources,
            reconcile_tools,
            expected_revision,
        } = &command
        else {
            anyhow::bail!("not bound recovery")
        };
        ensure!(
            !command_id.is_nil()
                && acknowledge_resources.len() <= 256
                && (reconcile_tools.is_none() || expected_revision.is_some())
                && acknowledge_resources.iter().all(|id| !id.is_nil())
                && acknowledge_cleanup.is_none_or(|id| !id.is_nil())
                && reconcile_tools.is_none_or(|id| !id.is_nil()),
            "invalid bound recovery intent"
        );
        let lock = self.lifecycle_lock(*session_id).await?;
        let _guard = lock.lock().await;
        let registration = self.registration(*session_id).await?;
        ensure!(
            registration.incarnation == *incarnation
                && registration.state != ProcessState::Relinquished
                && registration.peer_uids.is_some(),
            "stale or relinquished bound recovery"
        );
        let identity = database::bound_cleanup_identity(&self.directory, &registration).await?;
        let directory = super::runtime_storage::bound_directory(
            &self.directory,
            *session_id,
            identity.uid,
            identity.gid,
        )?;
        let request = BoundRecoveryRequest {
            directory,
            session_id: *session_id,
            incarnation: *incarnation,
            command_id: *command_id,
            local_process_retired: true,
            current_scope_cleanup_observed: super::guardian::cleanup_observed(
                &self.directory,
                *session_id,
                *incarnation,
            )
            .unwrap_or(false),
            actor: scope.actor(&registration.workspace),
            acknowledge_cleanup: *acknowledge_cleanup,
            acknowledge_resources: acknowledge_resources.clone(),
            reconcile_tools: *reconcile_tools,
            expected_revision: *expected_revision,
        };
        self.recovery_authority(&scope, &registration, &identity, &request)
            .await?;
        let control = voyage_storage::protected_linux::RootDirectory::open(&self.directory)?
            .create_child("bound-recoveries".as_ref())?;
        let name = format!("{command_id}.json");
        let result_name = format!("{command_id}-result.json");
        let authority = match &scope {
            Scope::Owner => None,
            Scope::Connection(grant) => Some(GrantBinding {
                grant_id: grant.grant_id,
                principal_id: grant.principal_id,
                revision: grant.revision,
            }),
            Scope::Session(_) => unreachable!(),
        };
        let intent = serde_json::to_value(
            serde_json::json!({"command":command,"authority":authority,"request":request,"identity":identity.identity,"account_context":identity.account_context}),
        )?;
        match control.read(result_name.as_ref(), 65_536) {
            Ok(bytes) => {
                let prior: serde_json::Value = serde_json::from_slice(&bytes)?;
                ensure!(
                    prior["intent"] == intent,
                    "bound recovery receipt authority changed"
                );
                validate_receipt(&prior["result"], &request)?;
                return Ok(prior["result"].clone());
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
        if let Ok(bytes) = control.read(name.as_ref(), 65_536) {
            let prior: serde_json::Value = serde_json::from_slice(&bytes)?;
            ensure!(
                prior["intent"] == intent,
                "bound recovery authority or command changed"
            );
            if let Some(result) = prior.get("result") {
                validate_receipt(result, &request)?;
                return Ok(result.clone());
            }
        } else {
            control.publish_new(
                name.as_ref(),
                &serde_json::to_vec(&serde_json::json!({"intent":intent}))?,
                65_536,
            )?;
        }
        super::registry::command_record(&self.directory, *command_id, &command, true).await?;
        let mut process = tokio::process::Command::new(&self.binary);
        super::launch::protected_binary(&self.binary)?;
        process
            .arg("identity-helper")
            .current_dir(&identity.home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // This clone is used only to enter the original OS realm for offline
        // metadata mutation. It cannot enable a disabled catalogue identity.
        let mut cleanup_identity = identity.clone();
        cleanup_identity.enabled = true;
        super::launch::configure_identity(process.as_std_mut(), &cleanup_identity)?;
        if identity.authority == AuthorityClass::Administrator {
            let (data, config) = super::admin_execution::runtime_namespace(
                &self.directory,
                &registration,
                &identity,
            )?
            .context("reviewed administrator namespace unavailable")?;
            process
                .env("XDG_DATA_HOME", data)
                .env("XDG_CONFIG_HOME", config);
        }
        let (authority, descriptor) = super::identity_authority::attach(&mut process)?;
        let mut authority = Some(authority);
        unsafe {
            process.pre_exec(|| {
                for (resource, limit) in
                    [(libc::RLIMIT_AS, 512 * 1024 * 1024), (libc::RLIMIT_CPU, 5)]
                {
                    let value = libc::rlimit {
                        rlim_cur: limit,
                        rlim_max: limit,
                    };
                    if libc::setrlimit(resource, &value) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = process.spawn()?;
        drop(descriptor);
        let result=tokio::time::timeout(Duration::from_secs(20),async{
            let frame=IdentityHelperRequest{schema:IDENTITY_HELPER_SCHEMA,workspace:registration.workspace.clone(),operation:IdentityHelperOperation::RecoverBound{request:request.clone()}};
            write_frame(&mut child.stdin.take().context("private recovery input missing")?,&frame).await?;
            let mut output=child.stdout.take().context("private recovery output missing")?;
            let read=read_frame::<IdentityHelperResponse>(&mut output);tokio::pin!(read);
            let reply=loop{tokio::select!{response=&mut read=>break response?,check=async{if let Some(stream)=&mut authority{super::identity_authority::read(stream).await}else{std::future::pending().await}}=>{
                match check {
                    Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|error|error.kind()==std::io::ErrorKind::UnexpectedEof)=>authority=None,
                    value=>{
                        let allowed=value.is_ok_and(|check|matches!(check.right,IdentityAuthorityRight::Recover)&&check.actor==request.actor&&check.connection_id==request.command_id&&check.account_id.is_none())&&self.recovery_authority(&scope,&registration,&identity,&request).await.is_ok();
                        super::identity_authority::reply(authority.as_mut().context("offline authority stream missing")?,allowed).await?;
                        ensure!(allowed,"current offline authority refused");
                    }
                }
            },_ =tokio::time::sleep(Duration::from_millis(100))=>self.recovery_authority(&scope,&registration,&identity,&request).await?,}};
            ensure!(child.wait().await?.success(),"private offline recovery unavailable");
            self.recovery_authority(&scope,&registration,&identity,&request).await?;
            let IdentityHelperResponse::Value{value}=reply else{anyhow::bail!("private offline recovery refused")};
            validate_receipt(&value, &request)?;
            Ok::<_,anyhow::Error>(value)
        }).await;
        let value = match result {
            Ok(Ok(value)) => value,
            _ => {
                let _ = child.kill().await;
                anyhow::bail!(
                    anyhow::anyhow!("bound recovery outcome unconfirmed; inspect exact receipt")
                        .context(super::routing::OutcomeUnknown)
                );
            }
        };
        // Protected metadata receipt never broadens the current launch grant.
        control.publish_new(
            result_name.as_ref(),
            &serde_json::to_vec(&serde_json::json!({"intent":intent,"result":value}))?,
            65_536,
        )?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_metadata_retains_unknown_obligations_without_permission_claims() {
        let request = BoundRecoveryRequest {
            directory: "/synthetic/private-runtime".into(),
            session_id: uuid::Uuid::new_v4(),
            incarnation: uuid::Uuid::new_v4(),
            command_id: uuid::Uuid::new_v4(),
            local_process_retired: true,
            current_scope_cleanup_observed: false,
            actor: voyage_protocol::accounts::EnrollmentActor {
                principal: "synthetic owner".into(),
                workspace: "/synthetic/deleted-project".into(),
            },
            acknowledge_cleanup: None,
            acknowledge_resources: vec![],
            reconcile_tools: None,
            expected_revision: None,
        };
        let receipt = serde_json::json!({
            "session_id":request.session_id,"incarnation":request.incarnation,
            "command_id":request.command_id,"revision":1,
            "cleanup_disposition":"unresolved_retained","local_process_retired":true,
            "restart_permitted":false,"execution_authorized":false,
            "retained_run_ids":[uuid::Uuid::new_v4()],
            "retained_resource_ids":[uuid::Uuid::new_v4()],"last_run_id":null
        });
        validate_receipt(&receipt, &request).unwrap();
        for key in ["restart_permitted", "execution_authorized"] {
            let mut altered = receipt.clone();
            altered[key] = true.into();
            assert!(validate_receipt(&altered, &request).is_err());
        }
        let mut altered = receipt.clone();
        altered["local_process_retired"] = false.into();
        assert!(validate_receipt(&altered, &request).is_err());
        let mut altered = receipt.clone();
        altered["command_id"] = uuid::Uuid::new_v4().to_string().into();
        assert!(validate_receipt(&altered, &request).is_err());
        let mut altered = receipt;
        altered["configuration"] = "private diagnostic".into();
        assert!(validate_receipt(&altered, &request).is_err());
    }
}
