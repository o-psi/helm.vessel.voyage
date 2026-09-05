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
        let arguments = std::iter::once("git").chain(args).collect::<Vec<_>>();
        policy.check_command_denials(&arguments)?;
    }
    let mut command = Command::new("git");
    command.args(args).current_dir(cwd);
    if let Some(environment) = environment {
        command.env_clear().envs(environment);
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(cwd: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(cwd)
                .status()
                .unwrap()
                .success()
        );
    }
    #[test]
    fn planned_destination_rejects_non_components_without_creating_directories() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".git")).unwrap();
        let target = root.path().join("absent");
        let manager = WorktreeManager::new(root.path().into(), target.clone()).unwrap();
        for name in ["/absolute", "a/b", "../escape", "-flag", "", "a\\b"] {
            assert!(manager.planned_path(name).is_err(), "{name}");
        }
        assert_eq!(
            manager.planned_path("child-1").unwrap(),
            target.join("child-1")
        );
        assert!(!target.exists());
    }
    #[test]
    fn protected_git_environment_explicitly_clears_ambient_names() {
        let environment =
            std::collections::BTreeMap::from([("ONLY".to_string(), "value".to_string())]);
        let command = git_command(Some(&environment), None, Path::new("."), ["status"]).unwrap();
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            vec![(
                std::ffi::OsStr::new("ONLY"),
                Some(std::ffi::OsStr::new("value"))
            )]
        );
        // Exercise the same env-clear builder with a deterministic local executable.
        let mut command =
            git_command(Some(&environment), None, Path::new("."), ["--version"]).unwrap();
        assert!(command.output().unwrap().status.success());
    }
    #[test]
    fn exact_argv_denials_cover_internal_reads_and_all_git_mutation_commands() {
        let directory = tempfile::tempdir().unwrap();
        for denied in [
            "git",
            "status",
            "diff",
            "merge-base",
            "add",
            "commit",
            "merge",
            "remove",
        ] {
            let config = crate::Config {
                deny_commands: vec![denied.into()],
                ..crate::Config::default()
            };
            let policy = crate::policy::Policy::new(&config, directory.path().into()).unwrap();
            let args = if denied == "git" {
                ["status", "--porcelain"]
            } else {
                [denied, "--fixture"]
            };
            let error = git_output_env(None, Some(&policy), directory.path(), args).unwrap_err();
            assert!(
                error.to_string().contains("denied by policy"),
                "{denied}: {error}"
            );
        }
        // Internal inspection does not inherit the blanket ReadOnly shell denial.
        let config = crate::Config {
            access: Some(crate::config::AccessMode::ReadOnly),
            ..crate::Config::default()
        };
        let policy = crate::policy::Policy::new(&config, directory.path().into()).unwrap();
        assert!(matches!(
            policy.command("git --version"),
            crate::policy::Decision::Deny(_)
        ));
        assert!(
            git_output_env(None, Some(&policy), directory.path(), ["--version"])
                .unwrap()
                .contains("git version")
        );
    }
    #[test]
    fn dirty_worktree_is_never_removed() {
        let repo = tempfile::tempdir().unwrap();
        run(repo.path(), &["init", "-q"]);
        run(
            repo.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        run(repo.path(), &["config", "user.name", "Test"]);
        std::fs::write(repo.path().join("tracked"), "base").unwrap();
        run(repo.path(), &["add", "tracked"]);
        run(repo.path(), &["commit", "-qm", "base"]);
        let root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(repo.path().into(), root.path().join("agents")).unwrap();
        let lease = manager.create("child", "HEAD").unwrap();
        std::fs::write(lease.path.join("tracked"), "dirty").unwrap();
        assert!(!manager.is_clean(&lease).unwrap());
        assert!(manager.remove(&lease).is_err());
        assert!(lease.path.exists());
        std::fs::write(lease.path.join("tracked"), "base").unwrap();
        assert!(manager.is_clean(&lease).unwrap());
        std::fs::write(lease.path.join("tracked"), "child").unwrap();
        let commit = manager.commit(&lease, "child").unwrap();
        assert_eq!(commit.len(), 40);
        assert!(manager.is_clean(&lease).unwrap());
        std::fs::write(repo.path().join("tracked"), "parent").unwrap();
        run(repo.path(), &["add", "tracked"]);
        run(repo.path(), &["commit", "-qm", "parent"]);
        let target = git_output_env(None, None, repo.path(), ["branch", "--show-current"]).unwrap();
        let plan = manager.plan_integration(&lease, target.trim()).unwrap();
        assert_eq!(plan.conflicts.files, vec![PathBuf::from("tracked")]);
        assert!(manager.integrate(&lease, target.trim()).is_err());
        manager.remove(&lease).unwrap();
        assert!(!lease.path.exists());
    }
}
