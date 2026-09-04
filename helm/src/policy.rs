use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};

use crate::config::{ApprovalMode, Config};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask(String),
    Deny(String),
}

#[derive(Clone, Debug)]
pub struct Policy {
    workspace: PathBuf,
    readable: Vec<PathBuf>,
    writable: Vec<PathBuf>,
    deny_commands: Vec<String>,
    mode: ApprovalMode,
}

impl Policy {
    pub fn new(config: &Config, workspace: PathBuf) -> Result<Self> {
        let mut readable = vec![workspace.clone()];
        readable.extend(canonical_roots(&config.allow_read)?);
        let mut writable = vec![workspace.clone()];
        writable.extend(canonical_roots(&config.allow_write)?);
        Ok(Self {
            workspace,
            readable,
            writable,
            deny_commands: config.deny_commands.clone(),
            mode: config.approval.clone(),
        })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
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

    pub fn command(&self, command: &str) -> Decision {
        let parsed = shell_words::split(command).unwrap_or_default();
        if parsed.is_empty() {
            return Decision::Deny("command could not be parsed safely".into());
        }
        let denied = parsed.iter().find_map(|word| {
            let executable = Path::new(word).file_name()?.to_str()?;
            self.deny_commands
                .iter()
                .any(|denied| denied == executable)
                .then_some(executable)
        });
        if let Some(executable) = denied {
            return Decision::Deny(format!("command `{executable}` is denied by policy"));
        }
        let risky = looks_risky(command, &parsed);
        match (&self.mode, risky) {
            (ApprovalMode::Always, _) => Decision::Ask(format!("run shell command: {command}")),
            (ApprovalMode::OnRisk, true) => {
                Decision::Ask(format!("run potentially consequential command: {command}"))
            }
            (ApprovalMode::Never, _) | (ApprovalMode::OnRisk, false) => Decision::Allow,
        }
    }

    pub fn write(&self, path: &Path, replacing: bool) -> Decision {
        match (&self.mode, replacing) {
            (ApprovalMode::Always, _) => Decision::Ask(format!("write {}", path.display())),
            (ApprovalMode::OnRisk, true) => {
                Decision::Ask(format!("replace existing file {}", path.display()))
            }
            _ => Decision::Allow,
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
        "git",
    ];
    let contains_mutating_program = words.iter().any(|word| {
        Path::new(word)
            .file_name()
            .and_then(|p| p.to_str())
            .is_some_and(|name| MUTATING.contains(&name))
    });
    contains_mutating_program
        || command.contains(">")
        || command.contains("sudo ")
        || command.contains("--force")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_risk() {
        assert!(looks_risky("rm -rf x", &["rm".into()]));
        assert!(!looks_risky("cargo test", &["cargo".into(), "test".into()]));
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
}
