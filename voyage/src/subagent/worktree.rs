use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub struct WorktreeManager {
    repository: PathBuf,
    root: PathBuf,
    environment: Option<std::collections::BTreeMap<String, String>>,
    policy: Option<std::sync::Arc<crate::policy::Policy>>,
}
#[derive(Clone, Debug)]
pub struct WorktreeLease {
    pub path: PathBuf,
    pub branch: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictReport {
    pub files: Vec<PathBuf>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegrationPlan {
    pub source: String,
    pub target: String,
    pub merge_base: String,
    pub conflicts: ConflictReport,
}
impl WorktreeManager {
    pub fn new(repository: PathBuf, root: PathBuf) -> Result<Self> {
        let repository = repository
            .canonicalize()
            .context("repository does not exist")?;
        anyhow::ensure!(
            repository.join(".git").exists() || repository.join("HEAD").exists(),
            "not a Git repository"
        );
        if root.exists() {
            anyhow::ensure!(
                root.canonicalize()? != repository,
                "worktree root cannot be repository"
            );
        }
        Ok(Self {
            repository,
            root,
            environment: None,
            policy: None,
        })
    }
    pub fn with_policy(mut self, policy: std::sync::Arc<crate::policy::Policy>) -> Self {
        self.policy = Some(policy);
        self
    }
    fn check_scope(&self, path: &Path, write: bool) -> Result<()> {
        if let Some(policy) = &self.policy {
            policy.check_current()?;
            policy.resolve_read(path)?;
            if write {
                policy.resolve_write(path)?;
            }
        }
        Ok(())
    }
    /// Use the already ceiling-filtered runtime environment for every Git subprocess.
    pub fn with_environment(
        mut self,
        environment: std::collections::BTreeMap<String, String>,
    ) -> Self {
        self.environment = Some(environment);
        self
    }
    pub fn planned_path(&self, name: &str) -> Result<PathBuf> {
        validate(name)?;
        anyhow::ensure!(
            !name.contains('/'),
            "worktree name must be one path component"
        );
        Ok(self.root.join(name))
    }
    /// Exact argv rendered for Policy's command parser; never passed to a shell.
    pub fn create_command(&self, name: &str, start_point: &str) -> Result<String> {
        let path = self.planned_path(name)?;
        validate(start_point)?;
        let branch = format!("agents/{name}");
        Ok(shell_words::join([
            "git",
            "worktree",
            "add",
            "-b",
            &branch,
            path.to_str().context("non-UTF8 path")?,
            start_point,
        ]))
    }
    pub fn create(&self, name: &str, start_point: &str) -> Result<WorktreeLease> {
        let path = self.planned_path(name)?;
        validate(start_point)?;
        if let Some(policy) = &self.policy {
            policy.check_current()?;
            policy.check_delegated_workspace(&path)?;
            let command = self.create_command(name, start_point)?;
            let arguments = shell_words::split(&command)?;
            policy
                .check_command_denials(&arguments.iter().map(String::as_str).collect::<Vec<_>>())?;
        }
        std::fs::create_dir_all(&self.root)?;
        anyhow::ensure!(
            !path.exists(),
            "worktree path already exists: {}",
            path.display()
        );
        let branch = format!("agents/{name}");
        git_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            [
                "worktree",
                "add",
                "-b",
                &branch,
                path.to_str().context("non-UTF8 path")?,
                start_point,
            ],
        )?;
        Ok(WorktreeLease { path, branch })
    }
    pub fn is_clean(&self, lease: &WorktreeLease) -> Result<bool> {
        self.check_scope(&lease.path, false)?;
        let output = git_output_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &lease.path,
            ["status", "--porcelain"],
        )?;
        Ok(output.trim().is_empty())
    }
    /// Commit all changes owned by one managed worktree without invoking a shell.
    pub fn commit(&self, lease: &WorktreeLease, message: &str) -> Result<String> {
        self.check_scope(&lease.path, true)?;
        anyhow::ensure!(
            lease.path.starts_with(&self.root),
            "refusing worktree outside managed root"
        );
        anyhow::ensure!(
            !message.trim().is_empty()
                && message.len() <= 200
                && !message.contains('\n')
                && !message.contains('\r'),
            "commit message must be one non-empty line of at most 200 bytes"
        );
        anyhow::ensure!(
            !self.is_clean(lease)?,
            "agent worktree has no changes to commit"
        );
        git_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &lease.path,
            ["add", "--all"],
        )?;
        git_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &lease.path,
            ["commit", "-m", message],
        )?;
        Ok(git_output_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &lease.path,
            ["rev-parse", "HEAD"],
        )?
        .trim()
        .to_owned())
    }
    pub fn remove(&self, lease: &WorktreeLease) -> Result<()> {
        self.check_scope(&lease.path, true)?;
        anyhow::ensure!(
            lease.path.starts_with(&self.root),
            "refusing worktree outside managed root"
        );
        anyhow::ensure!(
            self.is_clean(lease)?,
            "refusing to remove dirty agent worktree"
        );
        git_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            [
                "worktree",
                "remove",
                lease.path.to_str().context("non-UTF8 path")?,
            ],
        )
    }
    /// Detect files changed by both isolated branches since their merge base.
    pub fn conflicts(&self, left: &WorktreeLease, right: &WorktreeLease) -> Result<ConflictReport> {
        self.check_scope(&left.path, false)?;
        self.check_scope(&right.path, false)?;
        let base = git_output_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            ["merge-base", &left.branch, &right.branch],
        )?;
        let base = base.trim();
        let left_files = changed(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            base,
            &left.branch,
        )?;
        let right_files = changed(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            base,
            &right.branch,
        )?;
        Ok(ConflictReport {
            files: left_files.intersection(&right_files).cloned().collect(),
        })
    }
    pub fn plan_integration(&self, lease: &WorktreeLease, target: &str) -> Result<IntegrationPlan> {
        self.check_scope(&lease.path, false)?;
        validate(target)?;
        let base = git_output_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            ["merge-base", &lease.branch, target],
        )?
        .trim()
        .to_owned();
        let source_files = changed(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            &base,
            &lease.branch,
        )?;
        let target_files = changed(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            &base,
            target,
        )?;
        Ok(IntegrationPlan {
            source: lease.branch.clone(),
            target: target.into(),
            merge_base: base,
            conflicts: ConflictReport {
                files: source_files.intersection(&target_files).cloned().collect(),
            },
        })
    }
    pub fn integrate(&self, lease: &WorktreeLease, target: &str) -> Result<()> {
        self.check_scope(&lease.path, false)?;
        self.check_scope(&self.repository, true)?;
        anyhow::ensure!(
            self.is_clean(lease)?,
            "refusing to integrate dirty agent worktree"
        );
        anyhow::ensure!(
            git_output_env(
                self.environment.as_ref(),
                self.policy.as_deref(),
                &self.repository,
                ["status", "--porcelain"]
            )?
            .trim()
            .is_empty(),
            "refusing to integrate into dirty repository"
        );
        let current = git_output_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            ["branch", "--show-current"],
        )?;
        anyhow::ensure!(
            current.trim() == target,
            "target branch `{target}` is not checked out"
        );
        let plan = self.plan_integration(lease, target)?;
        anyhow::ensure!(
            plan.conflicts.files.is_empty(),
            "integration has overlapping files: {:?}",
            plan.conflicts.files
        );
        git_env(
            self.environment.as_ref(),
            self.policy.as_deref(),
            &self.repository,
            ["merge", "--no-ff", "--no-edit", &lease.branch],
        )
    }
}
fn changed(
    environment: Option<&std::collections::BTreeMap<String, String>>,
    policy: Option<&crate::policy::Policy>,
    repository: &Path,
    base: &str,
    branch: &str,
) -> Result<std::collections::BTreeSet<PathBuf>> {
    Ok(git_output_env(
        environment,
        policy,
        repository,
        ["diff", "--name-only", base, branch],
    )?
    .lines()
    .filter(|line| !line.is_empty())
    .map(PathBuf::from)
    .collect())
}
fn validate(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value.contains("..")
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_/".contains(&b))
    {
        bail!("unsafe Git reference/name `{value}`")
    }
    Ok(())
}
fn git_env<const N: usize>(
    environment: Option<&std::collections::BTreeMap<String, String>>,
    policy: Option<&crate::policy::Policy>,
    cwd: &Path,
    args: [&str; N],
) -> Result<()> {
    if let Some(policy) = policy {
        policy.check_current()?;
        policy.resolve_write(cwd)?;
    }
    let output = git_command(environment, policy, cwd, args)?.output()?;
    if !output.status.success() {
        bail!("git failed: {}", String::from_utf8_lossy(&output.stderr))
    }
    Ok(())
}
fn git_output_env<const N: usize>(
    environment: Option<&std::collections::BTreeMap<String, String>>,
    policy: Option<&crate::policy::Policy>,
    cwd: &Path,
    args: [&str; N],
) -> Result<String> {
    let output = git_command(environment, policy, cwd, args)?.output()?;
    if !output.status.success() {
        bail!("git failed: {}", String::from_utf8_lossy(&output.stderr))
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_command<const N: usize>(
    environment: Option<&std::collections::BTreeMap<String, String>>,
    policy: Option<&crate::policy::Policy>,
    cwd: &Path,
    args: [&str; N],
) -> Result<Command> {
    if let Some(policy) = policy {
        policy.check_current()?;
        policy.resolve_read(cwd)?;
        let arguments = std::iter::once("git").chain(args).collect::<Vec<_>>();
        policy.check_command_denials(&arguments)?;
    }
    let mut command = match policy {
        Some(policy) => policy.process_command("git", cwd)?,
        None => Command::new("git"),
    };
    command.args(args).current_dir(cwd);
    if let Some(environment) = environment {
        command.env_clear().envs(environment);
    }
    if let Some(policy) = policy {
        policy.isolate_process(&mut command, cwd)?;
    }
    Ok(command)
}
