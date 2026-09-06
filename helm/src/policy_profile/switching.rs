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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::config::AccessMode;
    use crate::policy_profile::{
        Builtin,
        store::{Action, ProfileChange},
    };
    fn context() -> (tempfile::TempDir, SwitchContext, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles");
        ProfileStore::open(&path).unwrap();
        let mut config = Config {
            access: Some(AccessMode::Unrestricted),
            ..Config::default()
        };
        let snapshot = ProfileStore::open_existing(&path)
            .unwrap()
            .inspect("restricted")
            .unwrap()
            .unwrap();
        let request = SelectionRequest {
            directory: path.clone(),
            name: snapshot.name.clone(),
            revision: snapshot.revision,
            digest: snapshot.digest().unwrap(),
            explicit: Default::default(),
        };
        config.policy_profile = Some(Selection::bind(&config, dir.path(), request, None).unwrap());
        let previous = RuntimePolicy::resolve(&config, dir.path())
            .unwrap()
            .policy()
            .effective()
            .clone();
        (dir, SwitchContext::new(config, previous), path)
    }
    #[test]
    fn broader_live_profile_requires_confirmation_even_below_original_config() {
        let (_dir, context, path) = context();
        let profile = ProfileStore::open_existing(&path)
            .unwrap()
            .inspect("balanced")
            .unwrap()
            .unwrap();
        let target = context.target(path, &profile).unwrap();
        let preview = context.preview(&target).unwrap();
        assert!(preview.requires_confirmation);
        assert_eq!(preview.previous.rules().access, AccessMode::ReadOnly);
        assert_eq!(preview.baseline.rules().access, AccessMode::Unrestricted);
        let mut request = SwitchRequest {
            target,
            preview_digest: preview.digest.clone(),
            confirmation: None,
        };
        assert!(context.prepare(&request).is_err());
        request.confirmation = Some(preview.digest);
        let selected = context.prepare(&request).unwrap();
        assert_eq!(
            RuntimePolicy::resolve(&selected, context.current().workspace().path())
                .unwrap()
                .policy()
                .access_mode(),
            AccessMode::Approval
        );
        assert!(
            !toml::to_string(&selected)
                .unwrap()
                .contains("policy_profile")
        );
    }
    #[test]
    fn workspace_replacement_and_cross_target_receipts_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let config = Config {
            access: Some(AccessMode::ReadOnly),
            ..Config::default()
        };
        let previous = RuntimePolicy::resolve(&config, &workspace)
            .unwrap()
            .policy()
            .effective()
            .clone();
        let context = SwitchContext::new(config, previous);
        let path = dir.path().join("profiles");
        let profiles = context.profiles(&path).unwrap();
        let target = context
            .target(
                path.clone(),
                profiles.iter().find(|p| p.name == "autonomous").unwrap(),
            )
            .unwrap();
        let preview = context.preview(&target).unwrap();
        let mut request = SwitchRequest {
            target,
            preview_digest: preview.digest.clone(),
            confirmation: Some(preview.digest),
        };
        assert!(context.prepare(&request).is_ok());
        let original = request.target.clone();
        request.target = context
            .target(
                path,
                profiles.iter().find(|p| p.name == "balanced").unwrap(),
            )
            .unwrap();
        assert!(context.prepare(&request).is_err());
        request.target = original;
        std::fs::rename(&workspace, dir.path().join("old-workspace")).unwrap();
        std::fs::create_dir(&workspace).unwrap();
        assert!(context.prepare(&request).is_err());
    }
    #[test]
    fn browsing_shares_live_validation_locks_and_retries_only_temporary_writers() {
        let (_dir, context, path) = context();
        let directory =
            crate::attachment::local_actor::storage::Directory::open_existing(&path).unwrap();
        let read = directory.read_lock().unwrap();
        assert_eq!(context.profiles(&path).unwrap().len(), 3);
        drop(read);
        let write = directory.lock().unwrap();
        let held = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(80));
            drop(write);
        });
        assert_eq!(read_sources(|| context.profiles(&path)).unwrap().len(), 3);
        held.join().unwrap();
        let mut attempts = 0;
        let result: Result<()> = read_sources(|| {
            attempts += 1;
            Err(super::super::store::StoreError::Evidence.into())
        });
        assert!(result.is_err());
        assert_eq!(attempts, 1);
        let write = directory.lock().unwrap();
        let before = std::time::Instant::now();
        assert!(read_sources(|| context.profiles(&path)).is_err());
        assert!(before.elapsed() < std::time::Duration::from_secs(4));
        drop(write);
    }
    #[test]
    fn default_fallback_cannot_silently_restore_broader_config() {
        let (_dir, context, _) = context();
        let preview = context.preview(&Target::Defaults).unwrap();
        assert!(preview.requires_confirmation);
        assert!(
            context
                .prepare(&SwitchRequest {
                    target: Target::Defaults,
                    preview_digest: preview.digest,
                    confirmation: None
                })
                .is_err()
        );
    }
    #[test]
    fn edited_profile_invalidates_review_before_any_runtime_build() {
        let (_dir, context, path) = context();
        let store = ProfileStore::open_existing(&path).unwrap();
        let profile = store
            .change(&ProfileChange {
                operation_id: uuid::Uuid::new_v4(),
                name: "custom".into(),
                expected_revision: 0,
                action: Action::Create {
                    rules: Builtin::Balanced.document().rules,
                },
            })
            .unwrap()
            .snapshot;
        let target = context.target(path, &profile).unwrap();
        let preview = context.preview(&target).unwrap();
        store
            .change(&ProfileChange {
                operation_id: uuid::Uuid::new_v4(),
                name: "custom".into(),
                expected_revision: 1,
                action: Action::Replace {
                    rules: Builtin::Autonomous.document().rules,
                },
            })
            .unwrap();
        assert!(
            context
                .prepare(&SwitchRequest {
                    target,
                    preview_digest: preview.digest.clone(),
                    confirmation: Some(preview.digest)
                })
                .is_err()
        );
    }
}
