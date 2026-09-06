//! In-process interactive transitions. Preview metadata is never dispatch authority.
use super::{
    EffectivePolicy,
    selection::{Selection, SelectionRequest},
    store::{ProfileSnapshot, ProfileStore},
};
use crate::{Config, runtime_policy::RuntimePolicy};
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

fn source_busy(error: &anyhow::Error) -> bool {
    error.downcast_ref::<super::store::StoreError>() == Some(&super::store::StoreError::Busy)
        || error.downcast_ref::<super::defaults::StoreError>()
            == Some(&super::defaults::StoreError::Busy)
}
/// Run only on a blocking worker. Temporary administrative contention can retry;
/// stale revisions, invalid evidence and changed authority never do.
pub fn read_sources<T>(mut read: impl FnMut() -> Result<T>) -> Result<T> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match read() {
            Err(error) if source_busy(&error) && std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            result => return result,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Profile(SelectionRequest),
    Defaults,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchRequest {
    pub target: Target,
    pub preview_digest: String,
    pub confirmation: Option<String>,
}
#[derive(Clone, Debug)]
pub struct SwitchPreview {
    pub previous: EffectivePolicy,
    pub baseline: EffectivePolicy,
    pub proposed: EffectivePolicy,
    pub requires_confirmation: bool,
    pub digest: String,
}
#[derive(Clone)]
pub struct SwitchContext {
    config: Config,
    previous: EffectivePolicy,
}
impl SwitchContext {
    pub fn new(config: Config, previous: EffectivePolicy) -> Self {
        Self { config, previous }
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn current(&self) -> &EffectivePolicy {
        &self.previous
    }
    pub fn default_directory(&self) -> Result<PathBuf> {
        if let Some(selection) = &self.config.policy_profile {
            return Ok(selection.request().directory.clone());
        }
        if self.config.policy_defaults.is_some() {
            match super::defaults::preview(&self.config, self.previous.workspace().path()) {
                Ok(preview) => {
                    if let Some(profile) = preview.selected_profile {
                        return Ok(profile.directory);
                    }
                }
                Err(error) if source_busy(&error) => return Err(error),
                // Missing or stale defaults do not prevent explicit administrative
                // browsing/recovery in the ordinary private profile directory.
                Err(_) => {}
            }
        }
        crate::config::default_config_path()
            .and_then(|p| p.parent().map(|p| p.join("profiles")))
            .ok_or_else(|| anyhow::anyhow!("policy directory unavailable"))
    }
    /// Explicit administrative browsing initializes only inert profile metadata.
    pub fn profiles(&self, directory: &Path) -> Result<Vec<ProfileSnapshot>> {
        ensure!(directory.is_absolute(), "policy directory must be absolute");
        if directory == self.default_directory()? {
            #[cfg(unix)]
            super::cli::initialize_default_parent(
                directory
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("policy directory requires parent"))?,
            )?;
        }
        let store = match ProfileStore::open_existing(directory) {
            Ok(store) => store,
            Err(super::store::StoreError::Busy) => {
                return Err(super::store::StoreError::Busy.into());
            }
            // Explicit browsing may initialize inert metadata. Existing valid
            // stores use shared reads instead of contending with runtime checks.
            Err(_) => ProfileStore::open(directory)?,
        };
        let mut output = Vec::new();
        let mut after = None;
        loop {
            let page = store.list(after.as_deref(), 100)?;
            output.extend(page.profiles.into_iter().filter(|p| p.rules.is_some()));
            ensure!(output.len() <= 1027, "policy profile list exceeds limit");
            match page.next_after {
                Some(next) => {
                    ensure!(
                        after.as_ref().is_none_or(|old| old < &next),
                        "invalid profile pagination"
                    );
                    after = Some(next);
                }
                None => break,
            }
        }
        Ok(output)
    }
    pub fn target(&self, directory: PathBuf, profile: &ProfileSnapshot) -> Result<Target> {
        Ok(Target::Profile(SelectionRequest {
            directory,
            name: profile.name.clone(),
            revision: profile.revision,
            digest: profile.digest()?,
            explicit: self.config.policy_explicit.clone(),
        }))
    }
    fn candidate(&self, target: &Target) -> Result<Config> {
        let mut candidate = self.config.clone();
        candidate.policy_profile = None;
        if let Target::Profile(request) = target {
            let preview =
                Selection::preview(&candidate, self.previous.workspace().path(), request)?;
            // This stages an inert Config; prepare below checks the independent
            // combined interactive confirmation before returning it for a build.
            candidate.policy_profile = Some(Selection::bind(
                &candidate,
                self.previous.workspace().path(),
                request.clone(),
                Some(&preview.transition_digest),
            )?);
        }
        Ok(candidate)
    }
    fn prepare_preview(&self, target: &Target) -> Result<(SwitchPreview, Config)> {
        let candidate = self.candidate(target)?;
        let workspace = self.previous.workspace().path();
        let proposed = RuntimePolicy::resolve(&candidate, workspace)?
            .policy()
            .effective()
            .clone();
        let mut base = self.config.clone();
        base.policy_profile = None;
        base.policy_defaults = None;
        let baseline = RuntimePolicy::resolve(&base, workspace)?
            .policy()
            .effective()
            .clone();
        let live_change = super::transition(&self.previous, &proposed)?;
        let base_change = super::transition(&baseline, &proposed)?;
        let digest = super::hash(&(self.previous.digest(), baseline.digest(), proposed.digest()))?;
        Ok((
            SwitchPreview {
                previous: self.previous.clone(),
                baseline,
                proposed,
                requires_confirmation: live_change.requires_confirmation()
                    || base_change.requires_confirmation(),
                digest,
            },
            candidate,
        ))
    }
    pub fn preview(&self, target: &Target) -> Result<SwitchPreview> {
        Ok(self.prepare_preview(target)?.0)
    }
    /// Re-resolve every source at apply, then bind the exact preview and consent.
    pub fn prepare(&self, request: &SwitchRequest) -> Result<Config> {
        Ok(self.prepare_with_digest(request)?.0)
    }
    /// The expected runtime digest comes from the same validated read as consent.
    pub fn prepare_with_digest(&self, request: &SwitchRequest) -> Result<(Config, String)> {
        let (preview, candidate) = self.prepare_preview(&request.target)?;
        ensure!(
            preview.digest == request.preview_digest,
            "policy preview changed; review again"
        );
        ensure!(
            !preview.requires_confirmation
                || request.confirmation.as_deref() == Some(&preview.digest),
            "policy escalation requires explicit confirmation of this preview"
        );
        ensure!(
            request
                .confirmation
                .as_ref()
                .is_none_or(|confirmation| confirmation == &preview.digest),
            "policy confirmation is stale"
        );
        Ok((candidate, preview.proposed.digest().to_owned()))
    }
}
