use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub struct WorktreeManager {
    repository: PathBuf,
    root: PathBuf,
}
#[derive(Clone, Debug)]
pub struct WorktreeLease {
    pub path: PathBuf,
    pub branch: String,
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
        Ok(Self { repository, root })
    }
    pub fn create(&self, name: &str, start_point: &str) -> Result<WorktreeLease> {
        validate(name)?;
        validate(start_point)?;
        std::fs::create_dir_all(&self.root)?;
        let path = self.root.join(name);
        anyhow::ensure!(
            !path.exists(),
            "worktree path already exists: {}",
            path.display()
        );
        let branch = format!("agents/{name}");
        git(
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
        let output = git_output(&lease.path, ["status", "--porcelain"])?;
        Ok(output.trim().is_empty())
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
        git(
            &self.repository,
            [
                "worktree",
                "remove",
                lease.path.to_str().context("non-UTF8 path")?,
            ],
        )
    }
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
fn git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<()> {
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        bail!("git failed: {}", String::from_utf8_lossy(&output.stderr))
    }
    Ok(())
}
fn git_output<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        bail!("git failed: {}", String::from_utf8_lossy(&output.stderr))
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
        manager.remove(&lease).unwrap();
        assert!(!lease.path.exists());
    }
}
