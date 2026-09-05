use super::*;
use crate::{
    config::{AccessMode, Config},
    policy::Policy,
    tools::{InteractionMode, Redactor, ToolRegistry, UnattendedApprover},
    workflow::secrets::SecretInputs,
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn context(path: &std::path::Path) -> ToolContext {
    ToolContext {
        completion: None,
        policy: Arc::new(
            Policy::new(
                &Config {
                    access: Some(AccessMode::Unrestricted),
                    ..Config::default()
                },
                path.into(),
            )
            .unwrap(),
        ),
        approver: Arc::new(UnattendedApprover { allow: false }),
        timeout: Duration::from_secs(3),
        max_output_bytes: 3,
        environment: [("PATH".into(), std::env::var("PATH").unwrap_or_default())]
            .into_iter()
            .collect(),
        cancellation: CancellationToken::new(),
        execution_id: Uuid::new_v4(),
        interaction: InteractionMode::Unattended,
        redactor: Arc::new(Redactor::new(["status".into(), "exited".into()])),
    }
}
fn bindings(run: Uuid, value: &str) -> crate::workflow::secrets::RunBindings {
    let d=crate::workflow::parse(b"schema_version=1\nid='secret-check'\nversion='1'\ndescription='check'\nprompt='Use {{token}}'\n[parameters.token]\ntype='string'\nsecret=true\nrequired=true\n").unwrap();
    SecretInputs::collect(&d, vec![("token".into(), value.into())])
        .unwrap()
        .bind(run)
        .unwrap()
}

#[tokio::test]
async fn bound_shell_uses_environment_but_suppresses_short_unicode_split_and_encoded_output() {
    let modes: &[bool] = if cfg!(target_os = "linux") {
        &[false, true]
    } else {
        &[false]
    };
    for managed in modes {
        for value in ["q", "秘密🦀", "quotes\"slash\\newline\n"] {
            let dir = tempfile::tempdir().unwrap();
            let ctx = context(dir.path());
            let bound = bindings(ctx.execution_id, value);
            let mut registry = ToolRegistry::default();
            let manager = managed.then(ManagedShell::new);
            if let Some(manager) = &manager {
                registry.register(manager.clone());
            } else {
                registry.register(Shell);
            }
            // Child proves it received the value by writing its digest, not the
            // value. Both raw/split and encoded stdout/stderr must be null before
            // the ordinary three-byte output truncation could leak a prefix.
            let script = "python3 -c 'import os,sys,hashlib,base64; v=os.environ[\"HELM_WORKFLOW_TOKEN\"].encode(); open(\"digest\",\"w\").write(hashlib.sha256(v).hexdigest()); [os.write(1,bytes([c])) for c in v]; os.write(2,base64.b64encode(v)); os.write(1,v.hex().encode())'";
            let result = registry
                .execute_with_workflow_secrets(
                    "shell",
                    json!({"command":script,"workflow_secrets":["token"]}),
                    &ctx,
                    Some(&bound),
                )
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&result).unwrap(),
                json!({"status":"exited","code":0})
            );
            assert!(!result.contains(value));
            use sha2::Digest;
            assert_eq!(
                std::fs::read_to_string(dir.path().join("digest")).unwrap(),
                hex::encode(sha2::Sha256::digest(value.as_bytes()))
            );
            if let Some(manager) = manager {
                assert!(
                    manager
                        .shutdown(Duration::from_secs(5))
                        .await
                        .observation_complete
                );
            }
            assert!(
                ctx.environment
                    .keys()
                    .all(|k| !k.starts_with("HELM_WORKFLOW_"))
            );
        }
    }
}

