use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};

use crate::config::{AccessMode, Config};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask(String),
    Deny(String),
}

/// Optional foreground authority checked at each new dispatch boundary.
/// Implementations must read current state; a stored positive check is not a grant.
pub trait ExecutionAuthority: std::fmt::Debug + Send + Sync {
    fn check(&self) -> Result<()>;
}

#[derive(Clone, Debug)]
pub struct Policy {
    execution_authority: Option<std::sync::Arc<dyn ExecutionAuthority>>,
    workspace: PathBuf,
    readable: Vec<PathBuf>,
    writable: Vec<PathBuf>,
    deny_commands: Vec<String>,
    mode: AccessMode,
    snapshot: crate::runtime_policy::Snapshot,
}

impl Policy {
    pub fn new(config: &Config, workspace: PathBuf) -> Result<Self> {
        Ok(crate::runtime_policy::RuntimePolicy::resolve(config, &workspace)?.into_policy())
    }
    pub(crate) fn from_runtime(snapshot: crate::runtime_policy::Snapshot) -> Self {
        let effective = &snapshot.effective;
        Self {
            execution_authority: None,
            workspace: effective.workspace().path().into(),
            readable: effective.rules().read_roots.clone(),
            writable: effective.rules().write_roots.clone(),
            deny_commands: effective.rules().deny_commands.clone(),
            mode: effective.rules().access,
            snapshot,
        }
    }
    /// New turns refuse stale policy; existing effects are not instantaneously revoked.
    pub fn check_current(&self) -> Result<()> {
        self.check_execution_authority()?;
        self.snapshot.check_current()
    }
    pub fn with_execution_authority(
        mut self,
        authority: std::sync::Arc<dyn ExecutionAuthority>,
    ) -> Self {
        self.execution_authority = Some(authority);
        self
    }
    pub fn check_execution_authority(&self) -> Result<()> {
        if let Some(authority) = &self.execution_authority {
            authority.check()?;
        }
        Ok(())
    }
    pub(crate) fn inherit_execution_authority(&mut self, parent: &Self) {
        self.execution_authority = parent.execution_authority.clone();
    }
    pub(crate) fn inherit_profile_freshness(&mut self, parent: &Policy) {
        self.snapshot.ancestor_selection = parent.snapshot.inherited_selection();
        self.snapshot.ancestor_defaults = parent
            .snapshot
            .defaults
            .clone()
            .or_else(|| parent.snapshot.ancestor_defaults.clone());
    }
    pub fn ceiling_present(&self) -> bool {
        self.snapshot.effective.ceiling_digest().is_some()
    }
    pub fn check_delegated_workspace(&self, path: &Path) -> Result<()> {
        self.snapshot.verify_workspace()?;
        let (ancestor, suffix) = existing_ancestor(path)?;
        let mut resolved = ancestor.canonicalize()?;
        for part in suffix {
            resolved.push(part);
        }
        anyhow::ensure!(
            within_any(&resolved, &self.readable) && within_any(&resolved, &self.writable),
            "child/worktree workspace requires explicit parent read and write root delegation"
        );
        Ok(())
    }
    pub(crate) fn limit_child_config(&self, config: &mut Config, workspace: &Path) -> Result<()> {
        self.check_delegated_workspace(workspace)?;
        anyhow::ensure!(
            canonical_roots(&config.allow_read)?
                .iter()
                .all(|p| within_any(p, &self.readable))
                && canonical_roots(&config.allow_write)?
                    .iter()
                    .all(|p| within_any(p, &self.writable)),
            "child roots exceed captured parent authority"
        );
        let rank = |m| match m {
            AccessMode::ReadOnly => 0,
            AccessMode::Approval => 1,
            AccessMode::Unrestricted => 2,
        };
        if rank(config.access_mode()) > rank(self.mode) {
            config.access = Some(self.mode);
        }
        crate::runtime_policy::restrictive_unattended(
            &mut config.unattended_approval,
            &self.snapshot.effective.rules().unattended,
        );
        config.deny_commands.extend(self.deny_commands.clone());
        config.deny_commands.sort();
        config.deny_commands.dedup();
        config
            .inherit_env
            .retain(|name| self.snapshot.effective.rules().inherit_env.contains(name));
        if let Some(allowed) = self.snapshot.effective.environment_ceiling() {
            crate::runtime_policy::restrict_environment(config, allowed);
        }
        Ok(())
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn access_mode(&self) -> AccessMode {
        self.mode
    }

    pub fn resolve_read(&self, path: &Path) -> Result<PathBuf> {
        let path = self.absolute(path);
        let resolved = path
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("cannot access {}: {e}", path.display()))?;
        if !within_any(&resolved, &self.readable) {
            bail!("read outside allowed roots: {}", resolved.display());
        }
        Ok(resolved)
    }

