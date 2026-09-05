//! Persistent local admission attribution, independent of enrollment or authority.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

pub(crate) mod storage;

/// Stable attribution within one explicitly selected Helm installation.
/// Neither identifier authenticates a caller or grants permission.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalActor {
    pub installation_id: Uuid,
    pub principal_id: Uuid,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    actor: LocalActor,
}

const MAX_BYTES: usize = 1024;
const CANDIDATE: &str = "candidate.json";
const PUBLISHED: &str = "actor.json";

/// A verified private installation directory, not an enrollment or session lease.
/// Initialization is serialized; keeping this handle does not block other sessions.
pub struct LocalActorStore {
    directory: storage::Directory,
    actor: LocalActor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Boundary {
    DirectoryReady,
    CandidateCreated,
    CandidateWritten,
    CandidateDurable,
    BeforePublish,
    Published,
    Durable,
    Verified,
}

impl LocalActorStore {
    /// Open a dedicated directory whose parent already exists. No default path,
    /// enrollment, identity rotation, or operator-data migration is implicit.
    pub fn open(directory: &Path) -> Result<Self> {
        Self::open_with(directory, |_| Ok(()))
    }

    fn open_with(
        directory: &Path,
        mut boundary: impl FnMut(Boundary) -> Result<()>,
    ) -> Result<Self> {
        let directory = storage::Directory::open(directory)?;
        let _lock = directory.lock()?;
        boundary(Boundary::DirectoryReady)?;
        let published = directory.read(PUBLISHED)?;
        let candidate = directory.read(CANDIDATE)?;
        let (actor, bytes) = match (published, candidate) {
            (Some(published), candidate) => {
                let actor = decode(&published)?;
                if let Some(candidate) = candidate {
                    ensure!(
                        decode(&candidate)? == actor && candidate == published,
                        "local actor candidate conflicts with published identity"
                    );
                }
                // Re-establish the supported durability barrier even when a prior
                // publication returned an uncertain result. Never rewrite either file.
                directory.sync_file(PUBLISHED)?;
                directory.sync()?;
                directory.verify()?;
                return Ok(Self { directory, actor });
            }
            (None, Some(candidate)) => (decode(&candidate)?, candidate),
            (None, None) => {
                let actor = LocalActor {
                    installation_id: Uuid::new_v4(),
                    principal_id: Uuid::new_v4(),
                };
                let bytes = serde_json::to_vec(&Record { version: 1, actor })?;
                let mut file = directory.create(CANDIDATE)?;
                boundary(Boundary::CandidateCreated)?;
                use std::io::Write;
                file.write_all(&bytes)?;
                boundary(Boundary::CandidateWritten)?;
                file.sync_all()?;
                directory.sync()?;
                (actor, bytes)
            }
        };
        directory.sync_file(CANDIDATE)?;
        directory.sync()?;
        boundary(Boundary::CandidateDurable)?;
        directory.verify()?;
        boundary(Boundary::BeforePublish)?;
        ensure!(
            directory.read(CANDIDATE)?.as_deref() == Some(bytes.as_slice()),
            "local actor candidate changed before publication"
        );
        directory.publish_new(PUBLISHED, &bytes)?;
        boundary(Boundary::Published)?;
        directory.sync()?;
        boundary(Boundary::Durable)?;
        ensure!(
            directory.read(PUBLISHED)?.as_deref() == Some(bytes.as_slice()),
            "local actor publication verification failed"
        );
        ensure!(
            directory.read(CANDIDATE)?.as_deref() == Some(bytes.as_slice()),
            "local actor candidate changed during publication"
        );
        directory.verify()?;
        boundary(Boundary::Verified)?;
        Ok(Self { directory, actor })
    }

    /// Revalidate the pinned location and published record before attributing a
    /// local command. Callers still enforce their own authorization and policy.
    pub fn identity(&self) -> Result<LocalActor> {
        self.directory.verify()?;
        let bytes = self
            .directory
            .read(PUBLISHED)?
            .context("local actor identity missing")?;
        ensure!(
            decode(&bytes)? == self.actor,
            "local actor identity changed"
        );
        self.directory.verify()?;
        Ok(self.actor)
    }
}

fn decode(bytes: &[u8]) -> Result<LocalActor> {
    ensure!(bytes.len() <= MAX_BYTES, "local actor record exceeds limit");
    let record: Record = serde_json::from_slice(bytes).context("invalid local actor record")?;
    ensure!(record.version == 1, "unsupported local actor version");
    ensure!(
        !record.actor.installation_id.is_nil()
            && !record.actor.principal_id.is_nil()
            && record.actor.installation_id != record.actor.principal_id,
        "invalid local actor identifiers"
    );
    Ok(record.actor)
}

#[cfg(test)]
mod tests;