#[tokio::test]
async fn secret_references_require_exact_current_run_and_explicit_shell_opt_in_before_effects() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let bound = bindings(ctx.execution_id, "synthetic-private-material");
    let mut registry = ToolRegistry::standard();
    registry.register(Shell);
    for (name, args, provided) in [
        (
            "shell",
            json!({"command":"touch forbidden","workflow_secrets":["token"]}),
            None,
        ),
        (
            "shell",
            json!({"command":"touch forbidden","workflow_secrets":["unknown"]}),
            Some(&bound),
        ),
        (
            "shell",
            json!({"command":"touch forbidden","workflow_secrets":["token","token"]}),
            Some(&bound),
        ),
        (
            "process",
            json!({"action":"start","command":"touch forbidden","workflow_secrets":["token"]}),
            Some(&bound),
        ),
        (
            "read_file",
            json!({"path":"missing","workflow_secrets":["token"]}),
            Some(&bound),
        ),
        (
            "mcp_missing",
            json!({"workflow_secrets":["token"]}),
            Some(&bound),
        ),
        (
            "subagent",
            json!({"action":"spawn","task":"read secret","workflow_secrets":["token"]}),
            Some(&bound),
        ),
    ] {
        assert!(
            registry
                .execute_with_workflow_secrets(name, args, &ctx, provided)
                .await
                .is_err()
        );
        assert!(!dir.path().join("forbidden").exists());
    }
    let mut later = ctx.clone();
    later.execution_id = Uuid::new_v4();
    assert!(
        registry
            .execute_with_workflow_secrets(
                "shell",
                json!({"command":"touch forbidden","workflow_secrets":["token"]}),
                &later,
                Some(&bound)
            )
            .await
            .is_err()
    );
    let ordinary = registry
        .execute_with_workflow_secrets(
            "shell",
            json!({"command":"test -z \"$HELM_WORKFLOW_TOKEN\""}),
            &ctx,
            Some(&bound),
        )
        .await
        .unwrap();
    assert!(ordinary.starts_with("exi")); // ordinary output remains bounded at three bytes
    assert!(!dir.path().join("forbidden").exists());
}

#[tokio::test]
async fn bound_shell_preserves_read_only_denial_and_cancel_before_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = context(dir.path());
    let bound = bindings(ctx.execution_id, "synthetic-private-material");
    let mut registry = ToolRegistry::default();
    registry.register(Shell);
    ctx.policy = Arc::new(
        Policy::new(
            &Config {
                access: Some(AccessMode::ReadOnly),
                ..Config::default()
            },
            dir.path().into(),
        )
        .unwrap(),
    );
    let args = json!({"command":"touch forbidden","workflow_secrets":["token"]});
    assert!(matches!(
        registry
            .execute_with_workflow_secrets("shell", args.clone(), &ctx, Some(&bound))
            .await,
        Err(ToolError::Denied(_))
    ));
    ctx.policy = context(dir.path()).policy;
    ctx.cancellation.cancel();
    assert!(matches!(
        registry
            .execute_with_workflow_secrets("shell", args, &ctx, Some(&bound))
            .await,
        Err(ToolError::Cancelled)
    ));
    assert!(!dir.path().join("forbidden").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn private_managed_shell_cancellation_and_abandoned_future_keep_observed_cleanup() {
    for abandoned in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = context(dir.path());
        ctx.timeout = Duration::from_secs(10);
        let bound = bindings(ctx.execution_id, "synthetic-private-material");
        let manager = ManagedShell::new();
        let retained = manager.clone();
        let mut registry = ToolRegistry::default();
        registry.register(manager);
        let cancel = ctx.cancellation.clone();
        let task = tokio::spawn(async move {
            registry.execute_with_workflow_secrets("shell",json!({"command":"printf '%s' \"$HELM_WORKFLOW_TOKEN\"; echo $$ > leader; sleep 10","workflow_secrets":["token"]}),&ctx,Some(&bound)).await
        });
        tokio::time::timeout(Duration::from_secs(4), async {
            while !dir.path().join("leader").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        if abandoned {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            cancel.cancel();
            assert!(matches!(task.await.unwrap(), Err(ToolError::Cancelled)));
        }
        let report = retained.shutdown(Duration::from_secs(5)).await;
        assert!(report.observation_complete && report.remaining.is_empty());
        assert!(
            retained
                .shutdown(Duration::from_millis(1))
                .await
                .observation_complete
        );
    }
}

#[tokio::test]
async fn private_shell_preserves_approval_denial_environment_conflicts_and_sanitized_timeouts() {
    for managed in [false, true] {
        if managed && !cfg!(target_os = "linux") {
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = context(dir.path());
        let bound = bindings(ctx.execution_id, "sensitive-error-秘密");
        let mut registry = ToolRegistry::default();
        let manager = managed.then(ManagedShell::new);
        if let Some(manager) = &manager {
            registry.register(manager.clone());
        } else {
            registry.register(Shell);
        }
        let command =
            serde_json::json!({"command":"touch forbidden", "workflow_secrets":["token"]});
        ctx.environment
            .insert("HELM_WORKFLOW_TOKEN".into(), "configured-value".into());
        let error = registry
            .execute_with_workflow_secrets("shell", command.clone(), &ctx, Some(&bound))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::InvalidArguments(_)));
        assert!(!dir.path().join("forbidden").exists());
        ctx.environment.remove("HELM_WORKFLOW_TOKEN");
        ctx.policy = Arc::new(
            Policy::new(
                &Config {
                    access: Some(AccessMode::Approval),
                    ..Config::default()
                },
                dir.path().into(),
            )
            .unwrap(),
        );
        let error = registry
            .execute_with_workflow_secrets("shell", command, &ctx, Some(&bound))
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::Denied(_)));
        assert!(!dir.path().join("forbidden").exists());
        ctx.policy = context(dir.path()).policy;
        ctx.timeout = Duration::from_millis(80);
        let error = registry.execute_with_workflow_secrets("shell", serde_json::json!({"command":"printf '%s' \"$HELM_WORKFLOW_TOKEN\"; sleep 2", "workflow_secrets":["token"]}), &ctx, Some(&bound)).await.unwrap_err();
        assert!(matches!(error, ToolError::Timeout(_)));
        assert!(!error.to_string().contains("sensitive-error"));
        if let Some(manager) = manager {
            assert!(
                manager
                    .shutdown(Duration::from_secs(5))
                    .await
                    .observation_complete
            );
            assert!(registry.execute_with_workflow_secrets("shell", serde_json::json!({"command":"touch forbidden", "workflow_secrets":["token"]}), &ctx, Some(&bound)).await.is_err());
            assert!(!dir.path().join("forbidden").exists());
        }
    }
}

