//! Offline reconciliation never constructs an executor or submits provider work.
use super::*;
use crate::attachment::{
    local_actor::{LocalActorStore, storage::Directory},
    runtime::ManagedSessionOwner,
};
use voyage_protocol::identity_helper::BoundRecoveryRequest;

pub(super) async fn recover(request: BoundRecoveryRequest) -> Result<serde_json::Value> {
    validate(&request)?;
    #[cfg(target_os = "linux")]
    {
        let authority = super::authority::Pipe::open()?;
        let actor = request.actor.clone();
        let command = request.command_id;
        recover_checked(request, &|| {
            ensure!(
                authority.allows(&actor, IdentityAuthorityRight::Recover, command, None),
                "current offline recovery authority refused"
            );
            Ok(())
        })
        .await
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("offline identity recovery unsupported")
}

async fn recover_checked(
    request: BoundRecoveryRequest,
    check: &(dyn Fn() -> Result<()> + Sync),
) -> Result<serde_json::Value> {
    validate(&request)?;
    check()?;
    let directory = Directory::open_existing(&request.directory)?;
    let startup =
        crate::attachment::journal::open_private_file(&request.directory.join("startup.lock"))?;
    startup
        .try_lock()
        .map_err(|_| anyhow::anyhow!("runtime startup remains owned"))?;
    directory.verify()?;
    let owner =
        ManagedSessionOwner::open(request.directory.join("journal"), request.session_id).await?;
    let snapshot = owner.snapshot().await?;
    ensure!(
        snapshot.session.workspace == Path::new(&request.actor.workspace),
        "offline actor workspace does not match canonical session"
    );
    check()?;
    owner.initialize_process_commands().await?;
    check()?;
    owner.initialize_session_resources().await?;
    let receipts = Directory::open(&request.directory.join("bound-recoveries"))?;
    let _lock = receipts.lock()?;
    let name = format!("{}.json", request.command_id);
    let intent = serde_json::to_value(&request)?;
    if let Some(bytes) = receipts.read_bounded(&name, 65_536)? {
        let prior: serde_json::Value = serde_json::from_slice(&bytes)?;
        ensure!(
            prior["request"] == intent,
            "bound recovery command conflict"
        );
        if let Some(result) = prior.get("result") {
            ensure!(
                result["session_id"] == request.session_id.to_string()
                    && result["incarnation"] == request.incarnation.to_string()
                    && result["command_id"] == request.command_id.to_string()
                    && result["restart_permitted"] == false
                    && result["execution_authorized"] == false,
                "bound recovery receipt identity mismatch"
            );
            return Ok(result.clone());
        }
        ensure!(
            !(prior["operator_effects_claimed"] == true && operator_changes(&request)),
            "operator reconciliation outcome uncertain; inspect retained receipt before any new attestation"
        );
        // Local journal reconciliation is exact and idempotent. This retained
        // intent never authorizes replay of tools, goals or external cleanup.
    } else {
        receipts.publish_new(
            &name,
            &serde_json::to_vec(&serde_json::json!({"request":intent}))?,
        )?;
    }
    check()?;
    owner.recover_interrupted().await?;
    check()?;
    owner.fence_offline_goal().await?;
    // Protected guardian ECHILD or a never-launched marker establishes local
    // process retirement only. External effects/resources remain retained.
    if request.current_scope_cleanup_observed {
        check()?;
        crate::host_resources::recover_process_scope(request.session_id, request.incarnation)?;
    }
    // Private legacy run/resource rows do not carry sufficient incarnation
    // evidence to promote all of them from this guardian witness. Retain them
    // unknown until the current owner explicitly attests their exact IDs.
    if operator_changes(&request) {
        receipts.publish(
            &name,
            &serde_json::to_vec(
                &serde_json::json!({"request":intent,"operator_effects_claimed":true}),
            )?,
        )?;
    }
    let actor = LocalActorStore::open(&request.directory.join("identity"))?.identity()?;
    if let Some(run) = request.acknowledge_cleanup {
        check()?;
        owner.attest_local_cleanup(run, actor).await?;
    }
    for resource in &request.acknowledge_resources {
        check()?;
        owner.attest_session_resource(*resource, actor).await?;
    }
    if let Some(run) = request.reconcile_tools {
        check()?;
        owner
            .reconcile_local_tools(crate::attachment::journal::LocalReconcileRequest {
                session_id: request.session_id,
                run_id: run,
                installation_id: actor.installation_id,
                principal_id: actor.principal_id,
                expected_revision: request
                    .expected_revision
                    .ok_or_else(|| anyhow::anyhow!("exact reconciliation revision required"))?,
            })
            .await?;
    }
    check()?;
    owner.retain_interrupted_cleanup().await?;
    check()?;
    owner.recover_tool_outcomes().await?;
    let snapshot = owner.process_snapshot().await?;
    let retained = snapshot["retained_cleanup"]["run_ids"]
        .as_array()
        .is_some_and(|v| !v.is_empty())
        || snapshot["retained_cleanup"]["resources"]
            .as_array()
            .is_some_and(|v| !v.is_empty());
    let retained_run_ids = ids(&snapshot["retained_cleanup"]["run_ids"])?;
    let retained_resource_ids = ids(&snapshot["retained_cleanup"]["resources"])?;
    let last_run_id = snapshot["run"]["run_id"]
        .as_str()
        .map(str::parse::<uuid::Uuid>)
        .transpose()?;
    // This helper establishes only retired-owner journal bookkeeping. A later
    // explicit Restart has its own current binding and admission checks.
    let result = serde_json::json!({"session_id":request.session_id,"incarnation":request.incarnation,"command_id":request.command_id,"revision":snapshot["revision"],"cleanup_disposition":if retained{"unresolved_retained"}else if owner.has_cleanup_attestation().await?{"operator_attested"}else{"observed"},"restart_permitted":false,"execution_authorized":false,"retained_run_ids":retained_run_ids,"retained_resource_ids":retained_resource_ids,"last_run_id":last_run_id,"local_process_retired":true});
    check()?;
    receipts.publish(
        &name,
        &serde_json::to_vec(&serde_json::json!({"request":intent,"result":result}))?,
    )?;
    Ok(result)
}

fn operator_changes(request: &BoundRecoveryRequest) -> bool {
    request.acknowledge_cleanup.is_some()
        || !request.acknowledge_resources.is_empty()
        || request.reconcile_tools.is_some()
}

pub(super) fn validate_workspace(workspace: &Path, request: &BoundRecoveryRequest) -> Result<()> {
    validate(request)?;
    let actor_workspace = Path::new(&request.actor.workspace);
    // Canonical spelling is checked lexically because the original project may
    // have been deleted or unmounted. No project metadata is consulted here.
    let spelling: std::path::PathBuf = workspace.components().collect();
    ensure!(
        workspace.is_absolute()
            && workspace.components().all(|component| matches!(
                component,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            ))
            && spelling.as_os_str() == workspace.as_os_str()
            && workspace.as_os_str() == actor_workspace.as_os_str()
            && !request.actor.workspace.contains('\0'),
        "offline workspace label does not match exact stored actor"
    );
    Ok(())
}

fn validate(request: &BoundRecoveryRequest) -> Result<()> {
    ensure!(
        !request.session_id.is_nil()
            && !request.incarnation.is_nil()
            && !request.command_id.is_nil()
            && request.local_process_retired,
        "positive exact retirement required"
    );
    ensure!(
        request.acknowledge_resources.len() <= 256
            && request.acknowledge_resources.iter().all(|id| !id.is_nil())
            && (request.reconcile_tools.is_none() || request.expected_revision.is_some()),
        "invalid bound recovery request"
    );
    ensure!(
        request.directory.is_absolute()
            && !request.actor.principal.is_empty()
            && request.actor.principal.len() <= 256
            && Path::new(&request.actor.workspace).is_absolute()
            && request.actor.workspace.len() <= 4096,
        "invalid recovery actor or private realm"
    );
    ensure!(
        request.acknowledge_cleanup.is_none_or(|id| !id.is_nil())
            && request.reconcile_tools.is_none_or(|id| !id.is_nil()),
        "nil reconciliation reference"
    );
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use uuid::Uuid;
    fn request(path: &Path) -> BoundRecoveryRequest {
        BoundRecoveryRequest {
            directory: path.to_owned(),
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            local_process_retired: true,
            current_scope_cleanup_observed: false,
            actor: voyage_protocol::accounts::EnrollmentActor {
                principal: "synthetic current owner".into(),
                workspace: path.to_string_lossy().into_owned(),
            },
            acknowledge_cleanup: None,
            acknowledge_resources: vec![],
            reconcile_tools: None,
            expected_revision: None,
        }
    }
    #[tokio::test]
    async fn absent_retirement_or_current_authority_cannot_open_private_state() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("never-opened");
        let mut value = request(&missing);
        value.local_process_retired = false;
        assert!(recover_checked(value, &|| Ok(())).await.is_err());
        assert!(!missing.exists());
        assert!(
            recover_checked(request(&missing), &|| anyhow::bail!(
                "synthetic revoked authority"
            ))
            .await
            .is_err()
        );
        assert!(!missing.exists());
    }
    #[tokio::test]
    async fn malformed_attestations_and_actor_references_are_rejected_before_effects() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("never-opened");
        for field in 0..5 {
            let mut value = request(&missing);
            match field {
                0 => value.acknowledge_resources = vec![Uuid::nil()],
                1 => value.reconcile_tools = Some(Uuid::new_v4()),
                2 => value.actor.principal.clear(),
                3 => value.acknowledge_cleanup = Some(Uuid::nil()),
                _ => value.command_id = Uuid::nil(),
            };
            assert!(recover_checked(value, &|| Ok(())).await.is_err());
            assert!(!missing.exists());
        }
    }
    #[test]
    fn offline_workspace_label_is_exact_without_project_filesystem_access() {
        let root = tempfile::tempdir().unwrap();
        let absent = root.path().join("absent-project");
        let mut value = request(&root.path().join("private-runtime"));
        value.actor.workspace = absent.to_str().unwrap().into();
        assert!(validate_workspace(&absent, &value).is_ok());
        assert!(!absent.exists());
        assert!(validate_workspace(root.path(), &value).is_err());
        for suffix in ["/../other", "/./nested", "/nested/", "//nested"] {
            let altered = std::path::PathBuf::from(format!("{}{suffix}", absent.display()));
            value.actor.workspace = altered.to_str().unwrap().into();
            assert!(validate_workspace(&altered, &value).is_err());
        }
        value.actor.workspace = "relative-project".into();
        assert!(validate_workspace(Path::new("relative-project"), &value).is_err());
        assert!(!absent.exists());
    }
    const CHILD: &str = "identity_helper::recovery::tests::isolated_recovery_child";
    #[test]
    fn isolated_recovery_child() {
        let Ok(mode) = std::env::var("VOYAGE_BOUND_RECOVERY_FIXTURE") else {
            return;
        };
        let root =
            std::path::PathBuf::from(std::env::var_os("VOYAGE_BOUND_RECOVERY_ROOT").unwrap());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let directory = Directory::open(&root.join("runtime")).unwrap();
            drop(directory);
            let workspace = if mode == "deleted_workspace" {
                let workspace = root.join("deleted-project");
                std::fs::create_dir(&workspace).unwrap();
                let workspace = workspace.canonicalize().unwrap();
                std::fs::remove_dir(&workspace).unwrap();
                workspace
            } else {
                root.canonicalize().unwrap()
            };
            let mut session =
                crate::session::Session::new(workspace.clone(), "synthetic-no-provider".into());
            session.name = Some("retained canonical name".into());
            let actor = LocalActorStore::open(&root.join("runtime/identity"))
                .unwrap()
                .identity()
                .unwrap();
            let _ = actor;
            let mut journal =
                crate::attachment::journal::Journal::open(root.join("runtime/journal")).unwrap();
            journal.create_session(&session).unwrap();
            drop(journal);
            let mut value = request(&root.join("runtime"));
            value.session_id = session.id;
            value.actor.workspace = workspace.to_string_lossy().into_owned();
            validate_workspace(&workspace, &value).unwrap();
            if mode == "startup_owned" {
                let lock = crate::attachment::journal::open_private_file(
                    &root.join("runtime/startup.lock"),
                )
                .unwrap();
                lock.try_lock().unwrap();
                assert!(recover_checked(value, &|| Ok(())).await.is_err());
            } else if mode == "operator_uncertain" {
                value.acknowledge_cleanup = Some(Uuid::new_v4());
                let receipts = Directory::open(&root.join("runtime/bound-recoveries")).unwrap();
                let name = format!("{}.json", value.command_id);
                let intent = serde_json::to_value(&value).unwrap();
                let prior = serde_json::to_vec(
                    &serde_json::json!({"request":intent,"operator_effects_claimed":true}),
                )
                .unwrap();
                receipts.publish_new(&name, &prior).unwrap();
                let error = recover_checked(value, &|| Ok(())).await.unwrap_err();
                assert!(error.to_string().contains("outcome uncertain"));
                assert_eq!(
                    receipts.read_bounded(&name, 65_536).unwrap().unwrap(),
                    prior
                );
            } else {
                let result = recover_checked(value.clone(), &|| Ok(())).await.unwrap();
                assert_eq!(result["session_id"], session.id.to_string());
                assert_eq!(result["execution_authorized"], false);
                assert_eq!(result["restart_permitted"], false);
                assert!(!result.to_string().contains("retained canonical name"));
                assert_eq!(result["cleanup_disposition"], "observed");
                assert_eq!(
                    recover_checked(value.clone(), &|| Ok(())).await.unwrap(),
                    result
                );
                if mode == "deleted_workspace" {
                    assert!(!workspace.exists());
                }
                value.expected_revision = Some(999);
                assert!(recover_checked(value, &|| Ok(())).await.is_err());
                let journal =
                    crate::attachment::journal::Journal::open(root.join("runtime/journal"))
                        .unwrap();
                assert_eq!(
                    journal.load_session(session.id).unwrap().session.name,
                    session.name
                );
            }
            std::fs::write(root.join("done"), mode).unwrap();
        });
    }
    fn child(mode: &str) {
        let root = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", CHILD, "--nocapture"])
            .env("VOYAGE_BOUND_RECOVERY_FIXTURE", mode)
            .env("VOYAGE_BOUND_RECOVERY_ROOT", root.path())
            .env("HOME", root.path())
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CONFIG_HOME", root.path().join("config"));
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = command.spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated recovery fixture timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(root.path().join("done")).unwrap(),
            mode
        );
    }
    #[test]
    fn private_offline_receipt_replay_is_exact_and_grants_no_execution() {
        child("receipt");
    }
    #[test]
    fn positive_retirement_cannot_override_an_owned_startup_fence() {
        child("startup_owned");
    }
    #[test]
    fn claimed_operator_reconciliation_never_replays_after_missing_receipt() {
        child("operator_uncertain");
    }
    #[test]
    fn deleted_project_does_not_prevent_retired_private_journal_bookkeeping() {
        child("deleted_workspace");
    }
}

fn ids(value: &serde_json::Value) -> Result<Vec<uuid::Uuid>> {
    let Some(values) = value.as_array() else {
        return Ok(vec![]);
    };
    ensure!(
        values.len() <= 256,
        "retained obligation metadata exceeds bounds"
    );
    values
        .iter()
        .map(|value| {
            let text = value
                .as_str()
                .or_else(|| {
                    value
                        .get("resource_id")
                        .or_else(|| value.get("id"))
                        .and_then(serde_json::Value::as_str)
                })
                .ok_or_else(|| anyhow::anyhow!("retained obligation identity unavailable"))?;
            let id = text.parse::<uuid::Uuid>()?;
            ensure!(!id.is_nil(), "nil retained obligation identity");
            Ok(id)
        })
        .collect()
}
