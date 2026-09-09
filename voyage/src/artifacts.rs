//! Private immutable session-owned tool artifacts. MIME types are declarations, not decoder validation.
use anyhow::{Result, bail, ensure};
use base64::Engine;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;
use voyage_protocol::tool_result::{ArtifactReference, ToolContent, ToolOutput};

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_DATABASE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARTIFACTS: u64 = 256;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

fn label_valid(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{206f}'))
}
fn validate_metadata(name: &str, mime: &str) -> Result<()> {
    ensure!(
        label_valid(name, 255) && !name.contains(['/', '\\']),
        "invalid artifact display name"
    );
    ensure!(
        label_valid(mime, 127)
            && mime.is_ascii()
            && mime.contains('/')
            && !mime.bytes().any(|b| b.is_ascii_whitespace()),
        "invalid artifact MIME type"
    );
    Ok(())
}
type StoredUpload = (String, String, String, Vec<u8>, Vec<u8>);

pub struct Store {
    connection: Connection,
    session: Uuid,
    #[cfg(unix)]
    path: std::path::PathBuf,
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(unix)]
    directory: std::fs::File,
}
impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtifactStore")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl Store {
    #[cfg(not(unix))]
    pub fn open(_directory: &Path, _session: Uuid) -> Result<Self> {
        bail!("private artifact storage unsupported on this platform")
    }

    #[cfg(unix)]
    pub fn open(directory: &Path, session: Uuid) -> Result<Self> {
        use crate::attachment::journal::{open_private_file, prepare_directory};
        ensure!(!session.is_nil(), "invalid artifact store session");
        let journal = prepare_directory(directory.to_path_buf())?;
        let dir = prepare_directory(journal.join("artifacts"))?;
        let directory = std::fs::File::open(&dir)?;
        let path = dir.join("artifacts.sqlite3");
        let file = open_private_file(&path)?;
        ensure!(
            file.metadata()?.len() <= MAX_DATABASE_BYTES,
            "artifact database size limit exceeded"
        );
        check_sidecars(&path)?;
        let connection = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        let mut store = Self {
            connection,
            session,
            path,
            file,
            directory,
        };
        store.check_files()?;
        store
            .connection
            .busy_timeout(std::time::Duration::from_secs(5))?;
        store.connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;")?;
        let page_size: u64 = store
            .connection
            .query_row("PRAGMA page_size", [], |r| r.get(0))?;
        ensure!(page_size > 0, "invalid artifact database page size");
        store
            .connection
            .pragma_update(None, "max_page_count", MAX_DATABASE_BYTES / page_size)?;
        let tx = store
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS scope (singleton INTEGER PRIMARY KEY CHECK(singleton=1), session TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS artifacts (id TEXT PRIMARY KEY, session TEXT NOT NULL, principal TEXT NOT NULL,
                metadata TEXT NOT NULL, digest BLOB NOT NULL CHECK(length(digest)=32), bytes BLOB NOT NULL CHECK(length(bytes) BETWEEN 1 AND 4194304));")?;
        tx.execute(
            "INSERT OR IGNORE INTO scope VALUES (1, ?1)",
            [session.to_string()],
        )?;
        let owner: String =
            tx.query_row("SELECT session FROM scope WHERE singleton=1", [], |r| {
                r.get(0)
            })?;
        ensure!(
            owner == session.to_string(),
            "artifact store belongs to another session"
        );
        let (count, size): (u64, u64) = tx.query_row(
            "SELECT count(*), coalesce(sum(length(bytes)),0) FROM artifacts",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            count <= MAX_ARTIFACTS && size <= MAX_DATABASE_BYTES,
            "artifact store capacity exceeded"
        );
        tx.commit()?;
        store.check_files()?;
        store.directory.sync_all()?;
        std::fs::File::open(journal)?.sync_all()?;
        Ok(store)
    }

    #[cfg(unix)]
    fn check_files(&self) -> Result<()> {
        use crate::attachment::journal::{open_private_file, prepare_directory};
        use std::os::unix::fs::MetadataExt;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid artifact storage"))?;
        prepare_directory(parent.to_path_buf())?;
        let now = std::fs::symlink_metadata(parent)?;
        let original = self.directory.metadata()?;
        ensure!(
            now.dev() == original.dev() && now.ino() == original.ino(),
            "artifact storage directory changed"
        );
        let now = open_private_file(&self.path)?.metadata()?;
        let original = self.file.metadata()?;
        ensure!(
            now.dev() == original.dev() && now.ino() == original.ino(),
            "artifact database changed"
        );
        ensure!(
            now.len() <= MAX_DATABASE_BYTES,
            "artifact database size limit exceeded"
        );
        check_sidecars(&self.path)
    }
    #[cfg(not(unix))]
    fn check_files(&self) -> Result<()> {
        bail!("private artifact storage unsupported on this platform")
    }

    pub fn put(
        &mut self,
        id: Uuid,
        name: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<ArtifactReference> {
        self.check_files()?;
        let principal = self.session;
        validate_metadata(name, mime_type)?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_BYTES,
            "artifact byte limit exceeded or empty artifact"
        );
        let metadata = ArtifactReference {
            id,
            sha256: hex::encode(Sha256::digest(bytes)),
            name: name.to_owned(),
            mime_type: mime_type.to_owned(),
            byte_size: bytes.len() as u64,
        };
        ensure!(!id.is_nil(), "invalid artifact identifier");
        let encoded = serde_json::to_string(&metadata)?;
        let digest = Sha256::digest(bytes).to_vec();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<StoredUpload> = tx
            .query_row(
                "SELECT session, principal, metadata, digest, bytes FROM artifacts WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some((session, owner, meta, hash, data)) = existing {
            ensure!(
                session == self.session.to_string()
                    && owner == principal.to_string()
                    && meta == encoded
                    && hash == digest
                    && data == bytes,
                "artifact identifier conflicts with immutable attachment"
            );
        } else {
            let (count, size): (u64, u64) = tx.query_row(
                "SELECT count(*), coalesce(sum(length(bytes)),0) FROM artifacts",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            ensure!(
                count < MAX_ARTIFACTS
                    && size
                        .checked_add(bytes.len() as u64)
                        .is_some_and(|n| n <= MAX_DATABASE_BYTES),
                "artifact store capacity exceeded"
            );
            tx.execute("INSERT INTO artifacts (id, session, principal, metadata, digest, bytes) VALUES (?1,?2,?3,?4,?5,?6)",
                params![id.to_string(), self.session.to_string(), principal.to_string(), encoded, digest, bytes])?;
        }
        tx.commit()?;
        self.check_files()?;
        Ok(metadata)
    }

    fn load(&self, attachment: &ArtifactReference) -> Result<(Uuid, Vec<u8>)> {
        self.check_files()?;
        validate_metadata(&attachment.name, &attachment.mime_type)?;
        let row: Option<(String, String, Vec<u8>, Vec<u8>)> = self.connection.query_row(
            "SELECT principal, metadata, digest, bytes FROM artifacts WHERE id=?1 AND session=?2 AND length(bytes) BETWEEN 1 AND 4194304",
            params![attachment.id.to_string(), self.session.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
        let (principal, encoded, digest, bytes) =
            row.ok_or_else(|| anyhow::anyhow!("artifact attachment unavailable in this session"))?;
        let stored: ArtifactReference = serde_json::from_str(&encoded)
            .map_err(|_| anyhow::anyhow!("invalid stored artifact metadata"))?;
        ensure!(
            &stored == attachment && attachment.byte_size == bytes.len() as u64,
            "artifact metadata integrity failure"
        );
        ensure!(
            Sha256::digest(&bytes).as_slice() == digest
                && hex::encode(&digest) == attachment.sha256,
            "artifact content integrity failure"
        );
        let principal = Uuid::parse_str(&principal)
            .map_err(|_| anyhow::anyhow!("invalid artifact ownership"))?;
        ensure!(!principal.is_nil(), "invalid artifact ownership");
        Ok((principal, bytes))
    }

    pub fn resolve(&self, attachment: &ArtifactReference) -> Result<Vec<u8>> {
        self.load(attachment).map(|(_, bytes)| bytes)
    }

    /// Branch copying preserves immutable identifiers and metadata in the destination session.
    pub fn copy_to(
        &self,
        dest: &mut Store,
        metadata: &ArtifactReference,
    ) -> Result<ArtifactReference> {
        let (_, bytes) = self.load(metadata)?;
        dest.put(metadata.id, &metadata.name, &metadata.mime_type, &bytes)
    }
}

#[cfg(unix)]
fn check_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = std::path::PathBuf::from(name);
        match std::fs::symlink_metadata(&sidecar) {
            Ok(_) => {
                crate::attachment::journal::open_private_file(&sidecar)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => bail!("cannot inspect artifact database sidecar"),
        }
    }
    Ok(())
}

/// Authority supplied by the session owner, never by a tool or model argument.
#[derive(Clone, Debug)]
pub struct Scope {
    pub directory: std::path::PathBuf,
    pub session: Uuid,
}

impl Store {
    /// Callers must authorize access to this session before resolving identifiers.
    pub fn get(&self, id: Uuid) -> Result<(ArtifactReference, Vec<u8>)> {
        self.check_files()?;
        let encoded: Option<String> = self
            .connection
            .query_row(
                "SELECT metadata FROM artifacts WHERE id=?1 AND session=?2",
                params![id.to_string(), self.session.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let metadata: ArtifactReference = serde_json::from_str(
            &encoded.ok_or_else(|| anyhow::anyhow!("artifact unavailable in this session"))?,
        )
        .map_err(|_| anyhow::anyhow!("invalid stored artifact metadata"))?;
        let bytes = self.resolve(&metadata)?;
        Ok((metadata, bytes))
    }

    /// Bounded download after the caller has authorized observation of this session.
    pub fn chunk(&self, id: Uuid, offset: u64, limit: usize) -> Result<Value> {
        ensure!(
            (1..=65536).contains(&limit),
            "artifact chunk limit must be between 1 and 65536 bytes"
        );
        let (metadata, bytes) = self.get(id)?;
        ensure!(
            offset <= bytes.len() as u64,
            "artifact chunk offset exceeds artifact size"
        );
        let start = usize::try_from(offset)
            .map_err(|_| anyhow::anyhow!("invalid artifact chunk offset"))?;
        let end = start.saturating_add(limit).min(bytes.len());
        Ok(serde_json::json!({
            "metadata": metadata,
            "data_base64": base64::engine::general_purpose::STANDARD.encode(&bytes[start..end]),
            "offset": offset,
            "next_offset": end,
            "eof": end == bytes.len(),
        }))
    }

    fn ingest_blob(&mut self, data: &str, mime: &str, name: &str) -> Result<ArtifactReference> {
        ensure!(
            data.len() <= MAX_BYTES.div_ceil(3) * 4,
            "artifact encoded byte limit exceeded"
        );
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| anyhow::anyhow!("invalid artifact base64"))?;
        validate_metadata(name, mime)?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_BYTES,
            "artifact byte limit exceeded or empty artifact"
        );
        if name == "tool-image" {
            let (actual, _, _) = crate::images::validate(&bytes)?;
            ensure!(
                serde_json::to_value(actual)?.as_str() == Some(mime),
                "MCP image MIME does not match raster"
            );
        }
        self.check_files()?;
        let digest = Sha256::digest(&bytes).to_vec();
        let mut query = self
            .connection
            .prepare("SELECT metadata FROM artifacts WHERE session=?1 AND digest=?2")?;
        let candidates = query
            .query_map(params![self.session.to_string(), digest], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(query);
        for candidate in candidates {
            let meta: ArtifactReference = serde_json::from_str(&candidate)
                .map_err(|_| anyhow::anyhow!("invalid stored artifact metadata"))?;
            if meta.name == name && meta.mime_type == mime {
                ensure!(
                    self.resolve(&meta)? == bytes,
                    "artifact content integrity failure"
                );
                return Ok(meta);
            }
        }
        self.put(Uuid::new_v4(), name, mime, &bytes)
    }

    /// Preserve all supported MCP blocks in order. Never dereference resource URIs.
    /// Unsupported blocks fail explicitly instead of silently dropping information.
    /// A failed ingestion can retain bounded unreferenced blobs until session deletion.
    pub fn ingest_mcp(&mut self, value: &Value) -> Result<ToolOutput> {
        self.ingest_mcp_inner(value).map_err(|_| {
            anyhow::anyhow!(
                "MCP result is invalid, unsupported, or exceeds artifact storage limits"
            )
        })
    }

    fn ingest_mcp_inner(&mut self, value: &Value) -> Result<ToolOutput> {
        ensure!(
            serde_json::to_vec(value)?.len() <= MAX_RESULT_BYTES,
            "MCP result byte limit exceeded"
        );
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("invalid MCP result"))?;
        let blocks = object
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("invalid MCP content"))?;
        ensure!(blocks.len() <= 128, "MCP content count limit exceeded");
        let is_error = match object.get("isError") {
            None => false,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("invalid MCP error flag"))?,
        };
        let structured_content = object.get("structuredContent").cloned();
        ensure!(
            structured_content.as_ref().is_none_or(Value::is_object),
            "invalid MCP structured content"
        );
        let mut content = Vec::with_capacity(blocks.len());
        for block in blocks {
            let part = match string(block, "type")? {
                "text" => ToolContent::Text {
                    text: string(block, "text")?.to_owned(),
                },
                kind @ ("image" | "audio") => {
                    let mime = string(block, "mimeType")?;
                    ensure!(
                        mime.starts_with(&format!("{kind}/")),
                        "MCP content MIME mismatch"
                    );
                    let artifact =
                        self.ingest_blob(string(block, "data")?, mime, &format!("tool-{kind}"))?;
                    if kind == "image" {
                        ToolContent::Image { artifact }
                    } else {
                        ToolContent::Audio { artifact }
                    }
                }
                "resource" => {
                    let resource = block
                        .get("resource")
                        .ok_or_else(|| anyhow::anyhow!("invalid embedded resource"))?;
                    let uri = bounded_string(resource, "uri", 8192)?;
                    let mime_type = optional_string(resource, "mimeType", 127)?;
                    let text = optional_string(resource, "text", MAX_RESULT_BYTES)?;
                    let artifact = match resource.get("blob") {
                        None => None,
                        Some(v) => Some(
                            self.ingest_blob(
                                v.as_str()
                                    .ok_or_else(|| anyhow::anyhow!("invalid resource blob"))?,
                                mime_type.as_deref().unwrap_or("application/octet-stream"),
                                "tool-resource",
                            )?,
                        ),
                    };
                    ensure!(
                        text.is_some() ^ artifact.is_some(),
                        "embedded resource needs exactly one payload"
                    );
                    ToolContent::Resource {
                        uri,
                        mime_type,
                        text,
                        artifact,
                    }
                }
                "resource_link" => {
                    let size = match block.get("size") {
                        None => None,
                        Some(v) => Some(
                            v.as_u64()
                                .ok_or_else(|| anyhow::anyhow!("invalid resource size"))?,
                        ),
                    };
                    ToolContent::ResourceLink {
                        uri: bounded_string(block, "uri", 8192)?,
                        name: bounded_string(block, "name", 255)?,
                        description: optional_string(block, "description", 8192)?,
                        mime_type: optional_string(block, "mimeType", 127)?,
                        size,
                    }
                }
                _ => bail!("unsupported MCP content type"),
            };
            content.push(part);
        }
        Ok(ToolOutput {
            content,
            structured_content,
            is_error,
        })
    }
}

fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("invalid MCP string field"))
}
fn bounded_string(value: &Value, field: &str, max: usize) -> Result<String> {
    let s = string(value, field)?;
    ensure!(label_valid(s, max), "invalid MCP metadata string");
    Ok(s.to_owned())
}
fn optional_string(value: &Value, field: &str, max: usize) -> Result<Option<String>> {
    match value.get(field) {
        None => Ok(None),
        Some(v) => {
            let s = v
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid MCP string field"))?;
            ensure!(s.len() <= max, "MCP string byte limit exceeded");
            Ok(Some(s.to_owned()))
        }
    }
}
