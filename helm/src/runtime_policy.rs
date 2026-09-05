//! Mandatory runtime ceiling resolution; effective Config clones are never persisted.
use crate::{
    Config,
    config::UnattendedApprovalMode,
    policy::Policy,
    policy_profile::{self, EffectivePolicy, Rules},
};
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug)]
pub(crate) enum Source {
    System,
    #[cfg(all(test, target_os = "linux"))]
    Test(PathBuf),
}
impl Source {
    fn resolve(&self, workspace: &Path, base: &Rules) -> Result<EffectivePolicy> {
        match self {
            Self::System => Ok(policy_profile::resolve_runtime_current(workspace, base)?),
            #[cfg(all(test, target_os = "linux"))]
            Self::Test(root) => Ok(policy_profile::resolve_test_source(workspace, base, root)?),
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    source: Source,
    base: Rules,
    pub(crate) effective: EffectivePolicy,
}
impl Snapshot {
    pub(crate) fn check_current(&self) -> Result<()> {
        let current = self
            .source
            .resolve(self.effective.workspace().path(), &self.base)?;
        ensure!(
            current.digest() == self.effective.digest(),
            "system policy or workspace changed; restart or rebuild the session before another run"
        );
        Ok(())
    }
    pub(crate) fn verify_workspace(&self) -> Result<()> {
        Ok(self.effective.workspace().verify_current()?)
    }
}
/// Runtime-only clone. The original user Config stays the persistence/handoff source.
pub struct RuntimePolicy {
    config: Config,
    policy: Policy,
}
impl RuntimePolicy {
    pub fn resolve(config: &Config, workspace: &Path) -> Result<Self> {
        Self::resolve_with_source(config, workspace, Source::System)
    }
    pub fn resolve_child(config: &Config, workspace: &Path, parent: &Policy) -> Result<Self> {
        Self::resolve_child_with_source(config, workspace, parent, Source::System)
    }
    fn resolve_child_with_source(
        config: &Config,
        workspace: &Path,
        parent: &Policy,
        source: Source,
    ) -> Result<Self> {
        parent.check_current()?;
        let mut config = config.clone();
        parent.limit_child_config(&mut config, workspace)?;
        let mut resolved = Self::resolve_with_source(&config, workspace, source)?;
        resolved.policy.inherit_execution_authority(parent);
        resolved.policy.check_execution_authority()?;
        Ok(resolved)
    }
    fn resolve_with_source(config: &Config, workspace: &Path, source: Source) -> Result<Self> {
        let base = Rules {
            access: config.access_mode(),
            unattended: config.unattended_approval.clone(),
            read_roots: root_names(&config.allow_read)?,
            write_roots: root_names(&config.allow_write)?,
            deny_commands: config.deny_commands.clone(),
            inherit_env: config.inherit_env.clone(),
        };
        let effective = source.resolve(workspace, &base)?;
        let mut config = config.clone();
        let rules = effective.rules();
        config.access = Some(rules.access);
        config.unattended_approval = rules.unattended.clone();
        config.allow_read = rules.read_roots.clone();
        config.allow_write = rules.write_roots.clone();
        config.deny_commands = rules.deny_commands.clone();
        config.inherit_env = rules.inherit_env.clone();
        if let Some(allowed) = effective.environment_ceiling() {
            restrict_environment(&mut config, allowed);
        }
        Ok(Self {
            config,
            policy: Policy::from_runtime(Snapshot {
                source,
                base,
                effective,
            }),
        })
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
    pub fn ceiling_present(&self) -> bool {
        self.policy.ceiling_present()
    }
    pub(crate) fn into_policy(self) -> Policy {
        self.policy
    }
}
fn root_names(roots: &[PathBuf]) -> Result<Vec<String>> {
    roots
        .iter()
        .map(|root| {
            // Existing Config allowed relative roots; resolve them before strict schema.
            let root = root.canonicalize()?;
            Ok(root
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("policy root must be UTF-8"))?
                .to_owned())
        })
        .collect()
}
// Removed subprocess grants remain secrets even though they are no longer exported.
pub(crate) fn restrict_environment(config: &mut Config, allowed: &[String]) {
    let mut removed = Vec::new();
    let mut retain = |name: &String, value: &mut String| {
        let keep = allowed.contains(name);
        if !keep {
            removed.push(value.clone());
        }
        keep
    };
    config.env.retain(&mut retain);
    for server in config.mcp_servers.values_mut() {
        server.env.retain(&mut retain);
    }
    for value in removed {
        if !config.redact_values.contains(&value) {
            config.redact_values.push(value);
        }
    }
}
pub(crate) fn restrictive_unattended(
    value: &mut UnattendedApprovalMode,
    parent: &UnattendedApprovalMode,
) {
    if *parent == UnattendedApprovalMode::Deny {
        *value = UnattendedApprovalMode::Deny
    }
}
#[cfg(all(test, target_os = "linux"))]
mod tests;
