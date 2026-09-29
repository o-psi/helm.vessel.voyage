//! Bounded read of Git changes under the executing Voyage's own policy.
//! This never starts an agent turn or accepts a shell command from Helm.
use crate::policy::Policy;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};
use voyage_protocol::process::WorkspaceChangeScope;

const MAX_OUTPUT: usize = 64 * 1024;

pub(super) async fn read(
    workspace: &Path,
    policy: &Policy,
    scope: WorkspaceChangeScope,
    path: Option<&str>,
) -> Result<Value> {
    let workspace = policy.resolve_read(workspace)?;
    ensure!(workspace.is_dir(), "workspace unavailable");
    let dot_git = workspace.join(".git");
    let git_metadata = std::fs::symlink_metadata(&dot_git)
        .map_err(|_| anyhow::anyhow!("workspace Git repository unavailable"))?;
    ensure!(
        !git_metadata.file_type().is_symlink(),
        "workspace Git metadata must not be a symlink"
    );
    let git_directory = if git_metadata.is_dir() {
        dot_git
    } else {
        ensure!(
            git_metadata.is_file() && git_metadata.len() <= 4096,
            "invalid Git worktree pointer"
        );
        let pointer = std::fs::read_to_string(&dot_git)?;
        let target = pointer
            .strip_prefix("gitdir: ")
            .map(str::trim_end)
            .ok_or_else(|| anyhow::anyhow!("invalid Git worktree pointer"))?;
        ensure!(
            !target.is_empty() && !target.contains('\n'),
            "invalid Git worktree pointer"
        );
        workspace.join(target)
    };
    let git_directory = policy.resolve_read(&git_directory)?;
    ensure!(git_directory.is_dir(), "Git metadata unavailable");
    let common = git_directory.join("commondir");
    let common = if common.exists() {
        ensure!(
            std::fs::symlink_metadata(&common)?.len() <= 4096,
            "invalid Git common directory"
        );
        let value = std::fs::read_to_string(&common)?;
        let relative = value.trim_end();
        ensure!(
            value.len() <= 4096 && !relative.is_empty() && !relative.contains('\n'),
            "invalid Git common directory"
        );
        policy.resolve_read(&git_directory.join(relative))?
    } else {
        git_directory
    };
    ensure!(common.is_dir(), "Git common directory unavailable");
    ensure!(
        matches!(
            std::fs::symlink_metadata(common.join("objects/info/alternates")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        ),
        "external Git object stores are unavailable to Changes review"
    );
    let path = path.unwrap_or(".");
    ensure!(
        !path.is_empty()
            && path.len() <= 4096
            && !path.starts_with('/')
            && !path.contains('\\')
            && !path.contains(':')
            && !path.chars().any(char::is_control)
            && !path.split('/').any(|part| part == ".."),
        "invalid workspace-relative path"
    );

    let mut command = Command::new("git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .current_dir(&workspace)
        .arg("--no-pager")
        .arg("--no-optional-locks")
        .arg(format!("--work-tree={}", workspace.display()))
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "color.ui=false",
            "-c",
            "core.quotePath=true",
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "submodule.recurse=false",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let section = match scope {
        WorkspaceChangeScope::Status => {
            command.args([
                "status",
                "--porcelain=v1",
                "-z",
                "--no-renames",
                "--untracked-files=all",
                "--ignore-submodules=all",
            ]);
            "status"
        }
        WorkspaceChangeScope::Unstaged => {
            command.args([
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-renames",
                "--ignore-submodules=all",
            ]);
            "unstaged"
        }
        WorkspaceChangeScope::Staged => {
            command.args([
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-renames",
                "--ignore-submodules=all",
            ]);
            "staged"
        }
    };
    command.args(["--", path]);
    // Honor the same required OS isolation as agent tools. A read permission
    // alone must not make an unsandboxed subprocess when isolation is required.
    policy.isolate_process(command.as_std_mut(), &workspace)?;
    command.kill_on_drop(true);
    let observe = async {
        let mut child = command
            .spawn()
            .map_err(|_| anyhow::anyhow!("Git changes unavailable"))?;
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Git output unavailable"))?
            .take((MAX_OUTPUT + 1) as u64)
            .read_to_end(&mut output)
            .await?;
        let truncated = output.len() > MAX_OUTPUT;
        if truncated {
            let _ = child.kill().await;
            let _ = child.wait().await;
        } else {
            ensure!(child.wait().await?.success(), "Git changes unavailable");
        }
        output.truncate(MAX_OUTPUT);
        Ok::<_, anyhow::Error>((output, truncated))
    };
    let (output, truncated) = tokio::time::timeout(Duration::from_secs(4), observe)
        .await
        .map_err(|_| anyhow::anyhow!("Git changes timed out"))??;
    policy.check_current()?;
    let complete = if truncated && matches!(scope, WorkspaceChangeScope::Status) {
        output
            .iter()
            .rposition(|byte| *byte == 0)
            .map_or(0, |index| index + 1)
    } else {
        output.len()
    };
    let mut output = output[..complete].to_vec();
    if truncated && !matches!(scope, WorkspaceChangeScope::Status) {
        while !output.is_empty()
            && std::str::from_utf8(&output).is_err_and(|error| error.error_len().is_none())
        {
            output.pop();
        }
    }
    let text = String::from_utf8(output).map_err(|_| anyhow::anyhow!("Git output is not UTF-8"))?;
    Ok(json!({
        "scope": section,
        "path": path,
        "text": text,
        "truncated": truncated,
        "observed_at_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command as SyncCommand};

    fn git(root: &Path, args: &[&str]) {
        assert!(
            SyncCommand::new("git")
                .current_dir(root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }

    #[tokio::test]
    async fn fixed_read_observes_status_and_bounded_diffs_without_mutation() {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "-q"]);
        fs::write(root.path().join("tracked.txt"), "original\n").unwrap();
        git(root.path(), &["add", "tracked.txt"]);
        git(
            root.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "base",
            ],
        );
        fs::write(root.path().join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.path().join("new.txt"), "unsent\n").unwrap();
        let policy = Policy::new(&crate::Config::default(), root.path().to_path_buf()).unwrap();

        let status = read(root.path(), &policy, WorkspaceChangeScope::Status, None)
            .await
            .unwrap();
        assert_eq!(status["scope"], "status");
        assert_eq!(status["truncated"], false);
        assert!(
            status["text"]
                .as_str()
                .unwrap()
                .contains(" M tracked.txt\0")
        );
        assert!(status["text"].as_str().unwrap().contains("?? new.txt\0"));
        let diff = read(
            root.path(),
            &policy,
            WorkspaceChangeScope::Unstaged,
            Some("tracked.txt"),
        )
        .await
        .unwrap();
        assert!(diff["text"].as_str().unwrap().contains("+changed"));
        assert!(!diff["text"].as_str().unwrap().contains("new.txt"));

        git(root.path(), &["add", "tracked.txt"]);
        let staged = read(
            root.path(),
            &policy,
            WorkspaceChangeScope::Staged,
            Some("tracked.txt"),
        )
        .await
        .unwrap();
        assert!(staged["text"].as_str().unwrap().contains("+changed"));
        assert!(
            read(
                root.path(),
                &policy,
                WorkspaceChangeScope::Status,
                Some("../outside")
            )
            .await
            .is_err()
        );
        assert!(
            read(
                root.path(),
                &policy,
                WorkspaceChangeScope::Status,
                Some(":(glob)*")
            )
            .await
            .is_err()
        );

        fs::write(
            root.path().join("tracked.txt"),
            "x".repeat(MAX_OUTPUT + 2000),
        )
        .unwrap();
        let large = read(
            root.path(),
            &policy,
            WorkspaceChangeScope::Unstaged,
            Some("tracked.txt"),
        )
        .await
        .unwrap();
        assert_eq!(large["truncated"], true);
        assert!(large["text"].as_str().unwrap().len() <= MAX_OUTPUT);
    }

    #[tokio::test]
    async fn worktree_metadata_requires_explicit_read_root() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("main");
        fs::create_dir(&main).unwrap();
        git(&main, &["init", "-q"]);
        fs::write(main.join("tracked.txt"), "before\n").unwrap();
        git(&main, &["add", "tracked.txt"]);
        git(
            &main,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "base",
            ],
        );
        let branch = root.path().join("branch");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "test-branch",
                branch.to_str().unwrap(),
            ],
        );
        fs::write(branch.join("tracked.txt"), "after\n").unwrap();
        let restricted = Policy::new(&crate::Config::default(), branch.clone()).unwrap();
        assert!(
            read(&branch, &restricted, WorkspaceChangeScope::Status, None)
                .await
                .is_err()
        );
        let approved = Policy::new(&crate::Config::default(), root.path().to_path_buf()).unwrap();
        let status = read(&branch, &approved, WorkspaceChangeScope::Status, None)
            .await
            .unwrap();
        assert!(
            status["text"]
                .as_str()
                .unwrap()
                .contains(" M tracked.txt\0")
        );
    }
}
