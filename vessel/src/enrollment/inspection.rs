//! Read-only bounded operator views over the existing authority and audit history.
use super::*;
use serde::{Deserialize, Serialize};
use voyage_protocol::enrollment::inspection::{self as wire, AuditKind, Request};
const CURSOR_MS: i64 = 300_000;
const DOMAIN: &[u8] = b"voyage.enrollment.inspection.v1\0";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Machines,
    Audit,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u16,
    owner: Uuid,
    kind: Kind,
    limit: u16,
    expires: i64,
    through: i64,
    after: i64,
    machine: Option<Uuid>,
}
fn sign(key: &ring::hmac::Key, cursor: &Cursor) -> Result<String> {
    let bytes = serde_json::to_vec(cursor).map_err(|_| EnrollmentError::Invalid)?;
    let mut bound = DOMAIN.to_vec();
    bound.extend_from_slice(&bytes);
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&bytes),
        URL_SAFE_NO_PAD.encode(ring::hmac::sign(key, &bound).as_ref())
    ))
}
fn decode(
    key: &ring::hmac::Key,
    owner: Uuid,
    kind: Kind,
    request: &Request,
    now: i64,
) -> Result<Option<Cursor>> {
    request.validate().map_err(|_| EnrollmentError::Invalid)?;
    let Some(token) = &request.cursor else {
        return Ok(None);
    };
    let (body, tag) = token.split_once('.').ok_or(EnrollmentError::Invalid)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(body)
        .map_err(|_| EnrollmentError::Invalid)?;
    let tag = URL_SAFE_NO_PAD
        .decode(tag)
        .map_err(|_| EnrollmentError::Invalid)?;
    if tag.len() != 32 {
        return Err(EnrollmentError::Invalid);
    }
    let mut bound = DOMAIN.to_vec();
    bound.extend_from_slice(&bytes);
    ring::hmac::verify(key, &bound, &tag).map_err(|_| EnrollmentError::Denied)?;
    let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| EnrollmentError::Invalid)?;
    if cursor.version != 1
        || cursor.owner != owner
        || cursor.kind != kind
        || cursor.limit != request.limit
        || cursor.through < 0
        || cursor.after < 0
        || cursor.machine.is_some_and(|id| id.is_nil())
    {
        return Err(EnrollmentError::Denied);
    }
    if cursor.expires <= now || cursor.expires > now.saturating_add(CURSOR_MS) {
        return Err(EnrollmentError::Conflict);
    }
    Ok(Some(cursor))
}
fn clock(db: &Connection, now: i64) -> Result<()> {
    let last: i64 = db.query_row(
        "SELECT last_time FROM enrollment_schema WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    if now < last || !(0..=i64::MAX - CURSOR_MS).contains(&now) {
        return Err(EnrollmentError::Invalid);
    }
    Ok(())
}
fn identifier(value: String) -> Result<Uuid> {
    let id = Uuid::parse_str(&value).map_err(|_| EnrollmentError::Storage)?;
    if id.is_nil() || id.to_string() != value {
        return Err(EnrollmentError::Storage);
    }
    Ok(id)
}
impl EnrollmentStore {
    /// Authenticate the operator before calling. A page is observation, not authority.
    pub(crate) fn inspect_machines(
        &mut self,
        request: &Request,
        now: i64,
    ) -> Result<wire::Machines> {
        if request.after != 0 {
            return Err(EnrollmentError::Invalid);
        }
        let prior = decode(
            &self.challenge_key,
            self.owner,
            Kind::Machines,
            request,
            now,
        )?;
        let tx = self.db.transaction()?;
        clock(&tx, now)?;
        let revision:i64=tx.query_row("SELECT coalesce(max(sequence),0) FROM audit WHERE kind IN ('enrolled','rotated','revoked')",[],|r|r.get(0))?;
        if revision < 0 {
            return Err(EnrollmentError::Storage);
        }
        if prior
            .as_ref()
            .is_some_and(|c| c.through != revision || c.after != 0)
        {
            return Err(EnrollmentError::Conflict);
        }
        let after = prior
            .as_ref()
            .and_then(|c| c.machine)
            .map(|id| id.to_string());
        let mut statement=tx.prepare("SELECT CASE WHEN length(CAST(id AS BLOB))=36 THEN id END,epoch,revoked FROM machines WHERE (?1 IS NULL OR id>?1) ORDER BY id LIMIT ?2")?;
        let rows = statement
            .query_map(params![after, request.limit as i64 + 1], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut machines = Vec::new();
        for (id, epoch, revoked) in rows {
            if epoch <= 0 || revision == 0 || !matches!(revoked, 0 | 1) {
                return Err(EnrollmentError::Storage);
            }
            machines.push(wire::Machine {
                machine_id: identifier(id.ok_or(EnrollmentError::Storage)?)?,
                epoch: epoch as u64,
                revoked: revoked == 1,
            });
        }
        let more = machines.len() > request.limit as usize;
        machines.truncate(request.limit as usize);
        let next = if more {
            Some(sign(
                &self.challenge_key,
                &Cursor {
                    version: 1,
                    owner: self.owner,
                    kind: Kind::Machines,
                    limit: request.limit,
                    expires: prior.as_ref().map_or(now + CURSOR_MS, |c| c.expires),
                    through: revision,
                    after: 0,
                    machine: machines.last().map(|m| m.machine_id),
                },
            )?)
        } else {
            None
        };
        drop(statement);
        tx.commit()?;
        Ok(wire::Machines {
            version: 1,
            owner_id: self.owner,
            revision: revision as u64,
            machines,
            next,
        })
    }
    /// Existing audit rows are immutable and retained; capacity fails closed instead of pruning.
    pub(crate) fn inspect_audit(&mut self, request: &Request, now: i64) -> Result<wire::Audit> {
        let prior = decode(&self.challenge_key, self.owner, Kind::Audit, request, now)?;
        let tx = self.db.transaction()?;
        clock(&tx, now)?;
        let (first, last): (i64, i64) = tx.query_row(
            "SELECT coalesce(min(sequence),0),coalesce(max(sequence),0) FROM audit",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if first < 0 || last < 0 || (last != 0 && first != 1) {
            return Err(EnrollmentError::Storage);
        }
        let through = prior.as_ref().map_or(last, |c| c.through);
        let after = prior.as_ref().map_or(request.after as i64, |c| c.after);
        if through > last || after > through || prior.as_ref().is_some_and(|c| c.machine.is_some())
        {
            return Err(EnrollmentError::Conflict);
        }
        let mut statement=tx.prepare("SELECT sequence,machine_id IS NULL,CASE WHEN length(CAST(machine_id AS BLOB))=36 THEN machine_id END,CASE WHEN length(CAST(kind AS BLOB))<=32 THEN kind END,time FROM audit WHERE sequence>?1 AND sequence<=?2 ORDER BY sequence LIMIT ?3")?;
        let rows = statement
            .query_map(params![after, through, request.limit as i64 + 1], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut events = Vec::new();
        let mut expected = after;
        for (sequence, no_machine, machine, kind, time) in rows {
            expected = expected.checked_add(1).ok_or(EnrollmentError::Storage)?;
            if sequence != expected || time < 0 {
                return Err(EnrollmentError::Storage);
            }
            let kind = match kind.as_deref() {
                Some("invitation_created") => AuditKind::InvitationCreated,
                Some("enrolled") => AuditKind::Enrolled,
                Some("authenticated") => AuditKind::Authenticated,
                Some("rotated") => AuditKind::Rotated,
                Some("revoked") => AuditKind::Revoked,
                _ => return Err(EnrollmentError::Storage),
            };
            let machine_id = if no_machine {
                None
            } else {
                Some(identifier(machine.ok_or(EnrollmentError::Storage)?)?)
            };
            if no_machine != (kind == AuditKind::InvitationCreated) {
                return Err(EnrollmentError::Storage);
            }
            events.push(wire::AuditEvent {
                sequence: sequence as u64,
                machine_id,
                kind,
                time_ms: time,
            });
        }
        let more = events.len() > request.limit as usize;
        events.truncate(request.limit as usize);
        let end = events.last().map_or(after, |event| event.sequence as i64);
        if !more && end != through {
            return Err(EnrollmentError::Storage);
        }
        let next = if more {
            Some(sign(
                &self.challenge_key,
                &Cursor {
                    version: 1,
                    owner: self.owner,
                    kind: Kind::Audit,
                    limit: request.limit,
                    expires: prior.as_ref().map_or(now + CURSOR_MS, |c| c.expires),
                    through,
                    after: end,
                    machine: None,
                },
            )?)
        } else {
            None
        };
        drop(statement);
        tx.commit()?;
        Ok(wire::Audit {
            version: 1,
            owner_id: self.owner,
            through: through as u64,
            events,
            next,
            retention: wire::Retention::AllRetained,
        })
    }
}
#[cfg(test)]
mod tests;