struct WaitingApproval {
    entered: tokio::sync::Notify,
}
#[async_trait]
impl crate::tools::Approver for WaitingApproval {
    async fn approve(
        &self,
        request: &crate::tools::ApprovalRequest,
    ) -> crate::tools::ApprovalOutcome {
        let encoded = serde_json::to_string(request).unwrap();
        assert!(!encoded.contains("private-approval-秘密"));
        self.entered.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn cancelling_private_approval_does_not_spawn_or_expose_binding() {
    for managed in [false, true] {
        if managed && !cfg!(target_os = "linux") {
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = context(dir.path());
        ctx.policy = Arc::new(
            Policy::new(
                &Config {
                    access: Some(AccessMode::Approval),
                    ..Config::default()
                },
                dir.path().into(),
            )
            .unwrap(),
        );
        let approver = Arc::new(WaitingApproval {
            entered: tokio::sync::Notify::new(),
        });
        ctx.approver = approver.clone();
        let bound = bindings(ctx.execution_id, "private-approval-秘密");
        let mut registry = ToolRegistry::default();
        let manager = managed.then(ManagedShell::new);
        if let Some(manager) = &manager {
            registry.register(manager.clone());
        } else {
            registry.register(Shell);
        }
        let run = registry.execute_with_workflow_secrets(
            "shell",
            serde_json::json!({"command":"touch forbidden", "workflow_secrets":["token"]}),
            &ctx,
            Some(&bound),
        );
        tokio::pin!(run);
        tokio::select! { _=approver.entered.notified()=>{}, result=&mut run=>panic!("unexpected early result {result:?}") }
        ctx.cancellation.cancel();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), run)
                .await
                .unwrap(),
            Err(ToolError::Cancelled)
        ));
        assert!(!dir.path().join("forbidden").exists());
        if let Some(manager) = manager {
            assert!(
                manager
                    .shutdown(Duration::from_secs(5))
                    .await
                    .observation_complete
            );
        }
    }
}

fn delete_private_profile(directory: &std::path::Path) {
    use crate::policy_profile::store::{Action, ProfileChange, ProfileStore};
    ProfileStore::open(directory)
        .unwrap()
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "private".into(),
            expected_revision: 1,
            action: Action::Delete {},
        })
        .unwrap();
}
struct ChangingProfileApproval {
    directory: std::path::PathBuf,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait]
