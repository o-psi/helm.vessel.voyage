//! Local-owner lifecycle projection. No credentials, paths, free text or cleanup claims.
use super::*;
use sha2::{Digest, Sha256};

const RETAIN: usize = 4096;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Event {
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub kind: Kind,
    pub command_id: Option<Uuid>,
    pub invitation_id: Option<Uuid>,
    pub grant_id: Option<Uuid>,
    pub principal_id: Option<Uuid>,
    pub vessel_id: Uuid,
    pub revision: Option<u64>,
    pub expires_at_ms: Option<u64>,
    pub workspace_ids: Vec<Uuid>,
    pub rights: Vec<ProcessRight>,
    pub account_ids: Vec<Uuid>,
    pub enrollment_connection_ids: Vec<Uuid>,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    AuditStarted,
    InvitationRecorded,
    GrantPublicationIntended,
    GrantPublicationObserved,
    RevocationIntended,
    RevocationObserved,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    version: u32,
    generation: Uuid,
    vessel_id: Uuid,
    legacy_history_unavailable: bool,
    first_sequence: u64,
    next_sequence: u64,
    events: Vec<Event>,
}
impl Journal {
    pub fn new(vessel_id: Uuid, legacy: bool) -> Self {
        Self {
            version: 1,
            generation: Uuid::new_v4(),
            vessel_id,
            legacy_history_unavailable: legacy,
            first_sequence: 1,
            next_sequence: 1,
            events: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1
                && !self.generation.is_nil()
                && !self.vessel_id.is_nil()
                && self.first_sequence > 0
                && self.next_sequence >= self.first_sequence
                && self.events.len() <= RETAIN
                && self.next_sequence - self.first_sequence == self.events.len() as u64,
            "invalid connection audit journal"
        );
        for (i, event) in self.events.iter().enumerate() {
            ensure!(
                event.sequence == self.first_sequence + i as u64
                    && event.vessel_id == self.vessel_id
                    && event.workspace_ids.len() <= 32
                    && event.rights.len() <= 16
                    && event.account_ids.len() <= 64
                    && event.enrollment_connection_ids.len() <= 32,
                "connection audit sequence or metadata gap"
            );
        }
        Ok(())
    }
    pub fn append(&mut self, mut event: Event) -> Result<()> {
        self.validate()?;
        event.sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("connection audit sequence exhausted"))?;
        self.events.push(event);
        if self.events.len() > RETAIN {
            self.events.remove(0);
            self.first_sequence += 1;
        }
        self.validate()
    }
}

pub(super) fn event(
    kind: Kind,
    now: u64,
    vessel_id: Uuid,
    command_id: Option<Uuid>,
    invitation_id: Option<Uuid>,
    grant: Option<&ConnectionGrant>,
) -> Event {
    Event {
        sequence: 0,
        observed_at_ms: now,
        kind,
        command_id,
        invitation_id,
        vessel_id,
        grant_id: grant.map(|g| g.grant_id),
        principal_id: grant.map(|g| g.principal_id),
        revision: grant.map(|g| g.revision),
        expires_at_ms: grant.map(|g| g.expires_at_ms),
        workspace_ids: grant
            .map(|g| g.workspaces.iter().map(|w| w.id).collect())
            .unwrap_or_default(),
        rights: grant.map(|g| g.rights.clone()).unwrap_or_default(),
        account_ids: grant.map(|g| g.accounts.clone()).unwrap_or_default(),
        enrollment_connection_ids: grant
            .map(|g| g.enrollment_connections.clone())
            .unwrap_or_default(),
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u32,
    generation: Uuid,
    vessel_id: Uuid,
    after: u64,
    horizon: u64,
    horizon_digest: String,
    limit: usize,
}
fn digest(event: &Event) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(event)?)))
}

