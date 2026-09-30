//! Reviewed same-session handoff: retired source owns journal reads, target owns
//! private configuration commit, root owns only typed receipts and metadata FDs.
use super::{database, service::Supervisor};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    num::NonZeroU64,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use uuid::Uuid;
use voyage_protocol::{
    execution_identity::*, execution_review_control::*, execution_transition::*,
    identity_helper::IdentityConfigFacts, process::*,
};
use voyage_storage::protected_linux::RootDirectory;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    schema: u32,
    review_id: Uuid,
    command_id: Uuid,
    principal: Uuid,
    source: ProcessRegistration,
    source_identity: ConfiguredExecutionIdentity,
    target: ConfiguredExecutionIdentity,
    target_config: PathBuf,
    target_facts: IdentityConfigFacts,
    target_incarnation: Uuid,
}
fn directory(root: &Path) -> Result<RootDirectory> {
    RootDirectory::open(root)?.create_child("execution-transitions".as_ref())
}
fn write<T: Serialize>(root: &Path, id: Uuid, phase: &str, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let name = format!("{id}-{phase}.json");
    let directory = directory(root)?;
    match directory.read(name.as_ref(), 65536) {
        Ok(previous) => ensure!(previous == bytes, "execution transition receipt conflict"),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            directory.publish_new(name.as_ref(), &bytes, 65536)?
        }
        Err(error) => return Err(error),
    };
    Ok(())
}
fn read<T: serde::de::DeserializeOwned>(root: &Path, id: Uuid, phase: &str) -> Result<T> {
    Ok(serde_json::from_slice(
        &directory(root)?.read(format!("{id}-{phase}.json").as_ref(), 65536)?,
    )?)
}
fn digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn phase_command(review: Uuid, phase: &str) -> Uuid {
    let hash = Sha256::digest(
        [
            b"voyage/execution-transition-phase/v1".as_slice(),
            review.as_bytes(),
            phase.as_bytes(),
        ]
        .concat(),
    );
    Uuid::from_bytes(hash[..16].try_into().expect("SHA256 prefix"))
}
pub(super) fn retained_digest(
    root: &Path,
    registration: &ProcessRegistration,
) -> Result<Option<String>> {
    let result = read::<(Uuid, String)>(root, registration.incarnation, "retained-config");
    match result {
        Ok((session, digest)) => {
            ensure!(
                session == registration.session_id
                    && digest.len() == 64
                    && digest.bytes().all(|b| b.is_ascii_hexdigit()),
                "retained configuration pin changed"
            );
            Ok(Some(digest))
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
async fn helper(
    binary: &Path,
    identity: &ConfiguredExecutionIdentity,
    namespace: Option<(PathBuf, PathBuf)>,
    request: TransitionRequest,
    monitor: Option<(
        &Supervisor,
        &ConnectionGrant,
        &Intent,
        &SavedExecutionReview,
    )>,
) -> Result<TransitionResponse> {
    super::launch::protected_binary(binary)?;
    ensure!(
        serde_json::to_vec(&request)?.len() <= MAX_REQUEST,
        "transition request exceeds bounds"
    );
    let mut command = tokio::process::Command::new(binary);
    command
        .arg("transition-helper")
        .current_dir(&identity.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    super::launch::configure_identity(command.as_std_mut(), identity)?;
    if let Some((data, config)) = namespace {
        command
            .env("XDG_DATA_HOME", data)
            .env("XDG_CONFIG_HOME", config);
    }
    let mut child = command.spawn()?;
    let result:std::result::Result<anyhow::Result<TransitionResponse>,tokio::time::error::Elapsed>=tokio::time::timeout(Duration::from_secs(10),async{
        let exchange=async{
            write_frame(&mut child.stdin.take().context("transition input unavailable")?,&request).await?;
            let reply:TransitionResponse=read_frame(&mut child.stdout.take().context("transition output unavailable")?).await?;
            ensure!(serde_json::to_vec(&reply)?.len()<=MAX_RESPONSE&&child.wait().await?.success(),"transition observation unavailable");Ok(reply)
        };
        tokio::pin!(exchange);
        loop{tokio::select!{reply=&mut exchange=>break reply,_=tokio::time::sleep(Duration::from_millis(100)),if monitor.is_some()=>{let (supervisor,grant,intent,review)=monitor.expect("guarded private helper");supervisor.transition_authority(grant,intent,review).await?;}}}
    }).await;
    if let Ok(Ok(reply)) = result {
        return Ok(reply);
    }
    let _ = child.kill().await;
    anyhow::bail!("transition observation unavailable")
}
impl Supervisor {
    async fn transition_target(
        &self,
        grant: &ConnectionGrant,
        session: Uuid,
        command: Uuid,
        workspace: &Path,
        reference: &IdentityRef,
    ) -> Result<(ConfiguredExecutionIdentity, PathBuf, IdentityConfigFacts)> {
        let ordinary = super::default_execution::protected_default(&self.directory)?;
        if ordinary.identity == *reference {
            database::store_identity(&self.directory, &ordinary).await?;
            let (path, facts) = self
                .prepare_identity_config_for_review(grant, session, command, workspace, &ordinary)
                .await?;
            return Ok((ordinary, path, facts));
        }
        let record = super::admin_execution::provision(&self.directory)?;
        ensure!(
            record.identity.identity == *reference,
            "target identity was not configured"
        );
        database::store_identity(&self.directory, &record.identity).await?;
        let facts = super::admin_execution::facts(&self.binary, &record, workspace).await?;
        Ok((record.identity, record.config_path, facts))
    }
    async fn transition_facts(
        &self,
        grant: &ConnectionGrant,
        intent: &Intent,
        source: &RetiredJournalFacts,
    ) -> Result<ReviewFacts> {
        let authority = database::execution_reviews::authority(&self.directory, grant).await?;
        ensure!(
            source.session_id == intent.source.session_id
                && source.source_incarnation == intent.source.incarnation,
            "source journal identity changed"
        );
        let (target, path, current) = self
            .transition_target(
                grant,
                intent.source.session_id,
                intent.command_id,
                &intent.source.workspace,
                &intent.target.identity,
            )
            .await?;
        ensure!(
            target == intent.target
                && path == intent.target_config
                && current == intent.target_facts,
            "target account/configuration changed during review"
        );
        let (host, policy) = if target.authority == AuthorityClass::Administrator {
            let (_, facts, _) = self
                .execution_facts(
                    grant,
                    intent.review_id,
                    intent.command_id,
                    intent.source.session_id,
                    intent.target_incarnation,
                    &intent.source.workspace,
                    &target.identity,
                )
                .await?;
            (facts.host_identity_digest, facts.policy_digest)
        } else {
            (
                digest(&(&target, &current.account_root_digest))?,
                digest(&(&current.policy_digest, &current.config_digest))?,
            )
        };
        Ok(ReviewFacts {
            vessel_id: grant.vessel_id,
            session_id: intent.source.session_id,
            run_id: intent.review_id,
            incarnation: intent.target_incarnation,
            requester_id: grant.principal_id,
            connection_id: grant.grant_id,
            connection_revision: NonZeroU64::new(grant.revision)
                .context("connection revision unavailable")?,
            administrative_owner_id: grant.principal_id,
            authority_revision: NonZeroU64::new(authority)
                .context("administrator authority unavailable")?,
            expected_session_revision: source.revision,
            change: ExecutionChange::Transition,
            previous_incarnation: Some(intent.source.incarnation),
            identity: target.identity,
            account_context: target.account_context,
            account: current.account,
            account_capability_revision: current.capability_revision,
            workspace: intent.source.workspace.clone(),
            host_identity_digest: host,
            policy_digest: policy,
            release_digest: super::launch::protected_binary_digest(&self.binary)?,
            pending_work_digest: digest(source)?,
        })
    }
    // Source generation, target identity and stop intent are separate authority/provenance inputs.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn prepare_execution_transition(
        &self,
        grant: &ConnectionGrant,
        review_id: Uuid,
        command_id: Uuid,
        session: Uuid,
        source_incarnation: Uuid,
        target: IdentityRef,
        stop_source: bool,
    ) -> Result<serde_json::Value> {
        ensure!(
            stop_source && !review_id.is_nil() && !command_id.is_nil(),
            "explicit source-stop consent required for identity review"
        );
        database::execution_reviews::authority(&self.directory, grant).await?;
        let lock = self.bound_creation_lock(session).await?;
        let _guard = lock.lock().await;
        if let Ok(existing) = read::<Intent>(&self.directory, review_id, "intent") {
            ensure!(
                existing.command_id == command_id
                    && existing.principal == grant.principal_id
                    && existing.source.session_id == session
                    && existing.source.incarnation == source_incarnation
                    && existing.target.identity == target,
                "transition preparation receipt conflict"
            );
            return self.observe_execution_transition(grant, review_id).await;
        }
        let source = self.registration(session).await?;
        ensure!(
            source.incarnation == source_incarnation
                && source.peer_uids.is_some()
                && source.initialize.is_none(),
            "source is stale or not an independent bound owner"
        );
        self.execution_connection(grant, &source, true).await?;
        self.connection_session(grant, session, &source.workspace)?;
        let binding = database::execution_binding(&self.directory, session)
            .await?
            .context("source binding unavailable")?;
        let source_identity =
            database::configured_identity(&self.directory, &binding.identity).await?;
        let (target, target_config, target_facts) = self
            .transition_target(grant, session, command_id, &source.workspace, &target)
            .await?;
        let intent = Intent {
            schema: 1,
            review_id,
            command_id,
            principal: grant.principal_id,
            source,
            source_identity,
            target,
            target_config,
            target_facts,
            target_incarnation: Uuid::new_v4(),
        };
        write(&self.directory, review_id, "intent", &intent)?;
        let retired =
            super::guardian::cleanup_observed(&self.directory, session, source_incarnation)
                .unwrap_or(false)
                || super::migration::dormant(&self.directory, session, source_incarnation)
                    .unwrap_or(false);
        if !retired {
            self.bound_stop(&intent.source, None)
                .await
                .map_err(|error| error.context(super::routing::OutcomeUnknown))?;
        }
        ensure!(
            super::guardian::cleanup_observed(&self.directory, session, source_incarnation)
                .unwrap_or(false)
                || super::migration::dormant(&self.directory, session, source_incarnation)
                    .unwrap_or(false),
            "source retirement remains unconfirmed; observe retained transition"
        );
        self.finish_transition_preparation(grant, &intent).await
    }
    async fn finish_transition_preparation(
        &self,
        grant: &ConnectionGrant,
        intent: &Intent,
    ) -> Result<serde_json::Value> {
        database::execution_reviews::authority(&self.directory, grant).await?;
        ensure!(
            self.registration(intent.source.session_id)
                .await?
                .incarnation
                == intent.source.incarnation,
            "source incarnation changed"
        );
        let runtime = super::runtime_storage::bound_directory(
            &self.directory,
            intent.source.session_id,
            intent.source_identity.uid,
            intent.source_identity.gid,
        )?;
        let response = helper(
            &self.binary,
            &intent.source_identity,
            None,
            TransitionRequest {
                schema: SCHEMA,
                session_id: intent.source.session_id,
                source_incarnation: intent.source.incarnation,
                operation: TransitionOperation::Observe { directory: runtime },
            },
            None,
        )
        .await?;
        let TransitionResponse::Facts { facts: source } = response else {
            anyhow::bail!("retired source journal unavailable")
        };
        write(&self.directory, intent.review_id, "source", &source)?;
        let current = self.transition_facts(grant, intent, &source).await?;
        let now = u64::try_from(chrono::Utc::now().timestamp_millis())?;
        let mut review = ExecutionReview {
            schema: EXECUTION_SCHEMA,
            review_id: intent.review_id,
            command_id: intent.command_id,
            created_at_ms: now,
            expires_at_ms: now + MAX_REVIEW_LIFETIME_MS,
            facts: current.clone(),
            digest: String::new(),
        };
        review.digest = review
            .calculated_digest()
            .map_err(|_| anyhow::anyhow!("transition review unavailable"))?;
        // Store the exact review before reserving its database receipt, so a lost
        // response cannot regenerate timestamps/digest for the retained IDs.
        let review = match read::<ExecutionReview>(&self.directory, intent.review_id, "review") {
            Ok(previous) => previous,
            Err(_) => {
                write(&self.directory, intent.review_id, "review", &review)?;
                review
            }
        };
        Ok(serde_json::to_value(
            database::execution_reviews::prepare(&self.directory, grant, &review, &current).await?,
        )?)
    }
    pub(super) async fn observe_execution_transition(
        &self,
        grant: &ConnectionGrant,
        review_id: Uuid,
    ) -> Result<serde_json::Value> {
        database::execution_reviews::authority(&self.directory, grant).await?;
        let intent: Intent = read(&self.directory, review_id, "intent")?;
        ensure!(
            intent.principal == grant.principal_id,
            "transition belongs to another owner"
        );
        if let Ok(saved) =
            database::execution_reviews::resolve(&self.directory, grant, review_id).await
        {
            return Ok(serde_json::to_value(saved)?);
        }
        let retired = super::guardian::cleanup_observed(
            &self.directory,
            intent.source.session_id,
            intent.source.incarnation,
        )
        .unwrap_or(false)
            || super::migration::dormant(
                &self.directory,
                intent.source.session_id,
                intent.source.incarnation,
            )
            .unwrap_or(false);
        if retired {
            return self.finish_transition_preparation(grant, &intent).await;
        }
        Ok(
            serde_json::json!({"preparation":{"review_id":review_id,"command_id":intent.command_id,"session_id":intent.source.session_id,"source_incarnation":intent.source.incarnation,"phase":"unconfirmed_preparation","source_cleanup_observed":false},"message":"Source retirement remains unconfirmed. Observation never repeats the stop or launches the target."}),
        )
    }
    pub(super) async fn approve_execution_transition(
        &self,
        grant: &ConnectionGrant,
        approval: ReviewApproval,
        saved: SavedExecutionReview,
    ) -> Result<serde_json::Value> {
        let intent: Intent = read(&self.directory, approval.review_id, "intent")?;
        ensure!(
            intent.principal == grant.principal_id
                && intent.command_id == approval.command_id
                && saved.review.digest == approval.digest,
            "transition approval identity conflict"
        );
        let lock = self.bound_creation_lock(intent.source.session_id).await?;
        let _guard = lock.lock().await;
        ensure!(
            self.registration(intent.source.session_id)
                .await?
                .incarnation
                == intent.source.incarnation,
            "source incarnation changed"
        );
        ensure!(
            super::guardian::cleanup_observed(
                &self.directory,
                intent.source.session_id,
                intent.source.incarnation
            )
            .unwrap_or(false)
                || super::migration::dormant(
                    &self.directory,
                    intent.source.session_id,
                    intent.source.incarnation
                )
                .unwrap_or(false),
            "source cleanup remains unconfirmed"
        );
        let source: RetiredJournalFacts = read(&self.directory, approval.review_id, "source")?;
        let current = self.transition_facts(grant, &intent, &source).await?;
        let approved =
            database::execution_reviews::approve(&self.directory, grant, &approval, &current)
                .await?;
        if !database::execution_reviews::mark_launching(&self.directory, grant, &approval).await? {
            return Ok(serde_json::to_value(approved)?);
        }
        let transition = self
            .commit_execution_transition(grant, &intent, &source, &approved)
            .await;
        let outcome = match transition {
            Ok(observed) => ExecutionOutcome::Ready { observed },
            Err(_) => ExecutionOutcome::Unconfirmed {
                cleanup_obligations: vec![intent.source.session_id],
            },
        };
        Ok(serde_json::to_value(
            database::execution_reviews::finish_launch(
                &self.directory,
                grant,
                approval.review_id,
                outcome,
            )
            .await?,
        )?)
    }
    async fn transition_authority(
        &self,
        grant: &ConnectionGrant,
        intent: &Intent,
        expected: &SavedExecutionReview,
    ) -> Result<SavedExecutionReview> {
        let revision = database::execution_reviews::authority(&self.directory, grant).await?;
        let saved =
            database::execution_reviews::resolve(&self.directory, grant, intent.review_id).await?;
        ensure!(
            revision == expected.review.facts.authority_revision.get()
                && saved.review == expected.review
                && saved.review.facts.connection_id == grant.grant_id
                && saved.review.facts.connection_revision.get() == grant.revision
                && saved.review.facts.administrative_owner_id == grant.principal_id
                && intent.principal == grant.principal_id
                && matches!(
                    saved.receipt.outcome,
                    ExecutionOutcome::Launching | ExecutionOutcome::Unconfirmed { .. }
                ),
            "reviewed transition authority changed"
        );
        ensure!(
            database::configured_identity(&self.directory, &intent.target.identity).await?
                == intent.target,
            "reviewed target OS identity changed"
        );
        if let Some(id) = saved.administrator_grant_id {
            ensure!(
                database::execution_reviews::grant_facts(&self.directory, id).await?
                    == saved.review.facts,
                "administrator execution grant changed"
            );
        }
        Ok(saved)
    }
    pub(super) async fn reconcile_execution_transition(
        &self,
        grant: &ConnectionGrant,
        review_id: Uuid,
        command_id: Uuid,
        review_digest: &str,
    ) -> Result<serde_json::Value> {
        ensure!(
            !command_id.is_nil(),
            "reconciliation command identity missing"
        );
        database::execution_reviews::authority(&self.directory, grant).await?;
        let intent: Intent = read(&self.directory, review_id, "intent")?;
        let saved = database::execution_reviews::resolve(&self.directory, grant, review_id).await?;
        ensure!(
            intent.principal == grant.principal_id
                && saved.review.digest == review_digest
                && command_id != intent.command_id,
            "reconciliation review conflict"
        );
        ensure!(
            matches!(
                saved.receipt.outcome,
                ExecutionOutcome::Launching | ExecutionOutcome::Unconfirmed { .. }
            ),
            "review has no unresolved transition"
        );
        write(
            &self.directory,
            command_id,
            "reconciliation",
            &(review_id, review_digest, grant.principal_id),
        )?;
        let lock = self.bound_creation_lock(intent.source.session_id).await?;
        let _guard = lock.lock().await;
        let source: RetiredJournalFacts = read(&self.directory, review_id, "source")?;
        self.commit_execution_transition_mode(grant, &intent, &source, &saved, false)
            .await
    }
    async fn commit_execution_transition(
        &self,
        grant: &ConnectionGrant,
        intent: &Intent,
        source: &RetiredJournalFacts,
        approved: &SavedExecutionReview,
    ) -> Result<ObservedExecution> {
        let value = self
            .commit_execution_transition_mode(grant, intent, source, approved, true)
            .await?;
        Ok(serde_json::from_value(value)?)
    }
    async fn commit_execution_transition_mode(
        &self,
        grant: &ConnectionGrant,
        intent: &Intent,
        source: &RetiredJournalFacts,
        approved: &SavedExecutionReview,
        launch: bool,
    ) -> Result<serde_json::Value> {
        let session = intent.source.session_id;
        let old = intent.source.incarnation;
        self.transition_authority(grant, intent, approved).await?;
        ensure!(
            super::guardian::cleanup_observed(&self.directory, session, old).unwrap_or(false)
                || super::migration::dormant(&self.directory, session, old).unwrap_or(false),
            "source retirement remains unconfirmed"
        );
        if let Ok(current) = self.registration(session).await {
            if current.incarnation == intent.target_incarnation {
                let info = super::bound_lifecycle::inspect(&self.directory, &current).await;
                if info.state == ProcessState::Live {
                    let observed = super::admin_execution::observation(
                        &self.directory,
                        session,
                        current.incarnation,
                    )?;
                    return Ok(serde_json::to_value(
                        database::execution_reviews::finish_launch(
                            &self.directory,
                            grant,
                            intent.review_id,
                            ExecutionOutcome::Ready { observed },
                        )
                        .await?,
                    )?);
                }
                return Ok(
                    serde_json::json!({"transition":{"review_id":intent.review_id,"session_id":session,"incarnation":current.incarnation,"process_state":info.state,"phase":"admission_retained"},"message":"Retained admission observed. Reconciliation never repeats guardian launch."}),
                );
            }
            let mut immutable = current.clone();
            immutable.state = intent.source.state.clone();
            ensure!(
                serde_json::to_vec(&immutable)? == serde_json::to_vec(&intent.source)?,
                "source registration changed during reconciliation"
            );
        }
        self.transition_facts(grant, intent, source).await?;
        let runtime = super::runtime_storage::planned_bound_directory(&self.directory, session)?;
        let receipt = match read::<PreparedTransitionReceipt>(
            &self.directory,
            intent.review_id,
            "prepared",
        ) {
            Ok(receipt) => receipt,
            Err(_) => {
                super::runtime_storage::bound_directory(
                    &self.directory,
                    session,
                    intent.source_identity.uid,
                    intent.source_identity.gid,
                )?;
                let prepared = helper(
                    &self.binary,
                    &intent.source_identity,
                    None,
                    TransitionRequest {
                        schema: SCHEMA,
                        session_id: session,
                        source_incarnation: old,
                        operation: TransitionOperation::SourceFreeze {
                            directory: runtime.clone(),
                            command_id: phase_command(intent.review_id, "freeze"),
                            transition_id: intent.review_id,
                            target_incarnation: intent.target_incarnation,
                            expected: source.clone(),
                            target_uid: intent.target.uid,
                            target_gid: intent.target.gid,
                            target_config_digest: intent.target_facts.config_digest.clone(),
                            review_digest: approved.review.digest.clone(),
                        },
                    },
                    Some((self, grant, intent, approved)),
                )
                .await;
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        // A private helper may commit just before its reply or authority
                        // fence. Receipt-only lookup settles metadata; it never replays the
                        // freeze, copies target secrets, or authorizes a process launch.
                        if let Ok(TransitionResponse::Prepared { receipt }) = helper(
                            &self.binary,
                            &intent.source_identity,
                            None,
                            TransitionRequest {
                                schema: SCHEMA,
                                session_id: session,
                                source_incarnation: old,
                                operation: TransitionOperation::Lookup {
                                    directory: runtime.clone(),
                                    command_id: phase_command(intent.review_id, "freeze"),
                                },
                            },
                            None,
                        )
                        .await
                        {
                            ensure!(
                                receipt.review_digest == approved.review.digest
                                    && receipt.transition_id == intent.review_id,
                                "source freeze receipt identity changed"
                            );
                            write(&self.directory, intent.review_id, "prepared", &receipt)?;
                            if self
                                .transition_authority(grant, intent, approved)
                                .await
                                .is_err()
                                && let Ok(TransitionResponse::Aborted { receipt: aborted }) =
                                    helper(
                                        &self.binary,
                                        &intent.source_identity,
                                        None,
                                        TransitionRequest {
                                            schema: SCHEMA,
                                            session_id: session,
                                            source_incarnation: old,
                                            operation: TransitionOperation::AbortSource {
                                                directory: runtime.clone(),
                                                command_id: phase_command(
                                                    intent.review_id,
                                                    "abort",
                                                ),
                                                expected: receipt,
                                            },
                                        },
                                        None,
                                    )
                                    .await
                            {
                                write(&self.directory, intent.review_id, "aborted", &aborted)?;
                            }
                        }
                        return Err(error);
                    }
                };
                let TransitionResponse::Prepared { receipt } = prepared else {
                    anyhow::bail!("source transition remains unconfirmed")
                };
                write(&self.directory, intent.review_id, "prepared", &receipt)?;
                receipt
            }
        };
        let permitted = self
            .transition_authority(grant, intent, approved)
            .await
            .is_ok();
        if !permitted {
            ensure!(
                read::<(u32, u32, PreparedTransitionReceipt)>(
                    &self.directory,
                    intent.review_id,
                    "ownership-intent"
                )
                .is_err(),
                "authority fenced after ownership handoff; retained target requires operator reconciliation"
            );
            // No target ownership, catalogue binding or guardian admission has
            // been published at this point. Abort metadata only; paused/unknown
            // work and the original account configuration remain retained.
            let aborted = helper(
                &self.binary,
                &intent.source_identity,
                None,
                TransitionRequest {
                    schema: SCHEMA,
                    session_id: session,
                    source_incarnation: old,
                    operation: TransitionOperation::AbortSource {
                        directory: runtime.clone(),
                        command_id: phase_command(intent.review_id, "abort"),
                        expected: receipt.clone(),
                    },
                },
                None,
            )
            .await?;
            let TransitionResponse::Aborted { receipt: aborted } = aborted else {
                anyhow::bail!("source abort remains unconfirmed")
            };
            write(&self.directory, intent.review_id, "aborted", &aborted)?;
            anyhow::bail!("transition authority was fenced; source retirement remains retained");
        }
        let inventory = match read::<Vec<(u64, u64, u32, u64)>>(
            &self.directory,
            intent.review_id,
            "ownership-inventory",
        ) {
            Ok(inventory) => inventory,
            Err(_) => {
                let inventory = super::runtime_storage::transfer_inventory(
                    &self.directory,
                    session,
                    intent.source_identity.uid,
                    intent.source_identity.gid,
                )?;
                write(
                    &self.directory,
                    intent.review_id,
                    "ownership-inventory",
                    &inventory,
                )?;
                inventory
            }
        };
        self.transition_authority(grant, intent, approved).await?;
        write(
            &self.directory,
            intent.review_id,
            "ownership-intent",
            &(intent.source_identity.uid, intent.target.uid, &receipt),
        )?;
        if read::<(u32, u32)>(&self.directory, intent.review_id, "ownership-complete").is_err() {
            super::runtime_storage::transfer_bound_directory(
                &self.directory,
                session,
                intent.source_identity.uid,
                intent.source_identity.gid,
                intent.target.uid,
                intent.target.gid,
                &inventory,
            )?;
        }
        write(
            &self.directory,
            intent.review_id,
            "ownership-complete",
            &(intent.target.uid, intent.target.gid),
        )?;
        super::launch::validate_identity(&intent.target)?;
        let namespace = if intent.target.authority == AuthorityClass::Administrator {
            let provision = super::admin_execution::provision(&self.directory)?;
            Some((provision.data_directory, provision.config_directory))
        } else {
            None
        };
        super::runtime_storage::bound_directory(
            &self.directory,
            session,
            intent.target.uid,
            intent.target.gid,
        )?;
        self.transition_authority(grant, intent, approved).await?;
        self.transition_facts(grant, intent, source).await?;
        let committed = helper(
            &self.binary,
            &intent.target,
            namespace,
            TransitionRequest {
                schema: SCHEMA,
                session_id: session,
                source_incarnation: old,
                operation: TransitionOperation::TargetCommit {
                    directory: runtime,
                    command_id: phase_command(intent.review_id, "commit"),
                    expected: receipt,
                    target_config_path: intent.target_config.clone(),
                },
            },
            Some((self, grant, intent, approved)),
        )
        .await?;
        let TransitionResponse::Committed { receipt } = committed else {
            anyhow::bail!("target configuration commit remains unconfirmed")
        };
        write(&self.directory, intent.review_id, "committed", &receipt)?;
        ensure!(
            receipt.config_digest == intent.target_facts.config_digest
                && receipt.history_digest == source.history_digest,
            "target handoff receipt changed"
        );
        self.transition_authority(grant, intent, approved).await?;
        self.transition_facts(grant, intent, source).await?;
        if intent.target.authority == AuthorityClass::Administrator {
            let provision = super::admin_execution::provision(&self.directory)?;
            super::admin_execution::pin_namespace(
                &self.directory,
                session,
                intent.command_id,
                &provision,
            )?;
        }
        super::identity_start::pin_launch(
            &self.directory,
            intent.command_id,
            session,
            &intent.target_config,
            &intent.target_facts.config_digest,
        )?;
        write(
            &self.directory,
            intent.target_incarnation,
            "retained-config",
            &(session, intent.target_facts.config_digest.clone()),
        )?;
        let binding = ExecutionBinding {
            session_id: session,
            incarnation: intent.target_incarnation,
            identity: intent.target.identity.clone(),
            account_context: intent.target.account_context.clone(),
            peer_uids: ProcessPeerUids {
                supervisor: 0,
                runtime: intent.target.uid,
            },
            administrator_grant_id: approved.administrator_grant_id,
            host_identity_digest: approved.review.facts.host_identity_digest.clone(),
            policy_digest: approved.review.facts.policy_digest.clone(),
        };
        let mut next = intent.source.clone();
        next.incarnation = intent.target_incarnation;
        next.command_id = intent.command_id;
        next.restart_from = Some(old);
        next.config_path = Some(intent.target_config.clone());
        next.executable = Some(self.binary.clone());
        next.peer_uids = Some(binding.peer_uids.clone());
        next.state = ProcessState::Starting;
        next.token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        if let Ok(current) = self.registration(session).await {
            if current.incarnation == intent.target_incarnation {
                let info = super::bound_lifecycle::inspect(&self.directory, &current).await;
                return Ok(
                    serde_json::json!({"transition":{"review_id":intent.review_id,"session_id":session,"incarnation":current.incarnation,"process_state":info.state,"phase":"admission_retained"},"message":"Retained admission observed. Reconciliation never repeats guardian launch."}),
                );
            }
            let mut immutable = current.clone();
            immutable.state = intent.source.state.clone();
            ensure!(
                serde_json::to_vec(&immutable)? == serde_json::to_vec(&intent.source)?,
                "source registration changed during reconciliation"
            );
        }
        write(
            &self.directory,
            intent.target_incarnation,
            "dormant",
            &(session, intent.review_id),
        )?;
        if !launch {
            next.state = ProcessState::Stopped;
        }
        let previous = self.registration(session).await?;
        let mut immutable = previous.clone();
        immutable.state = intent.source.state.clone();
        ensure!(
            serde_json::to_vec(&immutable)? == serde_json::to_vec(&intent.source)?,
            "source admission changed during handoff"
        );
        self.transition_authority(grant, intent, approved).await?;
        database::transition_bound(
            &self.directory,
            &previous,
            &next,
            &binding,
            serde_json::to_vec(&VesselCommand::Execution {
                operation: ExecutionOperation::Approve {
                    approval: ReviewApproval {
                        review_id: intent.review_id,
                        command_id: intent.command_id,
                        digest: approved.review.digest.clone(),
                    },
                },
            })?,
            &approved.review,
        )
        .await?;
        if !launch {
            return Ok(
                serde_json::json!({"transition":{"review_id":intent.review_id,"session_id":session,"incarnation":next.incarnation,"process_state":"stopped","phase":"handoff_committed"},"message":"Exact handoff metadata committed. No process launched; explicitly restart this voyage to launch the reviewed target."}),
            );
        }
        self.transition_authority(grant, intent, approved).await?;
        write(
            &self.directory,
            intent.target_incarnation,
            "launch-intent",
            &(session, intent.review_id),
        )?;
        self.spawn_bound_guardian(&next).await?;
        let info = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let info = super::bound_lifecycle::inspect(&self.directory, &next).await;
                if info.state == ProcessState::Live {
                    return info;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await?;
        self.transition_authority(grant, intent, approved).await?;
        ensure!(
            info.incarnation == intent.target_incarnation,
            "transition process incarnation changed"
        );
        Ok(serde_json::to_value(super::admin_execution::observation(
            &self.directory,
            session,
            intent.target_incarnation,
        )?)?)
    }
}

/// Protected proof that a committed target incarnation was never launched.
/// Any retained launch intent fences this proof even without a child receipt.
pub(super) fn dormant(root: &Path, session: Uuid, incarnation: Uuid) -> Result<bool> {
    let admission = RootDirectory::open(root)?
        .child("guardians".as_ref())
        .and_then(|directory| directory.child(session.to_string().as_ref()))
        .and_then(|directory| directory.child(incarnation.to_string().as_ref()))
        .and_then(|directory| directory.read("admission.json".as_ref(), 4096));
    match admission {
        Ok(_) => return Ok(false),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
        Err(error) => return Err(error),
    }
    let marker = read::<(Uuid, Uuid)>(root, incarnation, "dormant")?;
    ensure!(marker.0 == session, "transition dormant identity mismatch");
    match read::<(Uuid, Uuid)>(root, incarnation, "launch-intent") {
        Ok(_) => Ok(false),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(true)
        }
        Err(error) => Err(error),
    }
}
pub(super) fn carry_retained_digest(
    root: &Path,
    previous: &ProcessRegistration,
    next: &ProcessRegistration,
) -> Result<()> {
    if dormant(root, previous.session_id, previous.incarnation).unwrap_or(false)
        && let Some(digest) = retained_digest(root, previous)?
    {
        write(
            root,
            next.incarnation,
            "retained-config",
            &(next.session_id, digest),
        )?;
    }
    Ok(())
}