    pub fn resolve_write(&self, path: &Path) -> Result<PathBuf> {
        let path = self.absolute(path);
        let (ancestor, suffix) = existing_ancestor(&path)?;
        let resolved_ancestor = ancestor.canonicalize()?;
        if !within_any(&resolved_ancestor, &self.writable) {
            bail!("write outside allowed roots: {}", path.display());
        }
        let mut resolved = resolved_ancestor;
        for part in suffix {
            resolved.push(part);
        }
        Ok(resolved)
    }

    /// Deny-list check for exact argv of internal operations. This does not grant
    /// access or replace the operation's separate mode/approval checks.
    pub fn check_command_denials(&self, arguments: &[&str]) -> Result<()> {
        for argument in arguments {
            if let Some(name) = Path::new(argument)
                .file_name()
                .and_then(|name| name.to_str())
                && self.deny_commands.iter().any(|denied| denied == name)
            {
                bail!("command `{name}` is denied by policy");
            }
        }
        Ok(())
    }

    pub fn command(&self, command: &str) -> Decision {
        let parsed = shell_words::split(command).unwrap_or_default();
        if parsed.is_empty() {
            return Decision::Deny("command could not be parsed safely".into());
        }
        if let Err(error) =
            self.check_command_denials(&parsed.iter().map(String::as_str).collect::<Vec<_>>())
        {
            return Decision::Deny(error.to_string());
        }
        let risky = looks_risky(command, &parsed);
        match (self.mode, risky) {
            (AccessMode::ReadOnly, _) => {
                Decision::Deny("commands are disabled in read-only access mode".into())
            }
            (AccessMode::Approval, true) => {
                Decision::Ask(format!("run potentially consequential command: {command}"))
            }
            (AccessMode::Approval, false) | (AccessMode::Unrestricted, _) => Decision::Allow,
        }
    }

    pub fn write(&self, path: &Path, _replacing: bool) -> Decision {
        match self.mode {
            AccessMode::ReadOnly => {
                Decision::Deny("writes are disabled in read-only access mode".into())
            }
            AccessMode::Approval => Decision::Ask(format!("write {}", path.display())),
            AccessMode::Unrestricted => Decision::Allow,
        }
    }

    /// MCP tools do not expose a trustworthy read/write classification, so the
    /// access mode must treat each invocation conservatively.
    pub fn external_tool(&self, name: &str) -> Decision {
        match self.mode {
            AccessMode::ReadOnly => Decision::Deny(format!(
                "external tool `{name}` is disabled in read-only access mode"
            )),
            AccessMode::Approval => Decision::Ask(format!("run external MCP tool `{name}`")),
            AccessMode::Unrestricted => Decision::Allow,
        }
    }

    fn absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            normalize(path)
        } else {
            normalize(&self.workspace.join(path))
        }
    }
}

fn canonical_roots(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    paths
        .iter()
        .map(|p| p.canonicalize().map_err(Into::into))
        .collect()
}

fn within_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn existing_ancestor(path: &Path) -> Result<(PathBuf, Vec<std::ffi::OsString>)> {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::new();
    while !cursor.exists() {
        let name = cursor
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("no existing ancestor for {}", path.display()))?;
        suffix.push(name.to_owned());
        cursor.pop();
    }
    suffix.reverse();
    Ok((cursor, suffix))
}

fn looks_risky(command: &str, words: &[String]) -> bool {
    const MUTATING: &[&str] = &[
        "rm",
        "mv",
        "cp",
        "dd",
        "chmod",
        "chown",
        "kill",
        "pkill",
        "systemctl",
        "service",
        "mount",
        "umount",
        "docker",
        "podman",
        "kubectl",
        "terraform",
    ];
    let contains_mutating_program = words.iter().any(|word| {
        Path::new(word)
            .file_name()
            .and_then(|p| p.to_str())
            .is_some_and(|name| MUTATING.contains(&name))
    });
    contains_mutating_program
        || command.contains('>')
        || command.contains("sudo ")
        || command.contains("--force")
        || !confidently_read_only(command, words)
}

