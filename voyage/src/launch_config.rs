//! Trusted local launch handoff only; never deserialize this from session history.
use crate::{
    Config,
    policy_profile::{
        Overrides,
        selection::{Selection, SelectionRequest},
    },
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchConfig {
    version: u32,
    workspace: PathBuf,
    config: Config,
    explicit: Overrides,
    selection: Option<SelectionRequest>,
    confirmation: Option<String>,
}
impl LaunchConfig {
    pub fn matches_config(&self, config: &Config) -> Result<bool> {
        Ok(
            serde_json::to_value(&self.config)? == serde_json::to_value(config)?
                && serde_json::to_value(&self.explicit)?
                    == serde_json::to_value(&config.policy_explicit)?
                && serde_json::to_value(&self.selection)?
                    == serde_json::to_value(
                        config
                            .policy_profile
                            .as_ref()
                            .map(|selection| selection.request()),
                    )?
                && self.confirmation.as_deref()
                    == config
                        .policy_profile
                        .as_ref()
                        .map(|selection| selection.confirmation()),
        )
    }
    pub fn capture(config: &Config, workspace: &Path) -> Result<Self> {
        crate::runtime_policy::RuntimePolicy::resolve(config, workspace)?;
        Ok(Self {
            version: 1,
            workspace: workspace.canonicalize()?,
            config: config.clone(),
            explicit: config.policy_explicit.clone(),
            selection: config
                .policy_profile
                .as_ref()
                .map(|selection| selection.request().clone()),
            confirmation: config
                .policy_profile
                .as_ref()
                .map(|selection| selection.confirmation().to_owned()),
        })
    }
    /// Private retained settings for public-envelope validation only. This does
    /// not resolve policy or grant execution authority, and needs no live root.
    pub(crate) fn observation_config(self, workspace: &Path) -> Result<Config> {
        ensure!(
            self.version == 1 && workspace == self.workspace,
            "launch configuration workspace/version mismatch"
        );
        Ok(self.config)
    }

    /// Caller must first authenticate ownership, privacy and session binding of
    /// the local launch file. Re-resolve profile revisions and ceilings here.
    pub fn resolve(mut self, workspace: &Path) -> Result<Config> {
        ensure!(
            self.version == 1 && workspace.canonicalize()? == self.workspace,
            "launch configuration workspace/version mismatch"
        );
        self.config.policy_explicit = self.explicit;
        self.config.policy_profile = match self.selection {
            Some(request) => Some(Selection::bind(
                &self.config,
                workspace,
                request,
                self.confirmation.as_deref(),
            )?),
            None => {
                ensure!(
                    self.confirmation.is_none(),
                    "launch confirmation without selection"
                );
                None
            }
        };
        crate::runtime_policy::RuntimePolicy::resolve(&self.config, workspace)?;
        Ok(self.config)
    }
}
