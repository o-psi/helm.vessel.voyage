//! Durable control metadata in the existing private enrollment database.
//! No canonical history, task admission or positive execution grant exists here.
use super::*;
use std::sync::Arc;
use voyage_protocol::control::{
    self, Address, ClientOperation, ExecutionAuthority, Lease, Mutation, MutationKind, Record,
    Status, View,
};

pub(crate) struct Peer {
    pub machine: control::Machine,
    pub owner: Uuid,
    pub connection: Uuid,
    /// Constructed only by the authenticated transport. Checked under transaction.
    pub live: Arc<dyn Fn() -> bool + Send + Sync>,
}
const MAX_ROW: usize = 16 * 1024;
fn key(address: Address) -> String {
    format!("{}:{}", address.installation_id, address.session_id)
}
fn current(db: &Connection, binding: control::Machine) -> Result<()> {
    let m = machine(db, binding.machine_id)?;
    if m.revoked || m.epoch as u64 != binding.epoch {
        Err(EnrollmentError::Denied)
    } else {
        Ok(())
    }
}
fn generation(tx: &Transaction<'_>, expected: u64) -> Result<()> {
    let actual: i64 = tx.query_row(
        "SELECT generation FROM coordination_schema WHERE id=1 AND version=1",
        [],
        |r| r.get(0),
    )?;
    if actual > 0 && actual as u64 == expected {
        Ok(())
    } else {
        Err(EnrollmentError::Conflict)
    }
}
fn checked_clock(tx: &Transaction<'_>, clock: &impl Fn() -> Result<i64>) -> Result<i64> {
    let now = clock()?;
    check_clock(tx, now)?;
    Ok(now)
}
fn deadline(expires: i64, now: i64) -> Result<()> {
    if expires <= now
        || expires > now.saturating_add(voyage_protocol::attachment::MAX_COMMAND_LIFETIME_MS)
    {
        Err(EnrollmentError::Invalid)
    } else {
        Ok(())
    }
}
fn decode_record(value: String) -> Result<Record> {
    let record: Record = serde_json::from_str(&value).map_err(|_| EnrollmentError::Storage)?;
    record.validate().map_err(|_| EnrollmentError::Storage)?;
    Ok(record)
}
fn load(db: &Connection, address: Address) -> Result<Record> {
    let value:Option<Option<String>>=db.query_row("SELECT CASE WHEN length(CAST(record AS BLOB))<=?2 THEN record END FROM coordination_records WHERE address=?1",params![key(address),MAX_ROW as i64],|r|r.get(0)).optional()?;
    let record = decode_record(
        value
            .ok_or(EnrollmentError::Denied)?
            .ok_or(EnrollmentError::Storage)?,
    )?;
    if record.address != address {
        return Err(EnrollmentError::Storage);
    }
    Ok(record)
}
fn save(tx: &Transaction<'_>, record: &Record) -> Result<()> {
    record.validate().map_err(|_| EnrollmentError::Invalid)?;
    let value = json(record)?;
    if value.len() > MAX_ROW {
        return Err(EnrollmentError::Capacity);
    }
    tx.execute("INSERT INTO coordination_records(address,record) VALUES(?1,?2) ON CONFLICT(address) DO UPDATE SET record=excluded.record",params![key(record.address),value])?;
    Ok(())
}
fn prior(tx: &Transaction<'_>, id: Uuid, digest: &[u8]) -> Result<Option<Record>> {
    let row:Option<(Option<Vec<u8>>,Option<String>)>=tx.query_row("SELECT CASE WHEN length(digest)=32 THEN digest END,CASE WHEN length(CAST(record AS BLOB))<=?2 THEN record END FROM coordination_receipts WHERE id=?1",params![id.to_string(),MAX_ROW as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    row.map(|(old, value)| {
        if old.ok_or(EnrollmentError::Storage)? != digest {
            return Err(EnrollmentError::Conflict);
        }
        decode_record(value.ok_or(EnrollmentError::Storage)?)
    })
    .transpose()
}
fn receipt(
    tx: &Transaction<'_>,
    id: Uuid,
    digest: &[u8],
    record: &Record,
    kind: &str,
) -> Result<()> {
    let limit = if kind == "operator" && record.status == Status::Revoked {
        control::MAX_RECEIPTS
    } else {
        control::MAX_ORDINARY_RECEIPTS
    };
    capacity(tx, "coordination_receipts", limit as i64)?;
    tx.execute(
        "INSERT INTO coordination_receipts VALUES(?1,?2,?3,?4)",
        params![id.to_string(), digest, json(record)?, kind],
    )?;
    Ok(())
}
fn advance(n: u64) -> Result<u64> {
    n.checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(EnrollmentError::Capacity)
}
impl EnrollmentStore {
    /// Explicit opt-in, independently versioned tables; restarting invalidates leases.
    pub fn enable_control(&mut self) -> Result<u64> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='coordination_schema')",[],|r|r.get(0))?;
        if !exists {
            tx.execute_batch("CREATE TABLE coordination_schema(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL,generation INTEGER NOT NULL CHECK(generation>0));
                INSERT INTO coordination_schema VALUES(1,1,1);
                CREATE TABLE coordination_records(address TEXT PRIMARY KEY,record TEXT NOT NULL);
                CREATE TABLE coordination_receipts(id TEXT PRIMARY KEY,digest BLOB NOT NULL,record TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('register','operator')));
                CREATE TABLE coordination_leases(address TEXT NOT NULL REFERENCES coordination_records(address),machine TEXT NOT NULL,lease TEXT NOT NULL,PRIMARY KEY(address,machine));")?;
        } else {
            let (version, n): (i64, i64) = tx.query_row(
                "SELECT version,generation FROM coordination_schema WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if version != 1 || n <= 0 {
                return Err(EnrollmentError::Conflict);
            }
            let next = n.checked_add(1).ok_or(EnrollmentError::Capacity)?;
            tx.execute(
                "UPDATE coordination_schema SET generation=?1 WHERE id=1",
                [next],
            )?;
            tx.execute("DELETE FROM coordination_leases", [])?;
        }
        let n: i64 = tx.query_row(
            "SELECT generation FROM coordination_schema WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(n as u64)
    }
    pub(crate) fn control_peer(
        &mut self,
        instance: u64,
        peer: &Peer,
        operation: &ClientOperation,
        clock: impl Fn() -> Result<i64>,
    ) -> Result<control::Reply> {
        operation.validate().map_err(|_| EnrollmentError::Invalid)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        generation(&tx, instance)?;
        if peer.owner != self.owner || peer.connection.is_nil() || !(peer.live)() {
            return Err(EnrollmentError::Denied);
        }
        current(&tx, peer.machine)?;
        let reply = match operation {
            ClientOperation::Register {
                command_id,
                expires_at_ms,
                address,
                source_revision,
            } => {
                let digest = hash(json(&("register", peer.machine, operation))?.as_bytes());
                if let Some(record) = prior(&tx, *command_id, &digest)? {
                    control::Reply::Registered {
                        record,
                        duplicate: true,
                    }
                } else {
                    let now = checked_clock(&tx, &clock)?;
                    deadline(*expires_at_ms, now)?;
                    let exists: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM coordination_records WHERE address=?1)",
                        [key(*address)],
                        |r| r.get(0),
                    )?;
                    if exists {
                        return Err(EnrollmentError::Conflict);
                    }
                    capacity(&tx, "coordination_records", control::MAX_RECORDS as i64)?;
                    let record = Record {
                        address: *address,
                        source: peer.machine,
                        source_revision: *source_revision,
                        revision: 1,
                        scope_revision: 1,
                        coordinator_epoch: 1,
                        coordinator: peer.machine,
                        participants: vec![peer.machine],
                        status: Status::Configured,
                        execution_authority: ExecutionAuthority::None,
                    };
                    save(&tx, &record)?;
                    receipt(&tx, *command_id, &digest, &record, "register")?;
                    control::Reply::Registered {
                        record,
                        duplicate: false,
                    }
                }
            }
            ClientOperation::ObserveRegistration {
                command_id,
                expires_at_ms,
                address,
            } => {
                let stored: Option<Option<String>> = tx.query_row(
                    "SELECT CASE WHEN length(CAST(record AS BLOB))<=?2 THEN record END FROM coordination_receipts WHERE id=?1 AND kind='register'",
                    params![command_id.to_string(), MAX_ROW as i64], |r|r.get(0)).optional()?;
                if let Some(stored) = stored {
                    let record = decode_record(stored.ok_or(EnrollmentError::Storage)?)?;
                    if record.address != *address {
                        return Err(EnrollmentError::Denied);
                    }
                    if record.source.machine_id != peer.machine.machine_id
                        && load(&tx, *address)?.source != peer.machine
                    {
                        return Err(EnrollmentError::Denied);
                    }
                    control::Reply::Registered {
                        record,
                        duplicate: true,
                    }
                } else {
                    let now = checked_clock(&tx, &clock)?;
                    if now < *expires_at_ms {
                        return Err(EnrollmentError::Conflict);
                    }
                    control::Reply::NotRegistered {
                        command_id: *command_id,
                        address: *address,
                        expires_at_ms: *expires_at_ms,
                        observed_at_ms: now,
                    }
                }
            }
            ClientOperation::Release {} => {
                // Only this connection's leases: an old close cannot erase a replacement.
                let mut statement=tx.prepare("SELECT CASE WHEN length(CAST(address AS BLOB))<=73 THEN address END,CASE WHEN length(CAST(lease AS BLOB))<=4096 THEN lease END FROM coordination_leases WHERE machine=?1 LIMIT ?2")?;
                let rows = statement
                    .query_map(
                        params![
                            peer.machine.machine_id.to_string(),
                            control::MAX_RECORDS as i64 + 1
                        ],
                        |r| {
                            Ok((
                                r.get::<_, Option<String>>(0)?,
                                r.get::<_, Option<String>>(1)?,
                            ))
                        },
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if rows.len() > control::MAX_RECORDS {
                    return Err(EnrollmentError::Storage);
                }
                for (address, value) in rows {
                    let address = address.ok_or(EnrollmentError::Storage)?;
                    let value = value.ok_or(EnrollmentError::Storage)?;
                    let lease: Lease =
                        serde_json::from_str(&value).map_err(|_| EnrollmentError::Storage)?;
                    lease.validate().map_err(|_| EnrollmentError::Storage)?;
                    if lease.connection_id == peer.connection {
                        tx.execute(
                            "DELETE FROM coordination_leases WHERE address=?1 AND machine=?2",
                            params![address, peer.machine.machine_id.to_string()],
                        )?;
                    }
                }
                control::Reply::Released {}
            }
            ClientOperation::Refresh {} => {
                let now = checked_clock(&tx, &clock)?;
                let mut statement=tx.prepare("SELECT CASE WHEN length(CAST(address AS BLOB))<=73 THEN address END,CASE WHEN length(CAST(record AS BLOB))<=?1 THEN record END FROM coordination_records ORDER BY address LIMIT ?2")?;
                let rows = statement
                    .query_map(
                        params![MAX_ROW as i64, control::MAX_RECORDS as i64 + 1],
                        |r| {
                            Ok((
                                r.get::<_, Option<String>>(0)?,
                                r.get::<_, Option<String>>(1)?,
                            ))
                        },
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if rows.len() > control::MAX_RECORDS {
                    return Err(EnrollmentError::Storage);
                }
                let mut views = Vec::new();
                for (address, value) in rows {
                    let record = decode_record(value.ok_or(EnrollmentError::Storage)?)?;
                    if address.as_deref() != Some(key(record.address).as_str()) {
                        return Err(EnrollmentError::Storage);
                    }
                    let coordinator = record.coordinator == peer.machine;
                    let participant = record.participants.contains(&peer.machine);
                    // Out-of-scope peers cannot enumerate records, including private source IDs.
                    if record.source != peer.machine && !coordinator && !participant {
                        continue;
                    }
                    let valid = record.status == Status::Configured
                        && (coordinator || participant)
                        && current(&tx, record.source).is_ok();
                    let lease = if valid {
                        let existing:Option<Option<String>>=tx.query_row("SELECT CASE WHEN length(CAST(lease AS BLOB))<=4096 THEN lease END FROM coordination_leases WHERE address=?1 AND machine=?2",params![key(record.address),peer.machine.machine_id.to_string()],|r|r.get(0)).optional()?;
                        let old = existing
                            .map(|value| {
                                serde_json::from_str::<Lease>(
                                    &value.ok_or(EnrollmentError::Storage)?,
                                )
                                .map_err(|_| EnrollmentError::Storage)
                            })
                            .transpose()?;
                        if let Some(old) = &old {
                            old.validate().map_err(|_| EnrollmentError::Storage)?;
                        }
                        let id = old
                            .filter(|l| {
                                l.server_generation == instance
                                    && l.connection_id == peer.connection
                                    && l.machine == peer.machine
                                    && l.record_revision == record.revision
                                    && l.expires_at_ms > now
                            })
                            .map_or_else(Uuid::new_v4, |l| l.lease_id);
                        let lease = Lease {
                            lease_id: id,
                            server_generation: instance,
                            connection_id: peer.connection,
                            machine: peer.machine,
                            record_revision: record.revision,
                            coordinator,
                            participant,
                            expires_at_ms: now
                                .checked_add(control::LEASE_MS)
                                .ok_or(EnrollmentError::Invalid)?,
                        };
                        tx.execute("INSERT INTO coordination_leases VALUES(?1,?2,?3) ON CONFLICT(address,machine) DO UPDATE SET lease=excluded.lease",params![key(record.address),peer.machine.machine_id.to_string(),json(&lease)?])?;
                        Some(lease)
                    } else {
                        tx.execute(
                            "DELETE FROM coordination_leases WHERE address=?1 AND machine=?2",
                            params![key(record.address), peer.machine.machine_id.to_string()],
                        )?;
                        None
                    };
                    views.push(View { record, lease });
                }
                control::Reply::Snapshot { views }
            }
        };
        reply.validate().map_err(|_| EnrollmentError::Storage)?;
        if !(peer.live)() {
            return Err(EnrollmentError::Denied);
        }
        tx.commit()?;
        Ok(reply)
    }
    /// Administrative caller MUST authenticate the operator before this boundary.
    pub(crate) fn control_mutate(
        &mut self,
        instance: u64,
        request: &Mutation,
        clock: impl Fn() -> Result<i64>,
    ) -> Result<control::Receipt> {
        request.validate().map_err(|_| EnrollmentError::Invalid)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        generation(&tx, instance)?;
        let digest = hash(json(&("operator", self.owner, request))?.as_bytes());
        if let Some(record) = prior(&tx, request.command_id, &digest)? {
            tx.commit()?;
            return Ok(control::Receipt {
                record,
                duplicate: true,
            });
        }
        let now = checked_clock(&tx, &clock)?;
        deadline(request.expires_at_ms, now)?;
        let mut record = load(&tx, request.address)?;
        if record.revision != request.expected_revision || record.status == Status::Revoked {
            return Err(EnrollmentError::Conflict);
        }
        match &request.operation {
            MutationKind::RebindSource {
                expected_source,
                source,
            } => {
                if record.source != *expected_source {
                    return Err(EnrollmentError::Conflict);
                }
                current(&tx, *source)?;
                record.source = *source;
            }
            MutationKind::Configure {
                coordinator,
                participants,
            } => {
                current(&tx, record.source)?;
                current(&tx, *coordinator)?;
                for participant in participants {
                    current(&tx, *participant)?;
                }
                if record.coordinator != *coordinator {
                    record.coordinator_epoch = advance(record.coordinator_epoch)?;
                }
                record.coordinator = *coordinator;
                record.participants = participants.clone();
                record.scope_revision = advance(record.scope_revision)?;
            }
            MutationKind::Revoke {} => {
                record.status = Status::Revoked;
            }
        }
        record.revision = advance(record.revision)?;
        save(&tx, &record)?;
        receipt(&tx, request.command_id, &digest, &record, "operator")?;
        tx.execute(
            "DELETE FROM coordination_leases WHERE address=?1",
            [key(record.address)],
        )?;
        tx.commit()?;
        Ok(control::Receipt {
            record,
            duplicate: false,
        })
    }
    pub(crate) fn control_inspect(
        &mut self,
        instance: u64,
        address: Address,
        clock: impl Fn() -> Result<i64>,
    ) -> Result<(Record, Vec<Lease>)> {
        address.validate().map_err(|_| EnrollmentError::Invalid)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        generation(&tx, instance)?;
        let now = checked_clock(&tx, &clock)?;
        let record = load(&tx, address)?;
        let mut statement=tx.prepare("SELECT CASE WHEN length(CAST(lease AS BLOB))<=4096 THEN lease END FROM coordination_leases WHERE address=?1 LIMIT ?2")?;
        let values = statement
            .query_map(
                params![key(address), control::MAX_PARTICIPANTS as i64 + 1],
                |r| r.get::<_, Option<String>>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if values.len() > control::MAX_PARTICIPANTS {
            return Err(EnrollmentError::Storage);
        }
        let mut leases = Vec::new();
        for value in values {
            let lease: Lease = serde_json::from_str(&value.ok_or(EnrollmentError::Storage)?)
                .map_err(|_| EnrollmentError::Storage)?;
            lease.validate().map_err(|_| EnrollmentError::Storage)?;
            if lease.server_generation == instance
                && lease.expires_at_ms > now
                && record.status == Status::Configured
                && lease.record_revision == record.revision
                && current(&tx, lease.machine).is_ok()
                && current(&tx, record.source).is_ok()
            {
                control::Reply::Snapshot {
                    views: vec![View {
                        record: record.clone(),
                        lease: Some(lease.clone()),
                    }],
                }
                .validate()
                .map_err(|_| EnrollmentError::Storage)?;
                leases.push(lease);
            }
        }
        drop(statement);
        tx.commit()?;
        Ok((record, leases))
    }
}