pub(super) fn page(journal: Option<&Journal>, limit: usize, cursor: Option<&str>) -> Result<Value> {
    ensure!(
        (1..=128).contains(&limit),
        "audit limit must be between 1 and 128"
    );
    let Some(journal) = journal else {
        ensure!(
            cursor.is_none(),
            "audit history unavailable for continuation"
        );
        return Ok(
            json!({"schema_version":1,"recording":"not_started","legacy_history":"unavailable","events":[],"next_cursor":null}),
        );
    };
    journal.validate()?;
    let last = journal
        .events
        .last()
        .ok_or_else(|| anyhow::anyhow!("audit history unavailable"))?;
    let c = if let Some(cursor) = cursor {
        ensure!(cursor.len() <= 1024, "audit cursor exceeds limit");
        let c: Cursor =
            serde_json::from_str(cursor).map_err(|_| anyhow::anyhow!("invalid audit cursor"))?;
        ensure!(
            c.version == 1
                && c.generation == journal.generation
                && c.vessel_id == journal.vessel_id
                && c.limit == limit
                && c.after < c.horizon
                && c.horizon < journal.next_sequence,
            "audit cursor identity or horizon changed"
        );
        ensure!(
            c.after >= journal.first_sequence - 1,
            "audit history pruned; restart explicitly"
        );
        let horizon = journal
            .events
            .iter()
            .find(|e| e.sequence == c.horizon)
            .ok_or_else(|| anyhow::anyhow!("audit horizon pruned; restart explicitly"))?;
        ensure!(
            c.horizon_digest == digest(horizon)?,
            "audit history changed; original continuation unavailable"
        );
        c
    } else {
        Cursor {
            version: 1,
            generation: journal.generation,
            vessel_id: journal.vessel_id,
            after: journal.first_sequence - 1,
            horizon: last.sequence,
            horizon_digest: digest(last)?,
            limit,
        }
    };
    let events: Vec<_> = journal
        .events
        .iter()
        .filter(|e| e.sequence > c.after && e.sequence <= c.horizon)
        .take(limit)
        .collect();
    let after = events.last().map_or(c.after, |e| e.sequence);
    let next = if after < c.horizon {
        Some(serde_json::to_string(&Cursor { after, ..c })?)
    } else {
        None
    };
    Ok(
        json!({"schema_version":1,"generation":journal.generation,"vessel_id":journal.vessel_id,
        "legacy_history_unavailable":journal.legacy_history_unavailable,"first_retained_sequence":journal.first_sequence,
        "history_pruned":journal.first_sequence > 1,"horizon_sequence":c.horizon,
        "retention_events":RETAIN,"events":events,"next_cursor":next}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_account_scope_is_retained_without_secrets() {
        let vessel = Uuid::new_v4();
        let grant = ConnectionGrant {
            schema_version: 1,
            grant_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            vessel_id: vessel,
            revision: 1,
            rights: serde_json::from_str(r#"["catalogue","account_use","account_enroll","create","observe","history","execute","steer","decide","cancel","lifecycle","terminal"]"#).unwrap(),
            accounts: vec![Uuid::new_v4()],
            enrollment_connections: vec![Uuid::new_v4()],
            expires_at_ms: 1,
            revoked: false,
            token_hash: "synthetic-verifier-must-not-appear".into(),
            workspaces: vec![],
        };
        let mut journal = Journal::new(vessel, false);
        journal
            .append(event(
                Kind::GrantPublicationObserved,
                1,
                vessel,
                Some(Uuid::new_v4()),
                None,
                Some(&grant),
            ))
            .unwrap();
        let output = page(Some(&journal), 1, None).unwrap();
        assert_eq!(output["events"][0]["account_ids"], json!(grant.accounts));
        assert_eq!(
            output["events"][0]["enrollment_connection_ids"],
            json!(grant.enrollment_connections)
        );
        assert_eq!(output["events"][0]["rights"].as_array().unwrap().len(), 12);
        assert!(
            !serde_json::to_string(&output)
                .unwrap()
                .contains(&grant.token_hash)
        );
    }
    #[test]
    fn fixed_horizon_retention_and_restart() {
        let vessel = Uuid::new_v4();
        let mut j = Journal::new(vessel, true);
        for now in (0..5).rev() {
            j.append(event(Kind::AuditStarted, now, vessel, None, None, None))
                .unwrap();
        }
        let first = page(Some(&j), 2, None).unwrap();
        let cursor = first["next_cursor"].as_str().unwrap();
        j.append(event(Kind::AuditStarted, 9, vessel, None, None, None))
            .unwrap();
        let j: Journal = serde_json::from_slice(&serde_json::to_vec(&j).unwrap()).unwrap();
        let second = page(Some(&j), 2, Some(cursor)).unwrap();
        assert_eq!(second["events"][0]["sequence"], 3);
        let third = page(Some(&j), 2, second["next_cursor"].as_str()).unwrap();
        assert_eq!(third["events"].as_array().unwrap().len(), 1);
        assert_eq!(third["events"][0]["sequence"], 5);
        assert!(third["next_cursor"].is_null());
        assert!(page(Some(&j), 3, Some(cursor)).is_err());
        let mut j = j;
        for _ in 0..RETAIN {
            j.append(event(Kind::AuditStarted, 1, vessel, None, None, None))
                .unwrap();
        }
        assert!(page(Some(&j), 2, Some(cursor)).is_err());
        assert_eq!(j.events.len(), RETAIN);
        assert_eq!(page(Some(&j), 2, None).unwrap()["history_pruned"], true);
        j.events[1].sequence += 1;
        assert!(page(Some(&j), 2, None).is_err());
    }
    #[test]
    fn legacy_and_cursor_identity_fail_closed() {
        assert_eq!(
            page(None, 1, None).unwrap()["legacy_history"],
            "unavailable"
        );
        assert!(page(None, 1, Some("{}")).is_err());
        let vessel = Uuid::new_v4();
        let mut j = Journal::new(vessel, false);
        for n in 0..3 {
            j.append(event(Kind::AuditStarted, n, vessel, None, None, None))
                .unwrap();
        }
        let first = page(Some(&j), 1, None).unwrap();
        let cursor = first["next_cursor"].as_str().unwrap();
        j.events.last_mut().unwrap().observed_at_ms += 1;
        assert!(page(Some(&j), 1, Some(cursor)).is_err());
        assert!(page(Some(&j), 0, None).is_err());
        assert!(page(Some(&j), 129, None).is_err());
    }
}