impl crate::tools::Approver for ChangingProfileApproval {
    async fn approve(&self, _: &crate::tools::ApprovalRequest) -> crate::tools::ApprovalOutcome {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if crate::policy_profile::store::ProfileStore::open(&self.directory)
            .unwrap()
            .inspect("private")
            .unwrap()
            .unwrap()
            .rules
            .is_some()
        {
            delete_private_profile(&self.directory);
        }
        crate::tools::ApprovalOutcome::Approved
    }
}

#[tokio::test]
async fn private_binding_rechecks_selected_profile_before_and_after_approval() {
    use crate::policy_profile::{
        Builtin,
        selection::{Selection, SelectionRequest},
        store::{Action, ProfileChange, ProfileStore},
    };
    for managed in [false, true] {
        if managed && !cfg!(target_os = "linux") {
            continue;
        }
        for stale_before_approval in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let directory = dir.path().join("profiles");
            let snapshot = ProfileStore::open(&directory)
                .unwrap()
                .change(&ProfileChange {
                    operation_id: Uuid::new_v4(),
                    name: "private".into(),
                    expected_revision: 0,
                    action: Action::Create {
                        rules: Builtin::Balanced.document().rules,
                    },
                })
                .unwrap()
                .snapshot;
            let mut config = Config {
                access: Some(AccessMode::Unrestricted),
                ..Config::default()
            };
            let request = SelectionRequest {
                directory: directory.clone(),
                name: "private".into(),
                revision: 1,
                digest: snapshot.digest().unwrap(),
                explicit: Default::default(),
            };
            let preview = Selection::preview(&config, dir.path(), &request).unwrap();
            config.policy_profile = Some(
                Selection::bind(
                    &config,
                    dir.path(),
                    request,
                    Some(&preview.transition_digest),
                )
                .unwrap(),
            );
            let mut ctx = context(dir.path());
            ctx.policy = Arc::new(Policy::new(&config, dir.path().into()).unwrap());
            let approver = Arc::new(ChangingProfileApproval {
                directory: directory.clone(),
                calls: Default::default(),
            });
            ctx.approver = approver.clone();
            if stale_before_approval {
                delete_private_profile(&directory);
            }
            let bound = bindings(ctx.execution_id, "private-freshness-秘密");
            let mut registry = ToolRegistry::default();
            let manager = managed.then(ManagedShell::new);
            if let Some(manager) = &manager {
                registry.register(manager.clone());
            } else {
                registry.register(Shell);
            }
            let result = registry
                .execute_with_workflow_secrets(
                    "shell",
                    serde_json::json!({"command":"touch forbidden", "workflow_secrets":["token"]}),
                    &ctx,
                    Some(&bound),
                )
                .await;
            assert!(
                matches!(result, Err(ToolError::Denied(_))),
                "stale private binding admitted: {result:?}"
            );
            assert!(!dir.path().join("forbidden").exists());
            assert_eq!(
                approver.calls.load(std::sync::atomic::Ordering::SeqCst),
                usize::from(!stale_before_approval)
            );
            if let Some(manager) = manager {
                assert!(
                    manager
                        .shutdown(Duration::from_secs(5))
                        .await
                        .observation_complete
                );
            }
        }
    }
}

#[derive(Debug)]
struct RevocablePrivateAuthority(std::sync::atomic::AtomicBool);
impl crate::policy::ExecutionAuthority for RevocablePrivateAuthority {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.0.load(std::sync::atomic::Ordering::SeqCst), "revoked");
        Ok(())
    }
}
#[derive(Default)]
struct PausedPrivateApproval {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait]
impl crate::tools::Approver for PausedPrivateApproval {
    async fn approve(
        &self,
        request: &crate::tools::ApprovalRequest,
    ) -> crate::tools::ApprovalOutcome {
        assert!(
            !serde_json::to_string(request)
                .unwrap()
                .contains("private-authority-秘密")
        );
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        crate::tools::ApprovalOutcome::Approved
    }
}

