//! Human-only, exact-directory grants. Consent is not a generic tool approval.
use super::*;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RootPermission {
    Read,
    Write,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RootLifetime {
    CurrentRun,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootGrantRequest {
    pub id: Uuid,
    pub path: PathBuf,
    pub permission: RootPermission,
    pub lifetime: RootLifetime,
    pub reason: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RootGrantChoice {
    Approved,
    Denied,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootGrantResponse {
    pub root_grant: RootGrantChoice,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    path: PathBuf,
    permission: RootPermission,
    lifetime: RootLifetime,
    reason: String,
}
pub struct RequestFilesystemRoot;
#[async_trait]
impl Tool for RequestFilesystemRoot {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "request_filesystem_root".into(),
            description: "Request human consent for an exact existing canonical absolute directory, with read or write access for this run only. Write includes read. Never auto-approved, including unrestricted mode. Host ceilings still apply. Children and already-running processes do not inherit grants. Persistent PTY starts refuse temporary overlays; use bounded shell. Refuses unsupported platforms and unavailable human interfaces; no configuration edits or uncertain-effect replay.".into(),
            input_schema: json!({"type":"object","properties":{
                "path":{"type":"string","minLength":1},
                "permission":{"enum":["read","write"]},
                "lifetime":{"enum":["current_run"]},
                "reason":{"type":"string","minLength":1,"maxLength":2048}
            },"required":["path","permission","lifetime","reason"],"additionalProperties":false}),
            output_schema: None, annotations: None,
        }
    }
    async fn execute(&self, arguments: Value, context: &ToolContext) -> Result<String, ToolError> {
        let args: Arguments = serde_json::from_value(arguments)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        if args.reason.trim().is_empty()
            || args.reason.len() > 2048
            || args.reason.chars().any(char::is_control)
        {
            return Err(ToolError::InvalidArguments(
                "reason must be nonblank, bounded and printable".into(),
            ));
        }
        let denied = |e: anyhow::Error| ToolError::Denied(e.to_string());
        let candidate = context
            .policy
            .prepare_root(&args.path, args.permission)
            .map_err(denied)?;
        if context.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let request = RootGrantRequest {
            id: Uuid::new_v4(),
            path: args.path,
            permission: args.permission,
            lifetime: args.lifetime,
            reason: args.reason,
        };
        let outcome = tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => return Err(ToolError::Cancelled),
            result = tokio::time::timeout(context.timeout, context.approver.request_root(&request, context)) => result.map_err(|_| ToolError::ApprovalExpired)?,
        };
        match outcome {
            ApprovalOutcome::Approved => {}
            ApprovalOutcome::Denied => return Err(ToolError::ApprovalDenied),
            ApprovalOutcome::Expired => return Err(ToolError::ApprovalExpired),
            ApprovalOutcome::Cancelled => return Err(ToolError::Cancelled),
            ApprovalOutcome::Invalidated => return Err(ToolError::ApprovalInvalidated),
            ApprovalOutcome::Unavailable => return Err(ToolError::ApprovalUnavailable),
        }
        if context.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        context
            .policy
            .install_root(candidate, context.cancellation.clone())
            .map_err(denied)?;
        Ok(json!({"status":"granted","request":request,"scope":"current_run","existing_processes":"unchanged","children":"not inherited"}).to_string())
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::*;
    struct Human;
    #[async_trait]
    impl Approver for Human {
        async fn approve(&self, _: &ApprovalRequest) -> ApprovalOutcome {
            ApprovalOutcome::Denied
        }
        async fn request_root(&self, _: &RootGrantRequest, _: &ToolContext) -> ApprovalOutcome {
            ApprovalOutcome::Approved
        }
    }
    #[tokio::test]
    async fn root_tool_requires_separate_human_consent_even_when_unrestricted() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let mut ctx = crate::tools::reliability_tests::context(workspace.path());
        ctx.policy = Arc::new(
            (*ctx.policy)
                .clone()
                .with_live_access(Some(Arc::new(crate::policy::LiveAccess::new(
                    AccessMode::Unrestricted,
                ))))
                .enable_run_roots(),
        );
        ctx.approver = Arc::new(UnattendedApprover { allow: true });
        let mut tools = ToolRegistry::default();
        tools.register(RequestFilesystemRoot);
        tools.register(ReadFile);
        tools.register(WriteFile);
        let args = json!({"path": external.path(), "permission":"write", "lifetime":"current_run", "reason":"Edit requested configuration"});
        assert!(matches!(
            tools
                .execute("request_filesystem_root", args.clone(), &ctx)
                .await,
            Err(ToolError::ApprovalUnavailable)
        ));
        ctx.approver = Arc::new(Human);
        tools
            .execute("request_filesystem_root", args, &ctx)
            .await
            .unwrap();
        let path = external.path().join("configuration");
        tools
            .execute(
                "write_file",
                json!({"path":path, "content":"granted"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            tools
                .execute("read_file", json!({"path":path}), &ctx)
                .await
                .unwrap()
                .contains("granted")
        );
        ctx.execution_id = Uuid::new_v4();
        assert!(
            tools
                .execute("read_file", json!({"path":path}), &ctx)
                .await
                .is_err()
        );
        assert!(serde_json::from_value::<RootGrantResponse>(json!("approved")).is_err());
        assert!(
            serde_json::from_value::<RootGrantResponse>(
                json!({"root_grant":"approved", "extra":true})
            )
            .is_err()
        );
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod refusal_tests {
    use super::*;
    struct StaleHuman;
    #[async_trait]
    impl Approver for StaleHuman {
        async fn approve(&self, _: &ApprovalRequest) -> ApprovalOutcome {
            ApprovalOutcome::Approved
        }
        async fn request_root(
            &self,
            _: &RootGrantRequest,
            context: &ToolContext,
        ) -> ApprovalOutcome {
            context
                .policy
                .access_binding()
                .unwrap()
                .0
                .update(AccessMode::ReadOnly);
            ApprovalOutcome::Approved
        }
    }
    #[tokio::test]
    async fn root_consent_rechecks_freshness_and_persistent_processes_refuse_overlay() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let mut ctx = crate::tools::reliability_tests::context(workspace.path());
        ctx.policy = Arc::new(
            (*ctx.policy)
                .clone()
                .with_live_access(Some(Arc::new(crate::policy::LiveAccess::new(
                    AccessMode::Unrestricted,
                ))))
                .enable_run_roots(),
        );
        ctx.approver = Arc::new(StaleHuman);
        let mut tools = ToolRegistry::default();
        tools.register(RequestFilesystemRoot);
        let args = json!({"path": external.path(), "permission":"write", "lifetime":"current_run", "reason":"fixture"});
        assert!(
            tools
                .execute("request_filesystem_root", args, &ctx)
                .await
                .is_err()
        );
        assert!(
            ctx.policy
                .for_run_dispatch(ctx.execution_id)
                .resolve_read(external.path())
                .is_err()
        );
        ctx.policy
            .access_binding()
            .unwrap()
            .0
            .update(AccessMode::Unrestricted);
        let d = ctx.policy.for_run_dispatch(ctx.execution_id);
        d.install_root(
            d.prepare_root(external.path(), RootPermission::Read)
                .unwrap(),
            ctx.cancellation.clone(),
        )
        .unwrap();
        let processes = ProcessTool::default();
        tools.register(processes.clone());
        let error = tools
            .execute("process", json!({"action":"start", "command":"true"}), &ctx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("persistent PTYs"), "{error}");
        assert!(processes.metadata().unwrap().is_empty());
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;
    #[test]
    fn root_request_readonly_admission_keeps_write_forbidden() {
        assert!(super::super::allowed_in_read_only(
            "request_filesystem_root",
            &json!({"permission":"read"})
        ));
        for args in [
            json!({"permission":"write"}),
            json!({}),
            json!({"permission":"unknown"}),
        ] {
            assert!(!super::super::allowed_in_read_only(
                "request_filesystem_root",
                &args
            ));
        }
    }
}
