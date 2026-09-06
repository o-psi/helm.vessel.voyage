//! Explicit launch selection; never serialized into user/session authority.
use super::{EffectivePolicy, Layer, LayerKind, Overrides, Rules, store::ProfileStore, transition};
use crate::{
    Config,
    runtime_policy::{Source, config_rules},
};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionRequest {
    pub directory: PathBuf,
    pub name: String,
    pub revision: u64,
    pub digest: String,
    /// Only explicit invocation policy overrides, reapplied after the profile.
    pub explicit: Overrides,
}
/// Construction binds a fresh current profile and exact Config/workspace transition.
/// Cloning preserves this binding; it cannot be deserialized as resume authority.
#[derive(Clone, Debug)]
pub struct Selection {
    request: SelectionRequest,
    workspace: PathBuf,
    base: Rules,
    transition_digest: String,
    source: Source,
}
#[derive(Clone, Debug, Serialize)]
pub struct SelectionPreview {
    pub previous: EffectivePolicy,
    pub proposed: EffectivePolicy,
    pub requires_confirmation: bool,
    pub transition_digest: String,
}
impl SelectionRequest {
    fn layers(&self) -> Result<Vec<Layer>> {
        ensure!(
            self.directory.is_absolute()
                && self.revision > 0
                && self.digest.len() == 64
                && self
                    .digest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "invalid policy selection"
        );
        let snapshot = ProfileStore::open_existing(&self.directory)?
            .inspect(&self.name)?
            .ok_or_else(|| anyhow::anyhow!("selected policy profile is missing"))?;
        ensure!(
            snapshot.revision == self.revision
                && snapshot.digest()? == self.digest
                && snapshot.rules.is_some(),
            "selected policy profile changed or was deleted; explicitly reselect"
        );
        let mut profile = Layer::from_profile(LayerKind::Session, &snapshot.document()?)?;
        // Equal document rules/revisions in a recreated or relocated store are
        // still a different explicit selection. Bind that identity into the
        // effective provenance and therefore the transition confirmation.
        profile.profile_digest = Some(super::hash(&(
            snapshot.digest()?,
            self.directory.canonicalize()?,
        ))?);
        let mut layers = vec![profile];
        if self.explicit != Overrides::default() {
            layers.push(Layer::new(
                LayerKind::Explicit,
                "cli",
                self.explicit.clone(),
            )?);
        }
        Ok(layers)
    }
}
impl Selection {
    pub fn request(&self) -> &SelectionRequest {
        &self.request
    }
    pub fn preview(
        config: &Config,
        workspace: &Path,
        request: &SelectionRequest,
    ) -> Result<SelectionPreview> {
        Self::preview_using(&config_rules(config)?, workspace, request, &Source::System)
    }
    fn preview_using(
        base: &Rules,
        workspace: &Path,
        request: &SelectionRequest,
        source: &Source,
    ) -> Result<SelectionPreview> {
        let previous = source.resolve(workspace, base, &[])?;
        let proposed = source.resolve(workspace, base, &request.layers()?)?;
        let change = transition(&previous, &proposed)?;
        Ok(SelectionPreview {
            requires_confirmation: change.requires_confirmation(),
            transition_digest: change.digest().into(),
            previous,
            proposed,
        })
    }
    pub fn bind(
        config: &Config,
        workspace: &Path,
        request: SelectionRequest,
        confirmation: Option<&str>,
    ) -> Result<Self> {
        let base = config_rules(config)?;
        let preview = Self::preview_using(&base, workspace, &request, &Source::System)?;
        ensure!(
            !preview.requires_confirmation
                || confirmation == Some(preview.transition_digest.as_str()),
            "policy escalation requires the exact fresh preview confirmation digest"
        );
        ensure!(
            confirmation.is_none_or(|value| value == preview.transition_digest),
            "policy confirmation is stale"
        );
        Ok(Self {
            request,
            workspace: preview.proposed.workspace().path().into(),
            base,
            transition_digest: preview.transition_digest,
            source: Source::System,
        })
    }
    pub(crate) fn resolve(&self, workspace: &Path, base: &Rules) -> Result<EffectivePolicy> {
        ensure!(
            base == &self.base && workspace.canonicalize()? == self.workspace,
            "policy selection belongs to a different configuration or workspace; explicitly reselect"
        );
        let preview = Self::preview_using(base, workspace, &self.request, &self.source)?;
        ensure!(
            preview.transition_digest == self.transition_digest,
            "policy selection or system ceiling changed; explicitly reselect"
        );
        Ok(preview.proposed)
    }
    pub(crate) fn check_current(&self) -> Result<()> {
        self.resolve(&self.workspace, &self.base).map(|_| ())
    }
}
#[cfg(all(test, target_os = "linux"))]
mod tests;
