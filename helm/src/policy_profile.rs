//! Inspectable policy metadata. Existing runtime builders do not yet consume it.
//! Only `resolve_current` constructs public effective values; it always checks the
//! fixed system ceiling. A transition confirmation is not dispatch authority.
mod ceiling;
use crate::config::{AccessMode, UnattendedApprovalMode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};
const MAX_DOCUMENT: usize = 64 * 1024;
const MAX_ITEMS: usize = 128;
const MAX_LAYERS: usize = 16;
type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid or unsupported policy document")]
    Invalid,
    #[error("policy root is unavailable or invalid")]
    Root,
    #[error("system policy ceiling is unavailable or untrusted")]
    Ceiling,
    #[error("workspace is outside the system policy ceiling")]
    WorkspaceDenied,
    #[error("policy transition is stale or belongs to another workspace")]
    Transition,
    #[error("trusted system policy resolution currently requires Linux")]
    UnsupportedPlatform,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub access: AccessMode,
    pub unattended: UnattendedApprovalMode,
    /// Absolute existing directory or the literal `$workspace`.
    pub read_roots: Vec<String>,
    pub write_roots: Vec<String>,
    pub deny_commands: Vec<String>,
    /// Environment variable names only, never their values.
    pub inherit_env: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDocument {
    pub schema: u32,
    pub name: String,
    pub revision: u64,
    pub rules: Rules,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CeilingDocument {
    pub schema: u32,
    pub rules: Rules,
}
#[derive(Clone, Copy, Debug)]
pub enum Builtin {
    Restricted,
    Balanced,
    Autonomous,
}
impl Builtin {
    pub fn document(self) -> ProfileDocument {
        let (name, access) = match self {
            Self::Restricted => ("restricted", AccessMode::ReadOnly),
            Self::Balanced => ("balanced", AccessMode::Approval),
            Self::Autonomous => ("autonomous", AccessMode::Unrestricted),
        };
        ProfileDocument {
            schema: 1,
            name: name.into(),
            revision: 1,
            rules: Rules {
                access,
                unattended: UnattendedApprovalMode::Deny,
                read_roots: vec!["$workspace".into()],
                write_roots: vec!["$workspace".into()],
                deny_commands: vec!["shutdown".into(), "reboot".into(), "mkfs".into()],
                inherit_env: vec!["PATH".into(), "LANG".into(), "LC_ALL".into(), "TERM".into()],
            },
        }
    }
}
fn label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
        && s != "."
        && s != ".."
}
fn env_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}
fn absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|c| {
            matches!(
                c,
                Component::RootDir | Component::Normal(_) | Component::Prefix(_)
            )
        })
}
impl Rules {
    fn validate(&self) -> Result<()> {
        for list in [
            &self.read_roots,
            &self.write_roots,
            &self.deny_commands,
            &self.inherit_env,
        ] {
            if list.len() > MAX_ITEMS {
                return Err(Error::Invalid);
            }
        }
        if self.read_roots.iter().chain(&self.write_roots).any(|s| {
            s.is_empty()
                || s.len() > 4096
                || s.chars().any(char::is_control)
                || (s != "$workspace" && !absolute(Path::new(s)))
        }) || self.deny_commands.iter().any(|s| !label(s))
            || self.inherit_env.iter().any(|s| !env_name(s))
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
impl ProfileDocument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_DOCUMENT {
            return Err(Error::Invalid);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        value.validate()?;
        Ok(value)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        encode(self)
    }
    fn validate(&self) -> Result<()> {
        if self.schema != 1
            || !label(&self.name)
            || self.revision == 0
            || self.revision > i64::MAX as u64
        {
            return Err(Error::Invalid);
        }
        self.rules.validate()
    }
}
impl CeilingDocument {
    /// The protected system file uses TOML; unknown fields are rejected.
    #[cfg(target_os = "linux")]
    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_DOCUMENT {
            return Err(Error::Ceiling);
        }
        let value: Self = toml::from_str(std::str::from_utf8(bytes).map_err(|_| Error::Ceiling)?)
            .map_err(|_| Error::Ceiling)?;
        if value.schema != 1 {
            return Err(Error::Ceiling);
        }
        value.rules.validate().map_err(|_| Error::Ceiling)?;
        Ok(value)
    }
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_DOCUMENT {
        return Err(Error::Invalid);
    }
    Ok(bytes)
}
fn hash(value: &impl Serialize) -> Result<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(value).map_err(|_| Error::Invalid)?,
    )))
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    pub access: Option<AccessMode>,
    pub unattended: Option<UnattendedApprovalMode>,
    pub read_roots: Option<Vec<String>>,
    pub write_roots: Option<Vec<String>>,
    pub deny_commands: Option<Vec<String>>,
    pub inherit_env: Option<Vec<String>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayerKind {
    Global,
    Project,
    Session,
    Explicit,
}
#[derive(Clone, Debug, Serialize)]
pub struct Layer {
    kind: LayerKind,
    identity: String,
    profile_revision: Option<u64>,
    profile_digest: Option<String>,
    overrides: Overrides,
}
impl Layer {
    /// Binds source attribution to this exact validated document revision and bytes.
    pub fn from_profile(kind: LayerKind, profile: &ProfileDocument) -> Result<Self> {
        profile.validate()?;
        let r = &profile.rules;
        let mut value = Self::new(
            kind,
            &profile.name,
            Overrides {
                access: Some(r.access),
                unattended: Some(r.unattended.clone()),
                read_roots: Some(r.read_roots.clone()),
                write_roots: Some(r.write_roots.clone()),
                deny_commands: Some(r.deny_commands.clone()),
                inherit_env: Some(r.inherit_env.clone()),
            },
        )?;
        value.profile_revision = Some(profile.revision);
        value.profile_digest = Some(hash(profile)?);
        Ok(value)
    }
    pub fn new(kind: LayerKind, identity: &str, overrides: Overrides) -> Result<Self> {
        if !label(identity) {
            return Err(Error::Invalid);
        }
        let layer = Self {
            kind,
            identity: identity.into(),
            profile_revision: None,
            profile_digest: None,
            overrides,
        };
        let mut r = Builtin::Balanced.document().rules;
        layer.apply(&mut r);
        r.validate()?;
        Ok(layer)
    }
    fn apply(&self, r: &mut Rules) {
        macro_rules! field {
            ($name:ident) => {
                if let Some(v) = &self.overrides.$name {
                    r.$name = v.clone()
                }
            };
        }
        field!(access);
        field!(unattended);
        field!(read_roots);
        field!(write_roots);
        field!(deny_commands);
        field!(inherit_env);
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EffectiveRules {
    pub access: AccessMode,
    pub unattended: UnattendedApprovalMode,
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    pub deny_commands: Vec<String>,
    pub inherit_env: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Contribution {
    pub source: String,
    pub value: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkspaceIdentity {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl WorkspaceIdentity {
    pub(crate) fn verify_current(&self) -> Result<()> {
        let path = directory(&self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::metadata(&path).map_err(|_| Error::Root)?;
            if path != self.path || metadata.dev() != self.device || metadata.ino() != self.inode {
                return Err(Error::Transition);
            }
        }
        Ok(())
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct EffectivePolicy {
    workspace: WorkspaceIdentity,
    rules: EffectiveRules,
    provenance: BTreeMap<String, Vec<Contribution>>,
    ceiling_digest: Option<String>,
    environment_ceiling: Option<Vec<String>>,
    digest: String,
}
impl EffectivePolicy {
    pub fn rules(&self) -> &EffectiveRules {
        &self.rules
    }
    pub fn provenance(&self) -> &BTreeMap<String, Vec<Contribution>> {
        &self.provenance
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn workspace(&self) -> &WorkspaceIdentity {
        &self.workspace
    }
    /// Protected administrator name ceiling, distinct from inherited names.
    pub fn environment_ceiling(&self) -> Option<&[String]> {
        self.environment_ceiling.as_deref()
    }
    pub fn ceiling_digest(&self) -> Option<&str> {
        self.ceiling_digest.as_deref()
    }
}
fn directory(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize().map_err(|_| Error::Root)?;
    if !path.is_dir() {
        return Err(Error::Root);
    }
    Ok(path)
}
fn normalize_roots(mut roots: Vec<PathBuf>) -> Vec<PathBuf> {
    roots.sort();
    roots.dedup();
    let snapshot = roots.clone();
    roots.retain(|root| {
        !snapshot
            .iter()
            .any(|parent| parent != root && root.starts_with(parent))
    });
    roots
}
fn prepare(r: &Rules, workspace: &Path) -> Result<EffectiveRules> {
    r.validate()?;
    prepare_trusted_config(r, workspace)
}
// Existing operator Config has its own validation contract; imported documents stay strict.
fn prepare_trusted_config(r: &Rules, workspace: &Path) -> Result<EffectiveRules> {
    let roots = |values: &[String]| -> Result<Vec<PathBuf>> {
        values
            .iter()
            .map(|p| {
                directory(if p == "$workspace" {
                    workspace
                } else {
                    Path::new(p)
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(normalize_roots)
    };
    let sorted = |v: &Vec<String>| {
        v.iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    Ok(EffectiveRules {
        access: r.access,
        unattended: r.unattended.clone(),
        read_roots: roots(&r.read_roots)?,
        write_roots: roots(&r.write_roots)?,
        deny_commands: sorted(&r.deny_commands),
        inherit_env: sorted(&r.inherit_env),
    })
}
fn rank(mode: AccessMode) -> u8 {
    match mode {
        AccessMode::ReadOnly => 0,
        AccessMode::Approval => 1,
        AccessMode::Unrestricted => 2,
    }
}
fn intersect(a: &[PathBuf], b: &[PathBuf]) -> Vec<PathBuf> {
    normalize_roots(
        a.iter()
            .flat_map(|x| {
                b.iter().filter_map(move |y| {
                    if x.starts_with(y) {
                        Some(x.clone())
                    } else if y.starts_with(x) {
                        Some(y.clone())
                    } else {
                        None
                    }
                })
            })
            .collect(),
    )
}
fn ceiling_rules(r: &mut EffectiveRules, c: &EffectiveRules, workspace: &Path) -> Result<()> {
    if !c.read_roots.iter().any(|p| workspace.starts_with(p))
        || !c.write_roots.iter().any(|p| workspace.starts_with(p))
    {
        return Err(Error::WorkspaceDenied);
    }
    r.read_roots = intersect(&r.read_roots, &c.read_roots);
    r.write_roots = intersect(&r.write_roots, &c.write_roots);
    r.inherit_env.retain(|name| c.inherit_env.contains(name));
    r.deny_commands.extend(c.deny_commands.clone());
    r.deny_commands.sort();
    r.deny_commands.dedup();
    if rank(c.access) < rank(r.access) {
        r.access = c.access
    }
    if c.unattended == UnattendedApprovalMode::Deny {
        r.unattended = UnattendedApprovalMode::Deny
    }
    Ok(())
}
fn record(
    provenance: &mut BTreeMap<String, Vec<Contribution>>,
    source: &str,
    before: Option<&EffectiveRules>,
    rules: &EffectiveRules,
) -> Result<()> {
    let after = serde_json::to_value(rules).map_err(|_| Error::Invalid)?;
    let before = before
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| Error::Invalid)?;
    for (key, value) in after.as_object().ok_or(Error::Invalid)? {
        if before.as_ref().and_then(|v| v.get(key)) != Some(value) {
            provenance
                .entry(key.clone())
                .or_default()
                .push(Contribution {
                    source: source.into(),
                    value: value.clone(),
                });
        }
    }
    Ok(())
}
/// Always reads the protected fixed Linux ceiling before resolving local inputs.
/// Metadata only: callers must rebuild runtime authority and recheck at dispatch.
pub fn resolve_current(
    workspace: &Path,
    base: &Rules,
    layers: &[Layer],
) -> Result<EffectivePolicy> {
    let ceiling = ceiling::load()?;
    resolve_loaded(workspace, base, layers, ceiling.as_ref())
}
fn resolve_loaded(
    workspace: &Path,
    base: &Rules,
    layers: &[Layer],
    ceiling: Option<&CeilingDocument>,
) -> Result<EffectivePolicy> {
    resolve_loaded_inner(workspace, base, layers, ceiling, false)
}

pub(crate) fn resolve_runtime_current(workspace: &Path, base: &Rules) -> Result<EffectivePolicy> {
    #[cfg(target_os = "linux")]
    let ceiling = ceiling::load()?;
    // This internal zero-layer Config adapter preserves existing non-Linux runtime
    // behavior. Public profile resolution still refuses unsupported enforcement.
    #[cfg(not(target_os = "linux"))]
    let ceiling: Option<CeilingDocument> = None;
    resolve_loaded_inner(workspace, base, &[], ceiling.as_ref(), true)
}
fn resolve_loaded_inner(
    workspace: &Path,
    base: &Rules,
    layers: &[Layer],
    ceiling: Option<&CeilingDocument>,
    trusted_config: bool,
) -> Result<EffectivePolicy> {
    if layers.len() > MAX_LAYERS || layers.windows(2).any(|w| w[0].kind >= w[1].kind) {
        return Err(Error::Invalid);
    }
    let path = directory(workspace)?;
    let metadata = std::fs::metadata(&path).map_err(|_| Error::Root)?;
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let (device, inode) = {
        let _ = metadata;
        (0, 0)
    };
    let workspace = WorkspaceIdentity {
        path,
        device,
        inode,
    };
    let mut raw = base.clone();
    let prepared_base = if trusted_config {
        prepare_trusted_config(&raw, &workspace.path)?
    } else {
        prepare(&raw, &workspace.path)?
    };
    let mut prepared_layers = Vec::new();
    for layer in layers {
        layer.apply(&mut raw);
        let source = format!("{:?}:{}:{}", layer.kind, layer.identity, hash(layer)?);
        let fields = serde_json::to_value(&layer.overrides).map_err(|_| Error::Invalid)?;
        let fields = fields
            .as_object()
            .ok_or(Error::Invalid)?
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, _)| k.clone())
            .collect::<BTreeSet<_>>();
        prepared_layers.push((source, prepare(&raw, &workspace.path)?, fields));
    }
    let prepared_ceiling = ceiling
        .map(|ceiling| {
            if ceiling.schema != 1 {
                return Err(Error::Ceiling);
            }
            Ok((
                prepare(&ceiling.rules, &workspace.path).map_err(|_| Error::Ceiling)?,
                hash(ceiling)?,
            ))
        })
        .transpose()?;
    resolve_prepared(workspace, prepared_base, prepared_layers, prepared_ceiling)
}
/// Pure resolver: all filesystem reads and canonicalization occur before entry.
fn resolve_prepared(
    workspace: WorkspaceIdentity,
    mut current: EffectiveRules,
    layers: Vec<(String, EffectiveRules, BTreeSet<String>)>,
    ceiling: Option<(EffectiveRules, String)>,
) -> Result<EffectivePolicy> {
    let mut provenance = BTreeMap::new();
    record(&mut provenance, "base", None, &current)?;
    for (source, next, fields) in layers {
        let value = serde_json::to_value(&next).map_err(|_| Error::Invalid)?;
        for field in fields {
            provenance
                .entry(field.clone())
                .or_default()
                .push(Contribution {
                    source: source.clone(),
                    value: value.get(&field).ok_or(Error::Invalid)?.clone(),
                });
        }
        current = next;
    }
    // Preserve current Policy's implicit workspace semantics, never bypass ceiling.
    let before = current.clone();
    current.read_roots.push(workspace.path.clone());
    current.write_roots.push(workspace.path.clone());
    current.read_roots = normalize_roots(current.read_roots);
    current.write_roots = normalize_roots(current.write_roots);
    record(
        &mut provenance,
        "implicit-workspace",
        Some(&before),
        &current,
    )?;
    let environment_ceiling = ceiling.as_ref().map(|(c, _)| c.inherit_env.clone());
    let ceiling_digest = if let Some((c, digest)) = ceiling {
        ceiling_rules(&mut current, &c, &workspace.path)?;
        record(&mut provenance, "system-ceiling", None, &current)?;
        Some(digest)
    } else {
        None
    };
    let digest = hash(&(&workspace, &current, &provenance, &ceiling_digest))?;
    Ok(EffectivePolicy {
        workspace,
        rules: current,
        provenance,
        ceiling_digest,
        environment_ceiling,
        digest,
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct Transition {
    previous: String,
    proposed: String,
    workspace: WorkspaceIdentity,
    requires_confirmation: bool,
    digest: String,
}
impl Transition {
    pub fn requires_confirmation(&self) -> bool {
        self.requires_confirmation
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// Confirms metadata only; it is not an execution/dispatch permit.
    pub fn confirm(&self, digest: &str) -> Result<()> {
        if digest != self.digest {
            return Err(Error::Transition);
        }
        Ok(())
    }
}
pub fn transition(previous: &EffectivePolicy, proposed: &EffectivePolicy) -> Result<Transition> {
    if previous.workspace != proposed.workspace {
        return Err(Error::Transition);
    }
    let old = &previous.rules;
    let new = &proposed.rules;
    let added_roots =
        |a: &[PathBuf], b: &[PathBuf]| a.iter().any(|p| !b.iter().any(|q| p.starts_with(q)));
    let requires_confirmation = rank(new.access) > rank(old.access)
        || (new.unattended == UnattendedApprovalMode::Allow
            && old.unattended == UnattendedApprovalMode::Deny)
        || added_roots(&new.read_roots, &old.read_roots)
        || added_roots(&new.write_roots, &old.write_roots)
        || old
            .deny_commands
            .iter()
            .any(|x| !new.deny_commands.contains(x))
        || new.inherit_env.iter().any(|x| !old.inherit_env.contains(x));
    let digest = hash(&(
        &previous.digest,
        &proposed.digest,
        &previous.workspace,
        requires_confirmation,
    ))?;
    Ok(Transition {
        previous: previous.digest.clone(),
        proposed: proposed.digest.clone(),
        workspace: previous.workspace.clone(),
        requires_confirmation,
        digest,
    })
}
#[cfg(test)]
mod tests;

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn resolve_test_source(
    workspace: &Path,
    base: &Rules,
    root: &Path,
) -> Result<EffectivePolicy> {
    let ceiling = ceiling::test_load(root)?;
    resolve_loaded_inner(workspace, base, &[], ceiling.as_ref(), true)
}
