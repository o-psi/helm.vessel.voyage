//! Mandatory runtime ceiling resolution; effective Config clones are never persisted.
use crate::{
    Config,
    config::UnattendedApprovalMode,
    policy::Policy,
    policy_profile::{self, EffectivePolicy, Layer, Rules, selection::Selection},
};
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug)]
pub(crate) enum Source {
    System,
}
impl Source {
    pub(crate) fn resolve(
        &self,
        workspace: &Path,
        base: &Rules,
        layers: &[Layer],
    ) -> Result<EffectivePolicy> {
        match self {
            Self::System => Ok(policy_profile::resolve_runtime_layers(
                workspace, base, layers,
            )?),
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub(crate) sandbox: crate::sandbox::Sandbox,
    source: Source,
    base: Rules,
    selection: Option<Selection>,
    pub(crate) ancestor_selection: Option<Selection>,
    pub(crate) defaults: Option<crate::policy_profile::defaults::DefaultsGuard>,
    pub(crate) ancestor_defaults: Option<crate::policy_profile::defaults::DefaultsGuard>,
    pub(crate) effective: EffectivePolicy,
}
impl Snapshot {
    pub(crate) fn check_current(&self) -> Result<()> {
        if let Some(guard) = self.defaults.as_ref().or(self.ancestor_defaults.as_ref()) {
            guard.check_current()?;
        }
        if let Some(selection) = &self.ancestor_selection {
            selection.check_current()?;
        }
        let current = if let Some(selection) = &self.selection {
            selection.resolve(self.effective.workspace().path(), &self.base)?
        } else if self.defaults.is_some() {
            // The defaults guard above re-resolved its exact original Config and source.
            self.effective.clone()
        } else {
            self.source
                .resolve(self.effective.workspace().path(), &self.base, &[])?
        };
        ensure!(
            current.digest() == self.effective.digest(),
            "system policy or workspace changed; restart or rebuild the session before another run"
        );
        Ok(())
    }
    pub(crate) fn inherited_selection(&self) -> Option<Selection> {
        self.selection
            .clone()
            .or_else(|| self.ancestor_selection.clone())
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
        // The child gets resolved parent maxima, never the parent's broader layer.
        config.policy_profile = None;
        config.policy_defaults = None;
        config.policy_explicit = Default::default();
        let mut resolved = Self::resolve_with_source(&config, workspace, source)?;
        resolved.policy.inherit_profile_freshness(parent);
        resolved.policy.inherit_execution_authority(parent);
        resolved.policy.check_current()?;
        Ok(resolved)
    }
    fn resolve_with_source(config: &Config, workspace: &Path, source: Source) -> Result<Self> {
        let base = config_rules(config)?;
        let selection = config.policy_profile.clone();
        let (effective, defaults) = if let Some(selection) = &selection {
            (selection.resolve(workspace, &base)?, None)
        } else if config.policy_defaults.is_some() {
            let (effective, guard) =
                crate::policy_profile::defaults::resolve_using(config, workspace, source.clone())?;
            (effective, Some(guard))
        } else {
            (source.resolve(workspace, &base, &[])?, None)
        };
        let mut config = config.clone();
        config.policy_defaults = None;
        let rules = effective.rules();
        config.access = Some(rules.access);
        config.unattended_approval = rules.unattended.clone();
        config.allow_read = rules.read_roots.clone();
        config.allow_write = rules.write_roots.clone();
        config.legacy_deny_commands.clear();
        config.inherit_env = rules.inherit_env.clone();
        config.github_enabled = rules.github_enabled;
        if let Some(allowed) = effective.environment_ceiling() {
            restrict_environment(&mut config, allowed);
        }
        let sandbox =
            crate::sandbox::Sandbox::new(&config.sandbox, &rules.read_roots, &rules.write_roots)?;
        Ok(Self {
            config,
            policy: Policy::from_runtime(Snapshot {
                sandbox,
                source,
                base,
                selection,
                ancestor_selection: None,
                defaults,
                ancestor_defaults: None,
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
pub(crate) fn config_rules(config: &Config) -> Result<Rules> {
    Ok(Rules {
        access: config.access_mode(),
        unattended: config.unattended_approval.clone(),
        read_roots: root_names(&config.allow_read)?,
        write_roots: root_names(&config.allow_write)?,
        legacy_deny_commands: Vec::new(),
        inherit_env: config.inherit_env.clone(),
        github_enabled: config.github_enabled,
    })
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
