//! Versioned packages. Installation never runs code; format 1 remains declarative.
mod catalog;
pub mod cli;
pub(crate) mod executable;
mod index;
mod private_files;
pub(crate) mod runtime;
pub use private_files::PrivateFile;
mod store;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_ARCHIVE: usize = 65_536;
const MAX_FILES: usize = 32;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub id: String,
    pub version: String,
    /// Exact supported Helm major.minor line, e.g. "0.1".
    pub helm: String,
    pub capabilities: Vec<String>,
    pub contents: Vec<Content>,
    pub entrypoints: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub path: String,
    pub kind: Kind,
    pub sha256: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Skill,
    Resource,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    pub manifest: Manifest,
    #[serde(deserialize_with = "unique_map")]
    pub files: BTreeMap<String, String>,
}
#[derive(Clone, Debug)]
pub(crate) enum Package {
    Declarative(Archive),
    Executable(executable::Archive),
}
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum PackageManifest {
    Declarative(Manifest),
    Executable(executable::Manifest),
}
impl Package {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= executable::MAX_ARCHIVE,
            "package exceeds archive limit"
        );
        #[derive(Deserialize)]
        struct Header {
            manifest: Format,
        }
        #[derive(Deserialize)]
        struct Format {
            format: u32,
        }
        let header: Header = serde_json::from_slice(bytes)
            .map_err(|_| anyhow::anyhow!("invalid package format header"))?;
        // Parse original bytes, not a Value round-trip which could erase duplicate keys.
        match header.manifest.format {
            1 => Ok(Self::Declarative(Archive::parse(bytes)?)),
            2 => Ok(Self::Executable(executable::Archive::parse(bytes)?)),
            _ => anyhow::bail!("unsupported package format"),
        }
    }
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Declarative(a) => &a.manifest.id,
            Self::Executable(a) => &a.manifest.id,
        }
    }
    pub(crate) fn manifest(&self) -> PackageManifest {
        match self {
            Self::Declarative(a) => PackageManifest::Declarative(a.manifest.clone()),
            Self::Executable(a) => PackageManifest::Executable(a.manifest.clone()),
        }
    }
}
fn version(value: &str) -> Result<()> {
    let parts: Vec<_> = value.split('.').collect();
    ensure!(
        parts.len() == 3
            && parts.iter().all(|p| !p.is_empty()
                && p.len() <= 6
                && p.bytes().all(|b| b.is_ascii_digit())
                && (*p == "0" || !p.starts_with('0'))),
        "version must be numeric major.minor.patch"
    );
    Ok(())
}
pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn portable(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 160
        && path.split('/').all(|p| {
            !p.is_empty()
                && p.len() <= 64
                && !p.starts_with('.')
                && !p.ends_with('.')
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
                && !matches!(
                    p.split('.').next().unwrap(),
                    "con"
                        | "prn"
                        | "aux"
                        | "nul"
                        | "com1"
                        | "com2"
                        | "com3"
                        | "com4"
                        | "com5"
                        | "com6"
                        | "com7"
                        | "com8"
                        | "com9"
                        | "lpt1"
                        | "lpt2"
                        | "lpt3"
                        | "lpt4"
                        | "lpt5"
                        | "lpt6"
                        | "lpt7"
                        | "lpt8"
                        | "lpt9"
                )
        })
}
impl Archive {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_ARCHIVE, "package exceeds 64 KiB");
        let archive: Self =
            serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("invalid package JSON"))?;
        archive.validate()?;
        Ok(archive)
    }
    pub fn validate(&self) -> Result<()> {
        let m = &self.manifest;
        ensure!(
            m.format == 1 && identifier(&m.id),
            "unsupported package format or identity"
        );
        version(&m.version)?;
        let line = env!("CARGO_PKG_VERSION").rsplit_once('.').unwrap().0;
        ensure!(
            m.helm == line,
            "package requires another Helm compatibility line"
        );
        ensure!(
            m.capabilities == ["model_context"],
            "only model_context capability is supported; executable extensions are unavailable"
        );
        ensure!(
            !m.contents.is_empty()
                && m.contents.len() <= MAX_FILES
                && self.files.len() == m.contents.len(),
            "invalid package file count"
        );
        let mut seen = BTreeSet::new();
        for c in &m.contents {
            ensure!(
                portable(&c.path) && seen.insert(&c.path) && hash(&c.sha256),
                "invalid, duplicate or nonportable content path/hash"
            );
            let text = self
                .files
                .get(&c.path)
                .ok_or_else(|| anyhow::anyhow!("missing package content"))?;
            ensure!(
                digest(text.as_bytes()) == c.sha256 && !text.contains('\0'),
                "package content integrity or text failure"
            );
        }
        let mut entries = BTreeSet::new();
        for path in &m.entrypoints {
            ensure!(
                entries.insert(path)
                    && m.contents
                        .iter()
                        .any(|c| &c.path == path && c.kind == Kind::Skill),
                "entrypoint must name one declared skill"
            );
        }
        ensure!(m.entrypoints.len() <= MAX_FILES, "too many entrypoints");
        Ok(())
    }
}

/// The immutable per-run snapshot is consumed only as ephemeral provider guidance.
/// Broken catalog/grants isolate packages; they never prevent ordinary chat startup.
pub(crate) fn guidance(workspace: &std::path::Path) -> String {
    match catalog::Catalog::new(workspace, &crate::config::default_data_dir())
        .and_then(|c| c.guidance())
    {
        Ok(text) => text,
        Err(_) => {
            tracing::warn!(
                "extension guidance unavailable; inspect package catalog and activation records"
            );
            String::new()
        }
    }
}

// Map duplicate keys must be rejected before serde's BTreeMap can overwrite them.
fn unique_map<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique string map")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut entries = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if entries.len() >= 128 || entries.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate or oversized map"));
                }
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_map(Visitor)
}

#[cfg(test)]
mod package_tests;
