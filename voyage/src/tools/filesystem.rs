use crate::{
    model::ToolDefinition,
    policy::Decision,
    tools::{Tool, ToolContext, ToolError, truncate},
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
        let digest = hex::encode(Sha256::digest(&bytes));
        Ok(format!(
            "sha256: {digest}\n{}",
            truncate(bytes, ctx.max_output_bytes)
        ))
    }
}

pub struct WriteFile;
#[derive(Deserialize)]
struct WriteArgs {
    path: PathBuf,
    content: String,
}

pub struct ApplyPatch;
#[derive(Deserialize)]
struct PatchArgs {
    path: PathBuf,
    patch: String,
    /// Required for existing files; prevents overwriting content changed since it was read.
    base_sha256: Option<String>,
}

#[async_trait]
impl Tool for ApplyPatch {
    fn definition(&self) -> ToolDefinition {
        definition(
            "apply_patch",
            "Atomically apply a unified diff to one UTF-8 file. Existing files require the SHA-256 returned by read_file, preventing stale writes.",
            json!({"type":"object","properties":{"path":{"type":"string"},"patch":{"type":"string"},"base_sha256":{"type":"string","pattern":"^[a-fA-F0-9]{64}$"}},"required":["path","patch"]}),
        )
    }
    async fn execute(&self, value: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: PatchArgs = parse(value)?;
        let path = ctx.policy.resolve_write(&args.path).map_err(denied)?;
        let exists = path.exists();
        let original = if exists {
            fs::read_to_string(&path).await.map_err(failed)?
        } else {
            String::new()
        };
        if exists {
            let expected = args.base_sha256.as_deref().ok_or_else(|| {
                ToolError::InvalidArguments(
                    "base_sha256 is required when patching an existing file".into(),
                )
            })?;
            let actual = hex::encode(Sha256::digest(original.as_bytes()));
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(ToolError::Failed(format!(
                    "file changed since it was read (expected {expected}, actual {actual})"
                )));
            }
        }
        match ctx.policy.write(&path, exists) {
            Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
            Decision::Ask(reason) => ctx
                .approver
                .approve(&ctx.approval(
                    "filesystem.patch",
                    path.display().to_string(),
                    reason.clone(),
                ))
                .await
                .require_approved()?,
            _ => {}
        }
        let patch = diffy::Patch::from_str(&args.patch)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid unified diff: {e}")))?;
        let updated = diffy::apply(&original, &patch)
            .map_err(|e| ToolError::Failed(format!("patch does not apply cleanly: {e}")))?;
        let parent = path
            .parent()
            .ok_or_else(|| ToolError::Failed("target has no parent directory".into()))?;
        fs::create_dir_all(parent).await.map_err(failed)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(failed)?;
        std::io::Write::write_all(&mut temporary, updated.as_bytes()).map_err(failed)?;
        temporary.as_file().sync_all().map_err(failed)?;
        // Revalidate after patch construction and approval so a concurrent writer is
        // not silently overwritten during a long attended approval.
        if exists {
            let current = fs::read(&path).await.map_err(failed)?;
            if Sha256::digest(&current) != Sha256::digest(original.as_bytes()) {
                return Err(ToolError::Failed(
                    "file changed while the patch was being prepared; retry from a fresh read"
                        .into(),
                ));
            }
        } else if path.exists() {
            return Err(ToolError::Failed(
                "target was created concurrently; refusing to replace it".into(),
            ));
        }
        temporary.persist(&path).map_err(failed)?;
        Ok(format!(
            "patched {} ({} -> {} bytes, sha256 {})",
            path.display(),
            original.len(),
            updated.len(),
            hex::encode(Sha256::digest(updated.as_bytes()))
        ))
    }
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
            Decision::Ask(reason) => ctx
                .approver
                .approve(&ctx.approval(
                    "filesystem.write",
                    path.display().to_string(),
                    reason.clone(),
                ))
                .await
                .require_approved()?,
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
            if ctx.cancellation.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let mut entries = fs::read_dir(&dir).await.map_err(failed)?;
            while let Some(entry) = entries.next_entry().await.map_err(failed)? {
                let path = entry.path();
                let file_type = entry.file_type().await.map_err(failed)?;
                let suffix = path.strip_prefix(&root).unwrap_or(&path);
                lines.push(format!(
                    "{}{}",
                    suffix.display(),
                    if file_type.is_dir() { "/" } else { "" }
                ));
                if args.recursive && file_type.is_dir() && lines.len() < 10_000 {
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
            .env_clear()
            .envs(&ctx.environment)
            .kill_on_drop(true);
        let output = tokio::select! {
            _ = ctx.cancellation.cancelled() => return Err(ToolError::Cancelled),
            result = tokio::time::timeout(ctx.timeout, command.output()) => result.map_err(|_| ToolError::Timeout(ctx.timeout))?.map_err(failed)?,
        };
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
