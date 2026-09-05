use super::*;
use crate::{
    Config,
    policy_profile::{
        self, EffectivePolicy, Layer, LayerKind, Overrides, Rules, store::ProfileStore,
    },
    runtime_policy::{Source, config_rules},
};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::path::{Path, PathBuf};
use uuid::Uuid;
#[derive(Clone, Debug, Serialize)]
pub struct DefaultsPreview {
    pub previous: EffectivePolicy,
    pub proposed: EffectivePolicy,
    pub requires_confirmation: bool,
    pub activation_current: bool,
    pub candidate_digest: String,
    pub transition_digest: String,
    pub context_digest: String,
    pub selected_profile: Option<ProfileRef>,
    pub selected_scope: Option<LayerKind>,
    /// Distinct historical policies that explain required escalation confirmation.
    pub comparison_policies: Vec<EffectivePolicy>,
}
#[derive(Clone, Debug)]
pub(crate) struct DefaultsGuard {
    anchor: DefaultsSource,
    workspace: PathBuf,
    base: Rules,
    explicit: Overrides,
    candidate_digest: String,
    effective_digest: String,
    source: Source,
}
impl DefaultsGuard {
    pub(crate) fn check_current(&self) -> Result<()> {
        let preview = prepare(
            &self.anchor,
            &self.workspace,
            &self.base,
            &self.explicit,
            &self.source,
        )?;
        ensure!(
            preview.candidate_digest == self.candidate_digest
                && preview.proposed.digest() == self.effective_digest,
            "policy defaults changed; restart or explicitly reselect"
        );
        check_activation(&preview)
    }
}
fn preference(snapshot: Option<&DefaultsSnapshot>) -> Option<&ProfileRef> {
    snapshot.and_then(|s| match &s.value {
        DefaultValue::Preference { profile } => profile.as_ref(),
        _ => None,
    })
}
fn rules(effective: &EffectivePolicy) -> Rules {
    let r = effective.rules();
    Rules {
        access: r.access,
        unattended: r.unattended.clone(),
        read_roots: r
            .read_roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        write_roots: r
            .write_roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        deny_commands: r.deny_commands.clone(),
        inherit_env: r.inherit_env.clone(),
    }
}
fn effective(
    workspace: &Path,
    base: &Rules,
    explicit: &Overrides,
    profile: Option<(&ProfileRef, LayerKind)>,
    fresh: bool,
    provenance: &str,
    source: &Source,
) -> Result<EffectivePolicy> {
    let mut layers = Vec::new();
    if let Some((profile, kind)) = profile {
        if fresh {
            let observed = ProfileStore::open_existing(&profile.directory)?
                .inspect(&profile.name)?
                .ok_or_else(|| anyhow::anyhow!("default policy profile is missing"))?;
            ensure!(
                observed == profile.snapshot && observed.digest()? == profile.digest,
                "default policy profile changed; explicitly update the default"
            );
        }
        let mut layer = Layer::from_profile(kind, &profile.snapshot.document()?)?;
        layer.profile_digest = Some(policy_profile::hash(&(profile, provenance))?);
        layers.push(layer);
    }
    if *explicit != Overrides::default() {
        layers.push(Layer::new(LayerKind::Explicit, "cli", explicit.clone())?);
    }
    source.resolve(workspace, base, &layers)
}
fn prepare(
    anchor: &DefaultsSource,
    workspace: &Path,
    base: &Rules,
    explicit: &Overrides,
    source: &Source,
) -> Result<DefaultsPreview> {
    let workspace = workspace.canonicalize()?;
    let store = DefaultsStore::open_existing(anchor)?;
    let history = store.history()?;
    let baseline = effective(&workspace, base, explicit, None, false, "config", source)?;
    let context_digest = policy_profile::hash(&(
        anchor.store_id,
        anchor.directory.canonicalize()?,
        base,
        explicit,
        baseline.workspace(),
        baseline.ceiling_digest(),
    ))?;
    let mut global = None;
    let mut local = None;
    let mut choices = Vec::new();
    let mut activation = None;
    for (sequence, snapshot) in history {
        match &snapshot.key {
            DefaultKey::Preference {
                scope: DefaultScope::Global {},
            } => {
                global = Some(snapshot);
                if preference(local.as_ref()).is_some() {
                    continue;
                }
            }
            DefaultKey::Preference {
                scope: DefaultScope::Workspace { workspace: key },
            } if *key == workspace => local = Some(snapshot),
            DefaultKey::Activation { workspace: key } if *key == workspace => {
                activation = Some((sequence, snapshot));
                continue;
            }
            _ => continue,
        }
        let chosen = preference(local.as_ref())
            .map(|p| (p.clone(), LayerKind::Project))
            .or_else(|| preference(global.as_ref()).map(|p| (p.clone(), LayerKind::Global)));
        let dependency = if preference(local.as_ref()).is_some() {
            policy_profile::hash(&local)?
        } else {
            policy_profile::hash(&(&global, &local))?
        };
        choices.push((sequence, chosen, dependency));
    }
    let (chosen, dependency) = choices
        .last()
        .map(|(_, p, d)| (p.as_ref().map(|(p, k)| (p, *k)), d.clone()))
        .unwrap_or((None, policy_profile::hash(&"empty-defaults")?));
    let candidate_digest = policy_profile::hash(&(&context_digest, &dependency))?;
    let proposed = effective(
        &workspace,
        base,
        explicit,
        chosen,
        true,
        &candidate_digest,
        source,
    )?;
    let mut adopted_sequence = 0;
    let mut previous = baseline.clone();
    let mut activation_current = false;
    if let Some((sequence, snapshot)) = activation
        && let DefaultValue::Activation {
            context_digest: context,
            candidate_digest: candidate,
            effective: adopted,
            ..
        } = snapshot.value
        && context == context_digest
    {
        previous = source.resolve(&workspace, &adopted, &[])?;
        adopted_sequence = sequence;
        activation_current = candidate == candidate_digest && adopted == rules(&proposed);
    }
    // Preferences are candidates, not adoption. Compare every possibly applied
    // choice since explicit workspace adoption, so A -> unactivated B -> C
    // cannot launder an escalation by making C equal to B.
    // Adoption is exact-candidate authority, never a broader replacement for
    // the operator's original Config authority on a subsequent candidate.
    let mut comparisons = vec![
        policy_profile::transition(&previous, &proposed)?,
        policy_profile::transition(&baseline, &proposed)?,
    ];
    let mut comparison_policies = Vec::new();
    if comparisons[0].requires_confirmation() {
        comparison_policies.push(previous.clone());
    }
    if comparisons[1].requires_confirmation()
        && !comparison_policies
            .iter()
            .any(|p| rules(p) == rules(&baseline))
    {
        comparison_policies.push(baseline);
    }
    for (sequence, profile, dependency) in &choices {
        if *sequence <= adopted_sequence {
            continue;
        }
        let historical = effective(
            &workspace,
            base,
            explicit,
            profile.as_ref().map(|(p, k)| (p, *k)),
            false,
            dependency,
            source,
        )?;
        let comparison = policy_profile::transition(&historical, &proposed)?;
        if comparison.requires_confirmation()
            && !comparison_policies
                .iter()
                .any(|p| rules(p) == rules(&historical))
        {
            comparison_policies.push(historical);
        }
        comparisons.push(comparison);
    }
    let requires_confirmation = comparisons.iter().any(|c| c.requires_confirmation());
    let transition_digest =
        policy_profile::hash(&(&context_digest, &candidate_digest, &comparisons))?;
    Ok(DefaultsPreview {
        previous,
        proposed,
        requires_confirmation,
        activation_current,
        candidate_digest,
        transition_digest,
        context_digest,
        selected_profile: chosen.map(|(p, _)| p.clone()),
        selected_scope: chosen.map(|(_, k)| k),
        comparison_policies,
    })
}
fn check_activation(preview: &DefaultsPreview) -> Result<()> {
    ensure!(
        !preview.requires_confirmation || preview.activation_current,
        "policy defaults escalation requires exact workspace activation; run policy defaults preview and activate"
    );
    Ok(())
}
pub fn preview(config: &Config, workspace: &Path) -> Result<DefaultsPreview> {
    let anchor = config
        .policy_defaults
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("policy defaults are not enabled"))?;
    prepare(
        anchor,
        workspace,
        &config_rules(config)?,
        &config.policy_explicit,
        &Source::System,
    )
}
pub fn activate(
    config: &Config,
    workspace: &Path,
    operation_id: Uuid,
    expected_revision: u64,
    confirmation: &str,
) -> Result<DefaultsReceipt> {
    let anchor = config
        .policy_defaults
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("policy defaults are not enabled"))?;
    let store = DefaultsStore::open_existing(anchor)?;
    if let Some(receipt) = store.operation(operation_id)? {
        ensure!(
            receipt.snapshot.key
                == DefaultKey::Activation {
                    workspace: workspace.canonicalize()?
                }
                && receipt.snapshot.revision.checked_sub(1) == Some(expected_revision)
                && matches!(&receipt.snapshot.value,DefaultValue::Activation{transition_digest,..} if transition_digest==confirmation),
            "policy defaults activation operation conflicts"
        );
        return Ok(receipt);
    }
    let preview = preview(config, workspace)?;
    ensure!(
        confirmation == preview.transition_digest,
        "policy defaults confirmation is stale"
    );
    let store = DefaultsStore::open_existing(anchor)?;
    let receipt = store.change(&DefaultsChange {
        operation_id,
        key: DefaultKey::Activation {
            workspace: workspace.canonicalize()?,
        },
        expected_revision,
        value: DefaultValue::Activation {
            candidate_digest: preview.candidate_digest,
            transition_digest: preview.transition_digest,
            context_digest: preview.context_digest,
            effective: rules(&preview.proposed),
        },
    })?;
    let current = self::preview(config, workspace)?;
    ensure!(
        current.activation_current,
        "policy defaults changed during activation; preview again"
    );
    Ok(receipt)
}
#[cfg(test)]
pub(crate) fn resolve(
    config: &Config,
    workspace: &Path,
) -> Result<(EffectivePolicy, DefaultsGuard)> {
    resolve_using(config, workspace, Source::System)
}
pub(crate) fn resolve_using(
    config: &Config,
    workspace: &Path,
    source: Source,
) -> Result<(EffectivePolicy, DefaultsGuard)> {
    let anchor = config
        .policy_defaults
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("policy defaults are not enabled"))?;
    let base = config_rules(config)?;
    let preview = prepare(anchor, workspace, &base, &config.policy_explicit, &source)?;
    check_activation(&preview)?;
    let guard = DefaultsGuard {
        anchor: anchor.clone(),
        workspace: workspace.canonicalize()?,
        base,
        explicit: config.policy_explicit.clone(),
        candidate_digest: preview.candidate_digest,
        effective_digest: preview.proposed.digest().into(),
        source,
    };
    Ok((preview.proposed, guard))
}