#[tokio::test]
async fn private_shell_dispatch_preserves_revocation_and_cancellation_before_and_during_approval() {
    use std::sync::atomic::Ordering::SeqCst;
    for managed in [false, true] {
        if managed && !cfg!(target_os = "linux") {
            continue;
        }
        for during_approval in [false, true] {
            for revoke in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let mut ctx = context(dir.path());
                let authority = Arc::new(RevocablePrivateAuthority(
                    std::sync::atomic::AtomicBool::new(true),
                ));
                ctx.policy = Arc::new(
                    Policy::new(
                        &Config {
                            access: Some(AccessMode::Approval),
                            ..Config::default()
                        },
                        dir.path().into(),
                    )
                    .unwrap()
                    .with_execution_authority(authority.clone()),
                );
                let approval = Arc::new(PausedPrivateApproval::default());
                ctx.approver = approval.clone();
                let bound = bindings(ctx.execution_id, "private-authority-秘密");
                let mut registry = ToolRegistry::default();
                let manager = managed.then(ManagedShell::new);
                if let Some(manager) = &manager {
                    registry.register(manager.clone());
                } else {
                    registry.register(Shell);
                }
                let run = registry.execute_with_workflow_secrets(
                    "shell",
                    serde_json::json!({"command":"touch forbidden", "workflow_secrets":["token"]}),
                    &ctx,
                    Some(&bound),
                );
                tokio::pin!(run);
                if during_approval {
                    tokio::select! { _=approval.entered.notified()=>{}, result=&mut run=>panic!("approval was not awaited: {result:?}") }
                }
                if revoke {
                    authority.0.store(false, SeqCst);
                } else {
                    ctx.cancellation.cancel();
                }
                approval.release.notify_one();
                let result = tokio::time::timeout(Duration::from_secs(2), run)
                    .await
                    .unwrap();
                if revoke {
                    assert!(matches!(result, Err(ToolError::Denied(_))));
                } else {
                    assert!(matches!(result, Err(ToolError::Cancelled)));
                }
                assert_eq!(approval.calls.load(SeqCst), usize::from(during_approval));
                assert!(!dir.path().join("forbidden").exists());
                if let Some(manager) = manager {
                    assert!(
                        manager
                            .shutdown(Duration::from_secs(5))
                            .await
                            .observation_complete
                    );
                }
            }
        }
    }
}

/// This trusted test adapter deliberately performs no policy checks of its own:
/// the registry must guard every private implementation and wrap its approver.
struct ApprovalOnlyPrivateShell(Arc<std::sync::atomic::AtomicBool>);
#[async_trait]
impl Tool for ApprovalOnlyPrivateShell {
    fn definition(&self) -> crate::model::ToolDefinition {
        Shell.definition()
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> Result<String, ToolError> {
        unreachable!()
    }
    async fn execute_secret_environment(
        &self,
        _: serde_json::Value,
        ctx: &ToolContext,
        _: crate::workflow::secrets::BoundEnvironment,
    ) -> Result<SecretShellOutcome, ToolError> {
        let approval = ctx.approval("shell", "test private dispatch", "test approval".into());
        if !ctx.approver.approve(&approval).await.approved() {
            return Err(ToolError::Denied("approval denied".into()));
        }
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(SecretShellOutcome::Exited { code: 0 })
    }
}
#[tokio::test]
async fn registry_private_branch_uses_common_authority_guard_and_wrapped_approver() {
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    for during_approval in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = context(dir.path());
        let authority = Arc::new(RevocablePrivateAuthority(AtomicBool::new(true)));
        ctx.policy = Arc::new(
            (*ctx.policy)
                .clone()
                .with_execution_authority(authority.clone()),
        );
        let approval = Arc::new(PausedPrivateApproval::default());
        ctx.approver = approval.clone();
        let effect = Arc::new(AtomicBool::new(false));
        let mut registry = ToolRegistry::default();
        registry.register(ApprovalOnlyPrivateShell(effect.clone()));
        let bound = bindings(ctx.execution_id, "private-authority-秘密");
        let run = registry.execute_with_workflow_secrets(
            "shell",
            serde_json::json!({"command":"unused", "workflow_secrets":["token"]}),
            &ctx,
            Some(&bound),
        );
        tokio::pin!(run);
        if during_approval {
            tokio::select! { _=approval.entered.notified()=>{}, result=&mut run=>panic!("approval was not awaited: {result:?}") }
        }
        authority.0.store(false, SeqCst);
        approval.release.notify_one();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), run)
                .await
                .unwrap(),
            Err(ToolError::Denied(_))
        ));
        assert!(!effect.load(SeqCst));
        assert_eq!(approval.calls.load(SeqCst), usize::from(during_approval));
    }
}
