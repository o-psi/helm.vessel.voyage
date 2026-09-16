//! Durable composer storage, independent from Voyage admission and conversation.
use super::{access::store, database, service::Supervisor};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use voyage_protocol::{
    content::{ContentPart, ImageAttachment},
    drafts::{Draft, DraftDocument, DraftOperation, DraftTarget},
    process::{ConnectionGrant, GrantBinding, ProcessGrant, ProcessRight},
    vessel::{VoyageCommand, VoyageReply, VoyageRequest},
};

pub(super) enum Scope {
    Owner,
    Connection(ConnectionGrant),
    Session(ProcessGrant),
}
impl Scope {
    fn key(&self) -> String {
        match self {
            Self::Owner => "owner".into(),
            Self::Connection(g) if g.full_access => "owner".into(),
            Self::Connection(g) => format!("connection:{}:{}", g.grant_id, g.revision),
            Self::Session(g) => format!("session:{}:{}", g.grant_id, g.revision),
        }
    }
    fn current(&self, root: &std::path::Path) -> Result<()> {
        match self {
            Self::Owner => Ok(()),
            Self::Connection(g) => store::current_connection(root, g),
            Self::Session(g) => {
                // Draft discovery is a human client surface, not delegated worker authority.
                ensure!(
                    g.parent_grant.is_none()
                        && g.participant_binding.is_none()
                        && g.connection_binding.is_none(),
                    "delegated grants cannot access composer drafts"
                );
                let current: ProcessGrant = store::load(&store::grant_path(root, g.grant_id))?;
                store::current(&current)?;
                ensure!(
                    current.revision == g.revision && current.principal_id == g.principal_id,
                    "draft authority revoked"
                );
                Ok(())
            }
        }
    }
    fn right(&self, right: ProcessRight) -> Result<()> {
        ensure!(
            match self {
                Self::Owner => true,
                Self::Connection(g) => g.rights.contains(&right),
                Self::Session(g) => g.rights.contains(&right),
            },
            "draft permission denied"
        );
        Ok(())
    }
}
fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS composer_drafts(scope TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL,document TEXT,updated INTEGER NOT NULL,PRIMARY KEY(scope,id));
CREATE TABLE IF NOT EXISTS composer_images(scope TEXT NOT NULL,draft TEXT NOT NULL,id TEXT NOT NULL,metadata TEXT NOT NULL,bytes BLOB NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(scope,draft,id));
CREATE TABLE IF NOT EXISTS composer_receipts(scope TEXT NOT NULL,id TEXT NOT NULL,fingerprint TEXT NOT NULL,result TEXT,plan TEXT,created INTEGER NOT NULL,PRIMARY KEY(scope,id));")?;
    Ok(())
}
fn load(db: &Connection, scope: &str, id: Uuid) -> Result<Option<Draft>> {
    let row: Option<(u64, Option<String>, u64)> = db
        .query_row(
            "SELECT revision,document,updated FROM composer_drafts WHERE scope=?1 AND id=?2",
            params![scope, id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    match row {
        Some((revision, Some(document), updated_at_ms)) => Ok(Some(Draft {
            draft_id: id,
            revision,
            document: serde_json::from_str(&document)?,
            updated_at_ms,
        })),
        _ => Ok(None),
    }
}
fn image(
    db: &Connection,
    scope: &str,
    draft: Uuid,
    id: Uuid,
) -> Result<(ImageAttachment, Vec<u8>)> {
    let (meta, bytes): (String, Vec<u8>) = db.query_row(
        "SELECT metadata,bytes FROM composer_images WHERE scope=?1 AND draft=?2 AND id=?3",
        params![scope, draft.to_string(), id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((serde_json::from_str(&meta)?, bytes))
}
fn validate(db: &Connection, scope: &str, id: Uuid, document: &DraftDocument) -> Result<()> {
    ensure!(document.parts.len() <= 16, "draft has too many parts");
    // Only the dispatch path requires meaningful content; drafts preserve whitespace.
    match voyage_protocol::content::validate_content(
        &document.parts,
        voyage_protocol::content::ContentLimits {
            max_parts: 16,
            max_text_bytes: 64 * 1024,
            max_images: 4,
            max_image_bytes: 2 * 1024 * 1024,
            max_total_image_bytes: 2 * 1024 * 1024,
            max_dimension: 8192,
            max_pixels: 20_000_000,
        },
        true,
    ) {
        Ok(()) | Err(voyage_protocol::content::ContentError::Empty) => (),
        Err(error) => return Err(error.into()),
    }
    for part in &document.parts {
        if let ContentPart::Image { attachment } = part {
            ensure!(
                image(db, scope, id, attachment.id)?.0 == *attachment,
                "staged image metadata mismatch"
            );
        }
    }
    Ok(())
}
impl Supervisor {
    async fn draft_target(&self, scope: &Scope, target: &DraftTarget) -> Result<()> {
        scope.current(&self.directory)?;
        let workspace = match target {
            DraftTarget::NewChat { workspace } => {
                scope.right(ProcessRight::Create)?;
                ensure!(workspace.is_absolute(), "draft workspace must be absolute");
                workspace.clone()
            }
            DraftTarget::Message { session_id } | DraftTarget::Steer { session_id, .. } => {
                scope.right(if matches!(target, DraftTarget::Steer { .. }) {
                    ProcessRight::Steer
                } else {
                    ProcessRight::Execute
                })?;
                if let Scope::Session(g) = scope {
                    ensure!(g.session_id == *session_id, "draft session denied");
                }
                self.registration(*session_id).await?.workspace
            }
        };
        match scope {
            Scope::Owner => (),
            Scope::Connection(g) => ensure!(
                g.full_access || g.workspaces.iter().any(|w| w.path == workspace),
                "draft workspace denied"
            ),
            Scope::Session(g) => ensure!(
                !matches!(target, DraftTarget::NewChat { .. }) && g.workspace == workspace,
                "draft workspace denied"
            ),
        };
        Ok(())
    }
    pub(super) async fn drafts(&self, operation: DraftOperation, scope: Scope) -> Result<Value> {
        scope.current(&self.directory)?;
        // Read access requires history; mutations additionally require target admission rights.
        scope.right(ProcessRight::History)?;
        let key = scope.key();
        if let DraftOperation::Put { document, .. } = &operation {
            self.draft_target(&scope, &document.target).await?;
        }
        let id = match &operation {
            DraftOperation::List {} => None,
            DraftOperation::Get { draft_id }
            | DraftOperation::Put { draft_id, .. }
            | DraftOperation::Delete { draft_id, .. }
            | DraftOperation::UploadImage { draft_id, .. }
            | DraftOperation::ReadImage { draft_id, .. }
            | DraftOperation::Promote { draft_id, .. } => Some(*draft_id),
        };
        if let Some(id) = id {
            ensure!(!id.is_nil(), "nil draft identity");
            let draft = {
                let db = database::open_drafts(&self.directory)?;
                initialize(&db)?;
                load(&db, &key, id)?
            };
            if let Some(d) = draft {
                self.draft_target(&scope, &d.document.target).await?;
            }
        }
        if let DraftOperation::Promote { session_id, .. } = &operation {
            self.draft_target(
                &scope,
                &DraftTarget::Message {
                    session_id: *session_id,
                },
            )
            .await?;
            let draft = {
                let db = database::open_drafts(&self.directory)?;
                load(&db, &key, id.unwrap())?
            };
            if let Some(Draft {
                document:
                    DraftDocument {
                        target: DraftTarget::NewChat { workspace },
                        ..
                    },
                ..
            }) = draft
            {
                ensure!(
                    self.registration(*session_id).await?.workspace == workspace,
                    "promotion workspace mismatch"
                );
            }
        }
        // No await while a SQLite writer is held. The transaction is the cross-process CAS.
        let (result, plan) = self.draft_transaction(&operation, &scope)?;
        let Some(plan) = plan else { return Ok(result) };
        let DraftOperation::Promote {
            command_id,
            session_id,
            ..
        } = operation
        else {
            unreachable!()
        };
        let registration = self.registration(session_id).await?;
        let binding = match &scope {
            Scope::Owner => None,
            Scope::Connection(g) => {
                Some(self.connection_session(g, session_id, &registration.workspace)?)
            }
            Scope::Session(g) => Some(GrantBinding {
                grant_id: g.grant_id,
                principal_id: g.principal_id,
                revision: g.revision,
            }),
        };
        let mut parts: Vec<ContentPart> = serde_json::from_value(plan["parts"].clone())?;
        for (index, part) in parts.iter_mut().enumerate() {
            if let ContentPart::Image { attachment } = part {
                scope.current(&self.directory)?;
                let digest = Sha256::digest(format!("draft-promotion:{key}:{command_id}:{index}"));
                let upload_id = Uuid::from_bytes(digest[..16].try_into()?);
                let reply = self
                    .voyage(
                        VoyageRequest {
                            session_id,
                            incarnation: None,
                            command: VoyageCommand::UploadImage {
                                upload_id,
                                name: attachment.name.clone(),
                                data_base64: plan["images"][index.to_string()]
                                    .as_str()
                                    .ok_or_else(|| anyhow::anyhow!("missing promotion image"))?
                                    .to_owned(),
                            },
                        },
                        binding.clone(),
                    )
                    .await?;
                let reply: VoyageReply = serde_json::from_value(reply)?;
                *attachment = serde_json::from_value(reply.result)?;
            }
        }
        scope.current(&self.directory)?;
        let result = json!({"parts":parts});
        let db = database::open_drafts(&self.directory)?;
        db.execute(
            "UPDATE composer_receipts SET result=?3,plan=NULL WHERE scope=?1 AND id=?2",
            params![key, command_id.to_string(), serde_json::to_string(&result)?],
        )?;
        Ok(result)
    }
    fn draft_transaction(
        &self,
        op: &DraftOperation,
        scope: &Scope,
    ) -> Result<(Value, Option<Value>)> {
        scope.current(&self.directory)?;
        let key = scope.key();
        let now = store::now()?;
        let mut db = database::open_drafts(&self.directory)?;
        initialize(&db)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Retain fingerprints permanently: expiry never makes an old identity executable again.
        tx.execute("UPDATE composer_receipts SET result=NULL WHERE created<?1 AND plan IS NULL AND result IS NOT NULL",[now.saturating_sub(7*86_400_000)])?;
        let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(op)?));
        if let Some(id) = op.mutation_id() {
            ensure!(!id.is_nil(), "nil draft mutation identity");
            let prior:Option<(String,Option<String>,Option<String>)>=tx.query_row("SELECT fingerprint,result,plan FROM composer_receipts WHERE scope=?1 AND id=?2",params![key,id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            if let Some((hash, result, plan)) = prior {
                ensure!(
                    result.is_some() || plan.is_some(),
                    "draft mutation receipt expired; reconcile current draft, never replay this identity"
                );
                ensure!(
                    hash == fingerprint,
                    "draft mutation identity reused with different payload"
                );
                return Ok((
                    result
                        .map(|s| serde_json::from_str(&s))
                        .transpose()?
                        .unwrap_or(Value::Null),
                    plan.map(|s| serde_json::from_str(&s)).transpose()?,
                ));
            }
            let count: u64 =
                tx.query_row("SELECT count(*) FROM composer_receipts", [], |r| r.get(0))?;
            ensure!(
                count < 1_000_000,
                "draft receipt quota reached; receipts are retained to prevent replay"
            );
        }
        // Unreferenced staging has a 24-hour retry window. Referenced images never expire.
        tx.execute("DELETE FROM composer_images WHERE created < ?1 AND NOT EXISTS (SELECT 1 FROM composer_drafts d WHERE d.scope=composer_images.scope AND d.id=composer_images.draft AND d.document IS NOT NULL AND EXISTS (SELECT 1 FROM json_each(json_extract(d.document,'$.parts')) p WHERE json_extract(p.value,'$.attachment.id')=composer_images.id))",[now.saturating_sub(86_400_000)])?;
        let mut plan = None;
        let result = match op {
            DraftOperation::List {} => {
                let mut q=tx.prepare("SELECT id FROM composer_drafts WHERE scope=?1 AND document IS NOT NULL ORDER BY updated DESC,id")?;
                let ids = q
                    .query_map([&key], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let drafts = ids
                    .iter()
                    .map(|id| load(&tx, &key, Uuid::parse_str(id)?))
                    .collect::<Result<Vec<_>>>()?;
                json!({"drafts":drafts})
            }
            DraftOperation::Get { draft_id } => serde_json::to_value(load(&tx, &key, *draft_id)?)?,
            DraftOperation::Put {
                draft_id,
                expected_revision,
                document,
                ..
            } => {
                let revision: Option<u64> = tx
                    .query_row(
                        "SELECT revision FROM composer_drafts WHERE scope=?1 AND id=?2",
                        params![key, draft_id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                ensure!(
                    revision.unwrap_or(0) == *expected_revision,
                    "draft revision conflict; fetch current draft and reconcile"
                );
                ensure!(
                    revision.is_none() || load(&tx, &key, *draft_id)?.is_some(),
                    "deleted draft identity cannot be reused"
                );
                let count: u64 = tx.query_row(
                    "SELECT count(*) FROM composer_drafts WHERE scope=?1 AND document IS NOT NULL",
                    [&key],
                    |r| r.get(0),
                )?;
                ensure!(
                    revision.is_some() || count < 64,
                    "draft count quota reached"
                );
                let total: u64 =
                    tx.query_row("SELECT count(*) FROM composer_drafts", [], |r| r.get(0))?;
                ensure!(
                    revision.is_some() || total < 100_000,
                    "Vessel draft identity quota reached"
                );
                validate(&tx, &key, *draft_id, document)?;
                let document_bytes = serde_json::to_string(document)?;
                let aggregate:u64=tx.query_row("SELECT coalesce(sum(length(document)),0) FROM composer_drafts WHERE scope=?1 AND id!=?2",params![key,draft_id.to_string()],|r|r.get(0))?;
                ensure!(
                    aggregate + document_bytes.len() as u64 <= 1024 * 1024,
                    "draft text quota reached (1MiB per namespace)"
                );
                let next = expected_revision
                    .checked_add(1)
                    .filter(|r| *r <= i64::MAX as u64)
                    .ok_or_else(|| anyhow::anyhow!("draft revision exhausted"))?;
                let draft = Draft {
                    draft_id: *draft_id,
                    revision: next,
                    document: document.clone(),
                    updated_at_ms: now,
                };
                tx.execute("INSERT INTO composer_drafts VALUES(?1,?2,?3,?4,?5) ON CONFLICT(scope,id) DO UPDATE SET revision=excluded.revision,document=excluded.document,updated=excluded.updated",params![key,draft_id.to_string(),next,serde_json::to_string(document)?,now])?;
                serde_json::to_value(draft)?
            }
            DraftOperation::Delete {
                draft_id,
                expected_revision,
                ..
            } => {
                let d = load(&tx, &key, *draft_id)?
                    .ok_or_else(|| anyhow::anyhow!("draft unavailable"))?;
                ensure!(
                    d.revision == *expected_revision,
                    "draft revision conflict; sent draft has newer edits"
                );
                tx.execute(
                    "UPDATE composer_drafts SET document=NULL,updated=?3 WHERE scope=?1 AND id=?2",
                    params![key, draft_id.to_string(), now],
                )?;
                tx.execute(
                    "DELETE FROM composer_images WHERE scope=?1 AND draft=?2",
                    params![key, draft_id.to_string()],
                )?;
                json!({"deleted":true,"draft_id":draft_id,"revision":expected_revision})
            }
            DraftOperation::UploadImage {
                command_id,
                draft_id,
                name,
                data_base64,
            } => {
                ensure!(load(&tx, &key, *draft_id)?.is_some(), "draft unavailable");
                ensure!(
                    !name.trim().is_empty() && name.len() <= 255 && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{206f}')),
                    "invalid image label"
                );
                ensure!(
                    data_base64.len() <= voyage_runtime::images::MAX_BYTES.div_ceil(3) * 4,
                    "image exceeds 2MiB limit"
                );
                let bytes = STANDARD
                    .decode(data_base64)
                    .map_err(|_| anyhow::anyhow!("invalid image base64"))?;
                ensure!(
                    STANDARD.encode(&bytes) == *data_base64,
                    "image requires canonical base64"
                );
                let (media_type, width, height) = voyage_runtime::images::validate(&bytes)?;
                let (count,size):(u64,u64)=tx.query_row("SELECT count(*),coalesce(sum(length(bytes)),0) FROM composer_images WHERE scope=?1 AND draft=?2",params![key,draft_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?)))?;
                ensure!(
                    count < 16 && size + bytes.len() as u64 <= 8 * 1024 * 1024,
                    "draft staging quota reached; remove references and wait for orphan expiry or delete draft"
                );
                let total: u64 = tx.query_row(
                    "SELECT coalesce(sum(length(bytes)),0) FROM composer_images",
                    [],
                    |r| r.get(0),
                )?;
                ensure!(
                    total + bytes.len() as u64 <= 64 * 1024 * 1024,
                    "Vessel staging quota reached"
                );
                let attachment = ImageAttachment {
                    id: *command_id,
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                    name: name.clone(),
                    media_type,
                    byte_size: bytes.len() as u64,
                    width,
                    height,
                };
                tx.execute(
                    "INSERT INTO composer_images VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        key,
                        draft_id.to_string(),
                        command_id.to_string(),
                        serde_json::to_string(&attachment)?,
                        bytes,
                        now
                    ],
                )?;
                serde_json::to_value(attachment)?
            }
            DraftOperation::ReadImage {
                draft_id,
                attachment_id,
                offset,
                limit,
            } => {
                ensure!(load(&tx, &key, *draft_id)?.is_some(), "draft unavailable");
                let (attachment, bytes) = image(&tx, &key, *draft_id, *attachment_id)?;
                ensure!(
                    *offset <= bytes.len() as u64 && *limit > 0 && *limit <= 256 * 1024,
                    "invalid staged image read range (maximum 256KiB)"
                );
                let end = (*offset as usize + *limit as usize).min(bytes.len());
                json!({"attachment":attachment,"offset":offset,"data_base64":STANDARD.encode(&bytes[*offset as usize..end]),"next_offset":if end<bytes.len(){Some(end)}else{None},"total_bytes":bytes.len()})
            }
            DraftOperation::Promote {
                draft_id,
                expected_revision,
                session_id,
                ..
            } => {
                let draft = load(&tx, &key, *draft_id)?
                    .ok_or_else(|| anyhow::anyhow!("draft unavailable"))?;
                ensure!(
                    draft.revision == *expected_revision,
                    "draft revision conflict"
                );
                match draft.document.target {
                    DraftTarget::Message { session_id: id }
                    | DraftTarget::Steer { session_id: id, .. } => {
                        ensure!(id == *session_id, "draft target mismatch")
                    }
                    _ => (),
                }
                let mut images = serde_json::Map::new();
                for (i, p) in draft.document.parts.iter().enumerate() {
                    if let ContentPart::Image { attachment } = p {
                        images.insert(
                            i.to_string(),
                            json!(STANDARD.encode(image(&tx, &key, *draft_id, attachment.id)?.1)),
                        );
                    }
                }
                let pending: u64 = tx.query_row(
                    "SELECT coalesce(sum(length(plan)),0) FROM composer_receipts",
                    [],
                    |r| r.get(0),
                )?;
                ensure!(
                    pending < 64 * 1024 * 1024,
                    "pending promotion quota reached"
                );
                plan = Some(json!({"parts":draft.document.parts,"images":images}));
                Value::Null
            }
        };
        if let Some(id) = op.mutation_id() {
            tx.execute(
                "INSERT INTO composer_receipts VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    key,
                    id.to_string(),
                    fingerprint,
                    if plan.is_none() {
                        Some(serde_json::to_string(&result)?)
                    } else {
                        None
                    },
                    plan.as_ref().map(serde_json::to_string).transpose()?,
                    now
                ],
            )?;
        }
        // Result bodies are a bounded convenience cache; immutable fingerprints survive
        // compaction. An evicted retry refuses explicitly instead of replaying effects.
        let result_bytes: u64 = tx.query_row(
            "SELECT coalesce(sum(length(result)),0) FROM composer_receipts",
            [],
            |r| r.get(0),
        )?;
        if result_bytes > 16 * 1024 * 1024 {
            tx.execute("UPDATE composer_receipts SET result=NULL WHERE rowid IN (SELECT rowid FROM (SELECT rowid,sum(length(result)) OVER (ORDER BY created DESC,rowid DESC) AS retained FROM composer_receipts WHERE result IS NOT NULL AND plan IS NULL) WHERE retained>8*1024*1024)",[])?;
        }
        scope.current(&self.directory)?;
        tx.commit()?;
        Ok((result, plan))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::test_support::Fixture;
    fn put(id: Uuid, revision: u64, workspace: &std::path::Path, text: &str) -> DraftOperation {
        DraftOperation::Put {
            command_id: Uuid::new_v4(),
            draft_id: id,
            expected_revision: revision,
            document: DraftDocument {
                target: DraftTarget::NewChat {
                    workspace: workspace.to_owned(),
                },
                parts: vec![ContentPart::Text { text: text.into() }],
            },
        }
    }
    fn owner(f: &Fixture) -> ConnectionGrant {
        let mut g = f.connection();
        g.full_access = true;
        g.workspaces.clear();
        g.accounts.clear();
        g.enrollment_connections.clear();
        g.rights = ProcessRight::all();
        store::save(&store::connection_path(&f.0, g.grant_id), &g).unwrap();
        g
    }
    #[tokio::test]
    async fn separate_owner_pairings_share_cas_restart_and_revoke() {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let a = owner(&f);
        let b = owner(&f);
        let id = Uuid::new_v4();
        let op = put(id, 0, &f.0, "phone");
        let saved = s
            .drafts(op.clone(), Scope::Connection(a.clone()))
            .await
            .unwrap();
        assert_eq!(saved["revision"], 1);
        assert_eq!(
            s.drafts(
                DraftOperation::Get { draft_id: id },
                Scope::Connection(b.clone())
            )
            .await
            .unwrap(),
            saved
        );
        let next = s
            .drafts(put(id, 1, &f.0, "desktop"), Scope::Connection(b.clone()))
            .await
            .unwrap();
        assert_eq!(next["revision"], 2);
        assert!(
            s.drafts(put(id, 1, &f.0, "stale"), Scope::Connection(a.clone()))
                .await
                .is_err()
        );
        assert_eq!(
            s.drafts(op.clone(), Scope::Connection(b.clone()))
                .await
                .unwrap(),
            saved
        );
        let mut changed = op.clone();
        if let DraftOperation::Put { document, .. } = &mut changed {
            document.parts.clear();
        }
        assert!(
            s.drafts(changed, Scope::Connection(a.clone()))
                .await
                .is_err()
        );
        drop(s);
        let s = f.supervisor().await;
        assert_eq!(
            s.drafts(DraftOperation::Get { draft_id: id }, Scope::Owner)
                .await
                .unwrap(),
            next
        );
        assert!(database::catalogue(&f.0).await.unwrap().is_empty());
        let mut revoked = a.clone();
        revoked.revoked = true;
        store::save(&store::connection_path(&f.0, a.grant_id), &revoked).unwrap();
        assert!(
            s.drafts(DraftOperation::Get { draft_id: id }, Scope::Connection(a))
                .await
                .is_err()
        );
        assert!(
            s.drafts(
                DraftOperation::Delete {
                    command_id: Uuid::new_v4(),
                    draft_id: id,
                    expected_revision: 1
                },
                Scope::Connection(b.clone())
            )
            .await
            .is_err()
        );
        let delete = DraftOperation::Delete {
            command_id: Uuid::new_v4(),
            draft_id: id,
            expected_revision: 2,
        };
        let deleted = s
            .drafts(delete.clone(), Scope::Connection(b.clone()))
            .await
            .unwrap();
        assert_eq!(s.drafts(delete, Scope::Owner).await.unwrap(), deleted);
        assert!(
            s.drafts(put(id, 0, &f.0, "resurrect"), Scope::Owner)
                .await
                .is_err()
        );
        assert!(
            s.drafts(put(id, 2, &f.0, "resurrect"), Scope::Owner)
                .await
                .is_err()
        );
        assert!(
            s.drafts(DraftOperation::Get { draft_id: id }, Scope::Owner)
                .await
                .unwrap()
                .is_null()
        );
    }
    #[tokio::test]
    async fn scoped_connections_cannot_discover_owner_or_each_other() {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let id = Uuid::new_v4();
        s.drafts(put(id, 0, &f.0, "private owner"), Scope::Owner)
            .await
            .unwrap();
        let mut a = f.connection();
        a.rights = vec![ProcessRight::History, ProcessRight::Create];
        store::save(&store::connection_path(&f.0, a.grant_id), &a).unwrap();
        assert!(
            s.drafts(
                DraftOperation::Get { draft_id: id },
                Scope::Connection(a.clone())
            )
            .await
            .unwrap()
            .is_null()
        );
        assert_eq!(
            s.drafts(DraftOperation::List {}, Scope::Connection(a.clone()))
                .await
                .unwrap()["drafts"],
            json!([])
        );
        s.drafts(put(id, 0, &f.0, "scoped"), Scope::Connection(a.clone()))
            .await
            .unwrap();
        assert_eq!(
            s.drafts(DraftOperation::Get { draft_id: id }, Scope::Owner)
                .await
                .unwrap()["document"]["parts"][0]["text"],
            "private owner"
        );
        assert!(
            s.drafts(
                put(Uuid::new_v4(), 0, std::path::Path::new("/outside"), "x"),
                Scope::Connection(a.clone())
            )
            .await
            .is_err()
        );
        a.rights.retain(|r| *r != ProcessRight::Create);
        a.revision += 1;
        store::save(&store::connection_path(&f.0, a.grant_id), &a).unwrap();
        assert!(
            s.drafts(put(Uuid::new_v4(), 0, &f.0, "x"), Scope::Connection(a))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn raster_staging_reference_validation_reads_and_delete() {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let id = Uuid::new_v4();
        s.drafts(put(id, 0, &f.0, ""), Scope::Owner).await.unwrap();
        // Verified one-pixel PNG; decoder checks CRC and raster, not just MIME/header.
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        for name in ["../pixel.png", "a\\b.png", "  ", "bad\u{202e}.png"] {
            assert!(
                s.drafts(
                    DraftOperation::UploadImage {
                        command_id: Uuid::new_v4(),
                        draft_id: id,
                        name: name.into(),
                        data_base64: png.into()
                    },
                    Scope::Owner
                )
                .await
                .is_err()
            );
        }
        let upload = DraftOperation::UploadImage {
            command_id: Uuid::new_v4(),
            draft_id: id,
            name: "pixel.png".into(),
            data_base64: png.into(),
        };
        let meta = s.drafts(upload.clone(), Scope::Owner).await.unwrap();
        assert_eq!(s.drafts(upload, Scope::Owner).await.unwrap(), meta);
        let attachment: ImageAttachment = serde_json::from_value(meta).unwrap();
        assert_eq!(attachment.width, 1);
        let mut doc = DraftDocument {
            target: DraftTarget::NewChat {
                workspace: f.0.clone(),
            },
            parts: vec![ContentPart::Image {
                attachment: attachment.clone(),
            }],
        };
        s.drafts(
            DraftOperation::Put {
                command_id: Uuid::new_v4(),
                draft_id: id,
                expected_revision: 1,
                document: doc.clone(),
            },
            Scope::Owner,
        )
        .await
        .unwrap();
        let read = s
            .drafts(
                DraftOperation::ReadImage {
                    draft_id: id,
                    attachment_id: attachment.id,
                    offset: 0,
                    limit: 262144,
                },
                Scope::Owner,
            )
            .await
            .unwrap();
        assert_eq!(read["data_base64"], png);
        assert!(read["next_offset"].is_null());
        if let ContentPart::Image { attachment } = &mut doc.parts[0] {
            attachment.width = 2;
        }
        assert!(
            s.drafts(
                DraftOperation::Put {
                    command_id: Uuid::new_v4(),
                    draft_id: id,
                    expected_revision: 2,
                    document: doc
                },
                Scope::Owner
            )
            .await
            .is_err()
        );
        for bytes in ["PHN2Zz48L3N2Zz4=", "AAAA"] {
            assert!(
                s.drafts(
                    DraftOperation::UploadImage {
                        command_id: Uuid::new_v4(),
                        draft_id: id,
                        name: "bad.png".into(),
                        data_base64: bytes.into()
                    },
                    Scope::Owner
                )
                .await
                .is_err()
            );
        }
        assert!(
            s.drafts(
                DraftOperation::UploadImage {
                    command_id: Uuid::new_v4(),
                    draft_id: id,
                    name: "huge".into(),
                    data_base64: "A".repeat(3 * 1024 * 1024)
                },
                Scope::Owner
            )
            .await
            .is_err()
        );
        s.drafts(
            DraftOperation::Delete {
                command_id: Uuid::new_v4(),
                draft_id: id,
                expected_revision: 2,
            },
            Scope::Owner,
        )
        .await
        .unwrap();
        assert!(
            s.drafts(
                DraftOperation::ReadImage {
                    draft_id: id,
                    attachment_id: attachment.id,
                    offset: 0,
                    limit: 100
                },
                Scope::Owner
            )
            .await
            .is_err()
        );
        let db = database::open_drafts(&f.0).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM composer_images", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn quota_and_simultaneous_edits_are_bounded() {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let id = Uuid::new_v4();
        s.drafts(put(id, 0, &f.0, ""), Scope::Owner).await.unwrap();
        let (a, b) = tokio::join!(
            s.drafts(put(id, 1, &f.0, "a"), Scope::Owner),
            s.drafts(put(id, 1, &f.0, "b"), Scope::Owner)
        );
        assert_ne!(a.is_ok(), b.is_ok());
        for _ in 1..64 {
            s.drafts(put(Uuid::new_v4(), 0, &f.0, ""), Scope::Owner)
                .await
                .unwrap();
        }
        assert!(
            s.drafts(put(Uuid::new_v4(), 0, &f.0, "overflow"), Scope::Owner)
                .await
                .is_err()
        );
        assert_eq!(
            s.drafts(DraftOperation::List {}, Scope::Owner)
                .await
                .unwrap()["drafts"]
                .as_array()
                .unwrap()
                .len(),
            64
        );
    }
    #[tokio::test]
    async fn session_grants_are_isolated_and_delegation_refused() {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let registration = f.registration();
        database::save(&f.0, &registration).await.unwrap();
        let mut grant = f.session();
        grant.session_id = registration.session_id;
        grant.rights = vec![
            ProcessRight::History,
            ProcessRight::Execute,
            ProcessRight::Steer,
        ];
        f.save_session(&grant);
        let id = Uuid::new_v4();
        let operation = DraftOperation::Put {
            command_id: Uuid::new_v4(),
            draft_id: id,
            expected_revision: 0,
            document: DraftDocument {
                target: DraftTarget::Message {
                    session_id: registration.session_id,
                },
                parts: vec![ContentPart::Text {
                    text: " scoped ".into(),
                }],
            },
        };
        s.drafts(operation, Scope::Session(grant.clone()))
            .await
            .unwrap();
        assert!(
            s.drafts(DraftOperation::Get { draft_id: id }, Scope::Owner)
                .await
                .unwrap()
                .is_null()
        );
        let mut other = grant.clone();
        other.grant_id = Uuid::new_v4();
        f.save_session(&other);
        assert!(
            s.drafts(DraftOperation::Get { draft_id: id }, Scope::Session(other))
                .await
                .unwrap()
                .is_null()
        );
        assert!(
            s.drafts(
                put(Uuid::new_v4(), 0, &f.0, "new chat"),
                Scope::Session(grant.clone())
            )
            .await
            .is_err()
        );
        let mut steer = DraftOperation::Put {
            command_id: Uuid::new_v4(),
            draft_id: id,
            expected_revision: 1,
            document: DraftDocument {
                target: DraftTarget::Steer {
                    session_id: registration.session_id,
                    run_id: Uuid::new_v4(),
                    incarnation: registration.incarnation,
                },
                parts: vec![ContentPart::Text {
                    text: "steer".into(),
                }],
            },
        };
        s.drafts(steer.clone(), Scope::Session(grant.clone()))
            .await
            .unwrap();
        if let DraftOperation::Put { document, .. } = &mut steer {
            document.target = DraftTarget::Message {
                session_id: Uuid::new_v4(),
            };
        }
        assert!(
            s.drafts(steer, Scope::Session(grant.clone()))
                .await
                .is_err()
        );
        grant.revoked = true;
        f.save_session(&grant);
        assert!(
            s.drafts(DraftOperation::List {}, Scope::Session(grant))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn expired_receipts_refuse_replay_and_whitespace_survives() {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let id = Uuid::new_v4();
        let op = put(id, 0, &f.0, "  \n");
        s.drafts(op.clone(), Scope::Owner).await.unwrap();
        let db = database::open_drafts(&f.0).unwrap();
        db.execute("UPDATE composer_receipts SET created=0", [])
            .unwrap();
        assert!(
            s.drafts(op, Scope::Owner)
                .await
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
        assert_eq!(
            s.drafts(DraftOperation::Get { draft_id: id }, Scope::Owner)
                .await
                .unwrap()["revision"],
            1
        );
    }
}
