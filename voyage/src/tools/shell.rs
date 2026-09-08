mod managed;
pub use managed::{ManagedShell, ShellShutdown};

use crate::{
    model::ToolDefinition,
    policy::Decision,
    tools::{Tool, ToolContext, ToolError, truncate},
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::process::Command;

pub struct Shell;

#[derive(Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SecretShellOutcome {
    Exited { code: i64 },
    Signalled,
}
#[derive(Deserialize)]
struct Args {
    command: String,
    #[serde(default)]
    workflow_secrets: Vec<String>,
}

#[async_trait]
impl Tool for Shell {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "shell".into(),
            description:
                "Run a shell command in the workspace. Normally returns exit status, stdout and stderr. Optional workflow_secrets names explicitly bind current-run secret references to their documented HELM_WORKFLOW_* environment variables; these calls suppress stdout/stderr before capture and return only a fixed status/code. Unknown or stale references fail; no automatic inheritance."
                    .into(),
            input_schema: json!({"type":"object","properties":{"command":{"type":"string"},"workflow_secrets":{"type":"array","items":{"type":"string","minLength":1,"maxLength":64},"maxItems":32,"uniqueItems":true,"description":"Explicit names from current workflow secret references. Values never belong in tool arguments; output is suppressed when bindings are used."}},"required":["command"]}),
        }
    }
    async fn execute(&self, value: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: Args = serde_json::from_value(value)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        if !args.workflow_secrets.is_empty() {
            return Err(ToolError::InvalidArguments(
                "workflow secret references require the current-run registry binding".into(),
            ));
        }
        match ctx.policy.command(&args.command) {
            Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
            Decision::Ask(reason) => ctx
                .approver
                .approve(&ctx.approval("shell", &args.command, reason.clone()))
                .await
                .require_approved()?,
            _ => {}
        }
        let mut command = Command::from(
            ctx.policy
                .process_command("sh", ctx.policy.workspace())
                .map_err(|e| ToolError::Failed(e.to_string()))?,
        );
        command
            .arg("-lc")
            .arg(&args.command)
            .current_dir(ctx.policy.workspace())
            .env_clear()
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.envs(&ctx.environment);
        #[cfg(unix)]
        command.process_group(0);
        ctx.policy
            .isolate_process(command.as_std_mut(), ctx.policy.workspace())
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        let child = command
            .spawn()
            .map_err(|error| ToolError::Failed(error.to_string()))?;
        let process_id = child.id();
        let output = tokio::select! {
            _ = ctx.cancellation.cancelled() => {
                terminate_process_group(process_id);
                return Err(ToolError::Cancelled);
            },
            result = tokio::time::timeout(ctx.timeout, child.wait_with_output()) => match result {
                Ok(output) => output.map_err(|error| ToolError::Failed(error.to_string()))?,
                Err(_) => { terminate_process_group(process_id); return Err(ToolError::Timeout(ctx.timeout)); }
            },
        };
        let combined = format!(
            "exit: {}\nstdout:\n{}\nstderr:\n{}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into()),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(truncate(combined.into_bytes(), ctx.max_output_bytes))
    }

    async fn execute_secret_environment(
        &self,
        value: Value,
        ctx: &ToolContext,
        environment: crate::workflow::secrets::BoundEnvironment,
    ) -> Result<SecretShellOutcome, ToolError> {
        let args: Args = serde_json::from_value(value)
            .map_err(|_| ToolError::InvalidArguments("invalid one-shot shell arguments".into()))?;
        authorize_secret_command(&args, ctx)
            .await
            .map_err(private_shell_error)?;
        let mut command = Command::from(
            ctx.policy
                .process_command("sh", ctx.policy.workspace())
                .map_err(|e| ToolError::Failed(e.to_string()))?,
        );
        command
            .arg("-lc")
            .arg(&args.command)
            .current_dir(ctx.policy.workspace())
            .env_clear()
            .envs(&ctx.environment)
            .envs(environment.iter())
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        command.process_group(0);
        ctx.policy
            .isolate_process(command.as_std_mut(), ctx.policy.workspace())
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        let mut child = command
            .spawn()
            .map_err(|_| ToolError::Failed("private one-shot shell spawn failed".into()))?;
        let process_id = child.id();
        let status = tokio::select! {
            biased;
            _ = ctx.cancellation.cancelled() => {
                terminate_process_group(process_id);
                let _ = child.start_kill();
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1),child.wait()).await;
                return Err(ToolError::Cancelled);
            }
            result = tokio::time::timeout(ctx.timeout,child.wait()) => match result {
                Ok(result) => result.map_err(|_|ToolError::Failed("private one-shot shell wait failed".into()))?,
                Err(_) => {
                    terminate_process_group(process_id);
                    let _ = child.start_kill();
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(1),child.wait()).await;
                    return Err(ToolError::Timeout(ctx.timeout));
                }
            }
        };
        Ok(status.code().map_or(SecretShellOutcome::Signalled, |code| {
            SecretShellOutcome::Exited {
                code: i64::from(code),
            }
        }))
    }
}

fn private_shell_error(error: ToolError) -> ToolError {
    match error {
        ToolError::InvalidArguments(_) => {
            ToolError::InvalidArguments("invalid private one-shot shell arguments or limits".into())
        }
        ToolError::Denied(_) => {
            ToolError::Denied("private one-shot shell denied by local policy or approval".into())
        }
        ToolError::Failed(_) => ToolError::Failed(
            "private one-shot shell failed; cleanup observation must be checked".into(),
        ),
        other => other,
    }
}

fn check_private_policy(ctx: &ToolContext) -> Result<(), ToolError> {
    ctx.policy.check_current().map_err(|_| {
        ToolError::Denied(
            "private workflow binding policy changed; restart or explicitly reselect".into(),
        )
    })
}

async fn authorize_secret_command(args: &Args, ctx: &ToolContext) -> Result<(), ToolError> {
    check_private_policy(ctx)?;
    if ctx.cancellation.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    if args.workflow_secrets.is_empty() {
        return Err(ToolError::InvalidArguments(
            "private one-shot shell requires explicit workflow secret references".into(),
        ));
    }
    match ctx.policy.command(&args.command) {
        Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
        Decision::Ask(reason) => {
            let approval = ctx.approval("shell", &args.command, reason);
            let approved = tokio::select! {
                biased;
                _=ctx.cancellation.cancelled()=>return Err(ToolError::Cancelled),
                result=ctx.approver.approve(&approval)=>result,
            };
            approved.require_approved()?;
        }
        Decision::Allow => {}
    }
    check_private_policy(ctx)?;
    if ctx.cancellation.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    Ok(())
}

#[cfg(unix)]
fn terminate_process_group(process_id: Option<u32>) {
    if let Some(process_id) = process_id {
        // SAFETY: a negative PID targets only the process group created above.
        unsafe {
            libc::kill(-(process_id as i32), libc::SIGKILL);
        }
    }
}
#[cfg(not(unix))]
fn terminate_process_group(_: Option<u32>) {}
