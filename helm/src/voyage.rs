//! Private, durable voyage plans. These records grant no execution or sharing authority.
use crate::attachment::local_actor::storage::Directory;
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const SCHEMA: u32 = 1;
const MAX_BYTES: usize = 65_536;
const MAX_DRAFTS: usize = 256;

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub name: String,
    pub purpose: String,
    pub participants: Vec<Uuid>,
    pub coordinator: Option<Uuid>,
}

impl Draft {
    /// Incomplete form edits remain in memory; only reviewed, complete plans save.
    pub fn validate(&self) -> Result<()> {
        label(&self.name)?;
        ensure!(
            self.purpose.len() <= 8192
                && !self
                    .purpose
                    .chars()
                    .any(|c| (c.is_control() && !matches!(c, '\n' | '\t')) || bidi(c)),
            "purpose must be at most 8192 bytes without terminal controls"
        );
        ensure!(
            (1..=64).contains(&self.participants.len()),
            "select between 1 and 64 Helms"
        );
        let unique: BTreeSet<_> = self.participants.iter().copied().collect();
        ensure!(
            unique.len() == self.participants.len() && !unique.contains(&Uuid::nil()),
            "selected Helms must be distinct valid identities"
        );
        ensure!(
            self.coordinator.is_some_and(|id| unique.contains(&id)),
            "select a coordinator from the selected Helms"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredDraft {
    pub id: Uuid,
    /// Zero denotes a new record; successful saves advance this revision.
    pub revision: u64,
    pub updated_at: DateTime<Utc>,
    pub draft: Draft,
    pub origin: Option<String>,
    pub helm_labels: BTreeMap<Uuid, String>,
}

impl StoredDraft {
    pub fn new(draft: Draft, origin: Option<String>, helm_labels: BTreeMap<Uuid, String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            revision: 0,
            updated_at: Utc::now(),
            draft,
            origin,
            helm_labels,
        }
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_nil() && self.revision < i64::MAX as u64,
            "invalid draft identity or revision"
        );
        self.draft.validate()?;
        if let Some(origin) = &self.origin {
            ensure!(
                crate::attachment::origin::validate_origin(origin, true)
                    .is_ok_and(|value| value == *origin),
                "invalid Vessel origin"
            );
        }
        ensure!(self.helm_labels.len() <= 64, "too many Helm labels");
        for (id, name) in &self.helm_labels {
            ensure!(
                self.draft.participants.contains(id),
                "label belongs to an unselected Helm"
            );
            label(name)?;
        }
        Ok(())
    }
    fn same_content(&self, other: &Self) -> bool {
        self.id == other.id
            && self.draft == other.draft
            && self.origin == other.origin
            && self.helm_labels == other.helm_labels
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    value: StoredDraft,
}

/// An owner-private store. Busy locks fail promptly, and stale writes never replace newer edits.
pub struct Store {
    directory: Directory,
    path: PathBuf,
}

pub fn default_path() -> PathBuf {
    crate::config::default_data_dir().join("voyage-drafts")
}

impl Store {
    /// The parent must already exist. No symlinked directory components are accepted.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let directory = Directory::open(path)?;
        drop(directory.lock()?);
        Ok(Self {
            directory,
            path: path.into(),
        })
    }

    pub fn load(&self, id: Uuid) -> Result<Option<StoredDraft>> {
        let _lock = self.directory.read_lock()?;
        self.load_locked(id)
    }

    fn load_locked(&self, id: Uuid) -> Result<Option<StoredDraft>> {
        ensure!(!id.is_nil(), "invalid draft identity");
        let Some(bytes) = self.directory.read_bounded(&filename(id), MAX_BYTES)? else {
            return Ok(None);
        };
        let record: Record = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("invalid voyage draft record"))?;
        ensure!(record.schema == SCHEMA, "unsupported voyage draft schema");
        ensure!(
            record.value.id == id && record.value.revision > 0,
            "invalid stored draft identity or revision"
        );
        record.value.validate()?;
        Ok(Some(record.value))
    }

    fn ids_locked(&self) -> Result<Vec<Uuid>> {
        self.directory.verify()?;
        let mut ids = Vec::new();
        // Directory entry names are untrusted. Contents are always read through
        // the pinned, verified private directory, never through entry paths.
        for (index, entry) in std::fs::read_dir(&self.path)?.enumerate() {
            ensure!(index < 4096, "voyage draft directory has too many entries");
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if let Some(stem) = name.strip_suffix(".json") {
                let id = Uuid::parse_str(stem).context("invalid voyage draft filename")?;
                ensure!(filename(id) == name, "noncanonical voyage draft filename");
                ids.push(id);
                ensure!(ids.len() <= MAX_DRAFTS, "voyage draft capacity exceeded");
            }
        }
        self.directory.verify()?;
        Ok(ids)
    }

    pub fn list(&self) -> Result<Vec<StoredDraft>> {
        let _lock = self.directory.read_lock()?;
        let mut result = Vec::new();
        for id in self.ids_locked()? {
            result.push(self.load_locked(id)?.context("voyage draft disappeared")?);
        }
        result.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(result)
    }

    /// Retain the submitted record after errors: retrying identical content is
    /// safe even when publication succeeded but its acknowledgment was lost.
    pub fn save(&self, value: &StoredDraft) -> Result<StoredDraft> {
        value.validate()?;
        let _lock = self.directory.lock()?;
        let current = self.load_locked(value.id)?;
        if let Some(current) = &current {
            if current.revision == value.revision + 1 && current.same_content(value) {
                self.directory.sync_file(&filename(value.id))?;
                self.directory.sync()?;
                return Ok(current.clone());
            }
            ensure!(
                current.revision == value.revision,
                "voyage draft changed; reload before saving"
            );
        } else {
            ensure!(value.revision == 0, "voyage draft no longer exists");
            ensure!(
                self.ids_locked()?.len() < MAX_DRAFTS,
                "voyage draft capacity reached"
            );
        }
        let mut saved = value.clone();
        saved.revision += 1;
        saved.updated_at = Utc::now();
        saved.validate()?;
        let bytes = serde_json::to_vec(&Record {
            schema: SCHEMA,
            value: saved.clone(),
        })?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "voyage draft exceeds storage limit"
        );
        self.directory.publish(&filename(saved.id), &bytes)?;
        Ok(saved)
    }
}

fn filename(id: Uuid) -> String {
    format!("{id}.json")
}
fn bidi(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}
fn label(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty()
            && value.len() <= 256
            && !value.chars().any(|c| c.is_control() || bidi(c)),
        "name must contain 1–256 bytes without terminal controls"
    );
    Ok(())
}
