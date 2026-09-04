use crate::{
    model::ToolDefinition,
    policy::Decision,
    tools::{Tool, ToolContext, ToolError, truncate},
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio::fs;

pub struct ReadFile;
#[derive(Deserialize)]
struct PathArgs {
    path: PathBuf,
}

#[async_trait]
impl Tool for ReadFile {
    fn definition(&self) -> ToolDefinition {
        definition(
            "read_file",
            "Read a UTF-8 text file inside an allowed root.",
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        )
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: PathArgs = parse(args)?;
        let path = ctx.policy.resolve_read(&args.path).map_err(denied)?;
        let bytes = fs::read(path).await.map_err(failed)?;
        Ok(truncate(bytes, ctx.max_output_bytes))
    }
}

pub struct WriteFile;
#[derive(Deserialize)]
struct WriteArgs {
    path: PathBuf,
    content: String,
}
#[async_trait]
impl Tool for WriteFile {
    fn definition(&self) -> ToolDefinition {
        definition(
            "write_file",
            "Create or replace a UTF-8 text file inside an allowed writable root.",
            json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        )
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: WriteArgs = parse(args)?;
        let path = ctx.policy.resolve_write(&args.path).map_err(denied)?;
        match ctx.policy.write(&path, path.exists()) {
            Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
            Decision::Ask(reason) if !ctx.approver.approve(&reason).await => {
                return Err(ToolError::Denied("user declined approval".into()));
            }
            _ => {}
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(failed)?;
        }
        fs::write(&path, args.content.as_bytes())
            .await
            .map_err(failed)?;
        Ok(format!(
            "wrote {} bytes to {}",
            args.content.len(),
            path.display()
        ))
    }
}

pub struct ListDirectory;
#[derive(Deserialize)]
struct ListArgs {
    #[serde(default = "dot")]
    path: PathBuf,
    #[serde(default)]
    recursive: bool,
}
fn dot() -> PathBuf {
    ".".into()
}
#[async_trait]
impl Tool for ListDirectory {
    fn definition(&self) -> ToolDefinition {
        definition(
            "list_directory",
            "List files and directories in an allowed root.",
            json!({"type":"object","properties":{"path":{"type":"string"},"recursive":{"type":"boolean"}}}),
        )
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: ListArgs = parse(args)?;
        let root = ctx.policy.resolve_read(&args.path).map_err(denied)?;
        let mut pending = vec![root.clone()];
        let mut lines = Vec::new();
        while let Some(dir) = pending.pop() {
            let mut entries = fs::read_dir(&dir).await.map_err(failed)?;
            while let Some(entry) = entries.next_entry().await.map_err(failed)? {
                let path = entry.path();
                let suffix = path.strip_prefix(&root).unwrap_or(&path);
                lines.push(format!(
                    "{}{}",
                    suffix.display(),
                    if path.is_dir() { "/" } else { "" }
                ));
                if args.recursive && path.is_dir() && lines.len() < 10_000 {
                    pending.push(path);
                }
            }
        }
        lines.sort();
        Ok(truncate(
            lines.join("\n").into_bytes(),
            ctx.max_output_bytes,
        ))
    }
}

pub struct SearchFiles;
#[derive(Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default = "dot")]
    path: PathBuf,
    #[serde(default)]
    glob: Option<String>,
}
#[async_trait]
impl Tool for SearchFiles {
    fn definition(&self) -> ToolDefinition {
        definition(
            "search_files",
            "Search file contents with ripgrep. Query is a regular expression.",
            json!({"type":"object","properties":{"query":{"type":"string"},"path":{"type":"string"},"glob":{"type":"string"}},"required":["query"]}),
        )
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: SearchArgs = parse(args)?;
        let path = ctx.policy.resolve_read(&args.path).map_err(denied)?;
        let mut command = tokio::process::Command::new("rg");
        command.args(["--line-number", "--no-heading", "--color", "never"]);
        if let Some(glob) = args.glob {
            command.args(["--glob", &glob]);
        }
        command
            .arg("--")
            .arg(args.query)
            .arg(path)
            .kill_on_drop(true);
        let output = tokio::time::timeout(ctx.timeout, command.output())
            .await
            .map_err(|_| ToolError::Failed("search timed out".into()))?
            .map_err(failed)?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(ToolError::Failed(
                String::from_utf8_lossy(&output.stderr).into(),
            ));
        }
        Ok(truncate(output.stdout, ctx.max_output_bytes))
    }
}

fn definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema,
    }
}
fn parse<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, ToolError> {
    serde_json::from_value(value).map_err(|e| ToolError::InvalidArguments(e.to_string()))
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}
fn denied(error: impl std::fmt::Display) -> ToolError {
    ToolError::Denied(error.to_string())
}