/// Shell is not sandboxed, so approval mode only bypasses a prompt for a
/// deliberately small set of inspection commands. Anything ambiguous asks.
fn confidently_read_only(command: &str, words: &[String]) -> bool {
    if command
        .chars()
        .any(|character| matches!(character, '\n' | ';' | '`'))
        || command.contains("$((")
        || command.contains("$(")
        || command.contains("&&")
        || command.contains("||")
        || words.is_empty()
    {
        return false;
    }
    let executable = Path::new(&words[0])
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    match executable {
        "pwd" | "ls" | "cat" | "head" | "tail" | "wc" | "stat" | "file" | "du" | "df" | "ps"
        | "printenv" | "whoami" | "id" | "uname" | "which" | "true" | "false" => true,
        "env" => words.len() == 1,
        "rg" | "grep" => !words.iter().any(|word| word.starts_with("--pre")),
        "find" => !words.iter().any(|word| {
            matches!(
                word.as_str(),
                "-delete"
                    | "-exec"
                    | "-execdir"
                    | "-ok"
                    | "-okdir"
                    | "-fls"
                    | "-fprint"
                    | "-fprint0"
                    | "-fprintf"
            )
        }),
        "sed" => safe_sed(words),
        "git" => words.get(1).is_some_and(|subcommand| {
            matches!(
                subcommand.as_str(),
                "status" | "diff" | "log" | "show" | "rev-parse" | "ls-files" | "grep"
            ) || (subcommand == "branch"
                && words.iter().skip(2).all(|word| word == "--show-current"))
        }),
        _ => false,
    }
}

fn safe_sed(words: &[String]) -> bool {
    let Some(script) = words.iter().skip(1).find(|word| !word.starts_with('-')) else {
        return false;
    };
    script.chars().all(|character| {
        character.is_ascii_digit() || matches!(character, ',' | '$' | 'p' | 'q' | 'd' | ' ' | '\t')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_risk() {
        assert!(looks_risky("rm -rf x", &["rm".into()]));
        assert!(!looks_risky(
            "sed -n 1,20p file",
            &["sed".into(), "-n".into(), "1,20p".into(), "file".into()]
        ));
        assert!(looks_risky(
            "python -c pass",
            &["python".into(), "-c".into(), "pass".into()]
        ));
        assert!(looks_risky(
            "find . -delete",
            &["find".into(), ".".into(), "-delete".into()]
        ));
    }
    #[test]
    fn normalizes_parent() {
        assert_eq!(normalize(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
    }
    #[test]
    fn deny_list_cannot_be_bypassed_with_a_command_chain() {
        let directory = tempfile::tempdir().unwrap();
        let config = Config::default();
        let policy = Policy::new(&config, directory.path().to_owned()).unwrap();
        assert!(matches!(
            policy.command("echo ok; shutdown now"),
            Decision::Deny(_)
        ));
        assert!(matches!(policy.command("'unterminated"), Decision::Deny(_)));
    }

    #[test]
    fn access_modes_have_distinct_authority() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("new.txt");

        let mut config = Config {
            access: Some(AccessMode::ReadOnly),
            ..Config::default()
        };
        let read_only = Policy::new(&config, directory.path().to_owned()).unwrap();
        assert!(matches!(read_only.command("pwd"), Decision::Deny(_)));
        assert!(matches!(read_only.write(&target, false), Decision::Deny(_)));

        config.access = Some(AccessMode::Approval);
        let approval = Policy::new(&config, directory.path().to_owned()).unwrap();
        assert_eq!(approval.command("sed -n 1,5p file"), Decision::Allow);
        assert_eq!(approval.command("git status --short"), Decision::Allow);
        assert!(matches!(approval.command("rm file"), Decision::Ask(_)));
        assert!(matches!(approval.write(&target, false), Decision::Ask(_)));
        assert!(matches!(
            approval.external_tool("mcp_example_change"),
            Decision::Ask(_)
        ));

        config.access = Some(AccessMode::Unrestricted);
        let unrestricted = Policy::new(&config, directory.path().to_owned()).unwrap();
        assert_eq!(unrestricted.command("rm file"), Decision::Allow);
        assert_eq!(unrestricted.write(&target, false), Decision::Allow);
        assert_eq!(
            unrestricted.external_tool("mcp_example_change"),
            Decision::Allow
        );
        assert!(matches!(
            unrestricted.command("shutdown now"),
            Decision::Deny(_)
        ));
    }
}
