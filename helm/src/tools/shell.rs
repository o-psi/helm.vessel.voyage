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
#[derive(Deserialize)]
struct Args {
    command: String,
}

#[async_trait]
impl Tool for Shell {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "shell".into(),
            description:
                "Run a shell command in the workspace. Returns exit status, stdout, and stderr."
                    .into(),
            input_schema: json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
        }
    }
    async fn execute(&self, value: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: Args = serde_json::from_value(value)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        match ctx.policy.command(&args.command) {
            Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
            Decision::Ask(reason)
                if !ctx
                    .approver
                    .approve(&ctx.approval("shell", &args.command, reason.clone()))
                    .await
                    .approved() =>
            {
                return Err(ToolError::Denied("user declined approval".into()));
            }
            _ => {}
        }
        let mut command = Command::new("sh");
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
