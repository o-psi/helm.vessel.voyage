//! Provider-neutral completion-readiness contract.
//!
//! This module does not accept final responses or mutate task/agent stores. A caller
//! must persist the ledger and coordinate record reads, membership changes, and
//! final acceptance in one serialized runtime boundary. A snapshot is not a lock.
pub mod runtime;
pub mod store;
pub mod tool;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    subagent::{AgentId, AgentRecord, AgentStatus, AgentTree},
    todo::{TodoId, TodoItem, TodoList, TodoStatus},
};

const VERSION: u32 = 2;
pub const MAX_OBLIGATIONS: usize = 1024;
pub const MAX_REASON_BYTES: usize = 4096;
pub const MAX_REVIEWS_PER_OBLIGATION: usize = 16;
pub const MAX_LEDGER_BYTES: usize = 8 * 1024 * 1024;

/// A run is neither a session nor an individual child execution.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunId(pub Uuid);
impl Default for RunId {
    fn default() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Obligation {
    Todo(TodoId),
    Agent(AgentId),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DispositionKind {
    CompletedWithEvidence,
    CancelledWithReason,
    BlockedWithImpact,
    DeferredWithImpact,
    Incorporated,
    FailureWithImpact,
    NotNeededWithReason,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Disposition {
    kind: DispositionKind,
    reason: String,
    /// Fingerprint of the exact record reviewed, not merely its terminal status.
    reviewed: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Entry {
    obligation: Obligation,
    dispositions: Vec<Disposition>,
}

/// Membership is append-only: archival/deletion/unassignment cannot remove an
/// obligation. Legacy records are unowned until a caller explicitly adopts them.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RunLedger {
    version: u32,
    run_id: RunId,
    revision: u64,
    entries: Vec<Entry>,
    state: LedgerState,
}

/// A terminal decision is an immutable historical acceptance boundary. Later
/// edits to shared records do not rewrite it; revalidate before relying on it.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinalOutcome {
    Completed,
    Incomplete,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FinalDecision {
    pub outcome: FinalOutcome,
    pub readiness: Readiness,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "status",
    content = "decision",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum LedgerState {
    Open,
    Sealed(FinalDecision),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    MissingRecord,
    ActiveAgent,
    NeedsDisposition,
    RecordChanged,
    InvalidDisposition,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "status", rename_all = "snake_case")]
pub enum WorkStatus {
    Todo(TodoStatus),
    Agent(AgentStatus),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Unresolved {
    pub obligation: Obligation,
    pub reason: UnresolvedReason,
    pub status: Option<WorkStatus>,
}

/// Deliberately contains no transcript, task titles, results or secret-bearing
/// evidence. Fetch individual owned records through normal policy/redaction paths.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Readiness {
    pub run_id: RunId,
    pub revision: u64,
    pub fingerprint: String,
    pub total: usize,
    pub accounted: usize,
    /// Accounted for is not synonymous with successfully completed.
    pub completed: usize,
    pub incomplete: usize,
    /// All accounted unfinished obligations, bounded by MAX_OBLIGATIONS.
    pub incomplete_obligations: Vec<Obligation>,
    pub unresolved: Vec<Unresolved>,
    pub omitted_unresolved: usize,
}
impl Readiness {
    pub fn ready(&self) -> bool {
        self.accounted == self.total
    }
}

impl Default for RunLedger {
    fn default() -> Self {
        Self::new()
    }
}
impl RunLedger {
    pub fn new() -> Self {
        Self::with_id(RunId::default())
    }
    pub fn with_id(run_id: RunId) -> Self {
        Self {
            version: VERSION,
            run_id,
            revision: 0,
            entries: vec![],
            state: LedgerState::Open,
        }
    }
    pub fn obligations(&self) -> impl Iterator<Item = Obligation> + '_ {
        self.entries.iter().map(|entry| entry.obligation)
    }
    pub fn run_id(&self) -> RunId {
        self.run_id
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn decision(&self) -> Option<&FinalDecision> {
        match &self.state {
            LedgerState::Open => None,
            LedgerState::Sealed(decision) => Some(decision),
        }
    }
    pub(crate) fn ensure_open(&self) -> Result<()> {
        ensure!(
            self.decision().is_none(),
            "completion run is sealed; start a new run"
        );
        Ok(())
    }
    // Only the coordinated runtime constructs this from its private observation.
    fn seal(
        &mut self,
        readiness: Readiness,
        outcome: FinalOutcome,
        reason: Option<String>,
    ) -> Result<()> {
        self.check_revision(readiness.revision)?;
        let mut next = self.clone();
        next.bump()?;
        next.state = LedgerState::Sealed(FinalDecision {
            outcome,
            readiness,
            reason,
        });
        next.validate_decision()?;
        next.to_json()?;
        *self = next;
        Ok(())
    }
    fn validate_decision(&self) -> Result<()> {
        let Some(decision) = self.decision() else {
            return Ok(());
        };
        let observed = &decision.readiness;
        ensure!(
            observed.run_id == self.run_id
                && observed.revision.checked_add(1) == Some(self.revision)
                && observed.total == self.entries.len(),
            "invalid sealed completion identity/revision"
        );
        ensure!(
            observed.fingerprint.len() == 64
                && observed.fingerprint.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid sealed completion fingerprint"
        );
        ensure!(
            observed.completed.checked_add(observed.incomplete) == Some(observed.accounted)
                && observed.accounted <= observed.total
                && observed.omitted_unresolved == 0
                && observed.unresolved.len() == observed.total - observed.accounted
                && observed.incomplete_obligations.len() == observed.incomplete,
            "invalid sealed completion counts"
        );
        let owned: std::collections::BTreeSet<_> = self.obligations().collect();
        let mut seen = std::collections::BTreeSet::new();
        for id in observed
            .unresolved
            .iter()
            .map(|u| u.obligation)
            .chain(observed.incomplete_obligations.iter().copied())
        {
            ensure!(
                owned.contains(&id) && seen.insert(id),
                "invalid sealed completion obligations"
            );
        }
        match decision.outcome {
            FinalOutcome::Completed => ensure!(
                observed.ready() && observed.incomplete == 0 && decision.reason.is_none(),
                "successful completion requires fully successful readiness"
            ),
            FinalOutcome::Incomplete | FinalOutcome::Interrupted => validate_reason(
                decision
                    .reason
                    .as_deref()
                    .context("non-success completion requires a reason")?,
            )?,
        }
        Ok(())
    }

    /// Decoding is fail-closed. Do not substitute a new empty run on an error or
    /// missing ledger when a persisted run reference says one should exist.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            version: u32,
            run_id: RunId,
            revision: u64,
            entries: Vec<Entry>,
            #[serde(default)]
            state: Option<LedgerState>,
        }
        ensure!(
            bytes.len() <= MAX_LEDGER_BYTES,
            "completion ledger exceeds byte limit"
        );
        let wire: Wire = serde_json::from_slice(bytes).context("malformed completion ledger")?;
        ensure!(
            wire.version == 1 || wire.version == VERSION,
            "unsupported completion ledger version"
        );
        ensure!(
            wire.entries.len() <= MAX_OBLIGATIONS,
            "too many completion obligations"
        );
        let mut seen = std::collections::BTreeSet::new();
        for entry in &wire.entries {
            ensure!(
                seen.insert(entry.obligation),
                "duplicate completion obligation"
            );
            ensure!(
                entry.dispositions.len() <= MAX_REVIEWS_PER_OBLIGATION,
                "too many disposition reviews"
            );
            for disposition in &entry.dispositions {
                validate_reason(&disposition.reason)?;
                ensure!(
                    disposition.reviewed.len() == 64
                        && disposition.reviewed.bytes().all(|c| c.is_ascii_hexdigit()),
                    "invalid reviewed fingerprint"
                );
            }
        }
        let state = match (wire.version, wire.state) {
            (1, None) => LedgerState::Open,
            (VERSION, Some(state)) => state,
            _ => anyhow::bail!("missing or incompatible completion decision state"),
        };
        let ledger = Self {
            version: VERSION,
            run_id: wire.run_id,
            revision: wire.revision,
            entries: wire.entries,
            state,
        };
        ledger.validate_decision()?;
        Ok(ledger)
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= MAX_LEDGER_BYTES,
            "completion ledger exceeds byte limit"
        );
        Ok(bytes)
    }

    /// Trusted runtime code calls this on creation or explicit adoption. Merely
    /// listing, assigning, or loading an old record must not call this method.
    pub fn adopt(&mut self, obligation: Obligation, expected_revision: u64) -> Result<()> {
        self.check_revision(expected_revision)?;
        if self
            .entries
            .iter()
            .any(|entry| entry.obligation == obligation)
        {
            return Ok(());
        }
        ensure!(
            self.entries.len() < MAX_OBLIGATIONS,
            "too many completion obligations"
        );
        let mut next = self.clone();
        next.bump()?;
        next.entries.push(Entry {
            obligation,
            dispositions: vec![],
        });
        next.to_json()?;
        *self = next;
        Ok(())
    }

    /// Register descendants and terminal follow-ups at their trusted spawn boundary,
    /// before execution/publication. The parent must already belong to this run.
    pub fn adopt_child(
        &mut self,
        parent: AgentId,
        child: AgentId,
        expected_revision: u64,
    ) -> Result<()> {
        ensure!(parent != child, "agent cannot be its own child");
        ensure!(
            self.entries
                .iter()
                .any(|entry| entry.obligation == Obligation::Agent(parent)),
            "parent is not owned by this run"
        );
        self.adopt(Obligation::Agent(child), expected_revision)
    }

    /// Dispositions are bound to exact observed records. Re-review after any edit,
    /// late result, restart transition, or follow-up that changes the record.
    pub fn account_todo(
        &mut self,
        item: &TodoItem,
        kind: DispositionKind,
        reason: String,
        expected_revision: u64,
    ) -> Result<()> {
        ensure!(
            valid_todo(item, kind),
            "disposition does not account for todo status/evidence"
        );
        self.account(
            Obligation::Todo(item.id),
            kind,
            reason,
            digest(item)?,
            expected_revision,
        )
    }
    pub fn account_agent(
        &mut self,
        item: &AgentRecord,
        kind: DispositionKind,
        reason: String,
        expected_revision: u64,
    ) -> Result<()> {
        ensure!(
            valid_agent(item, kind),
            "disposition does not account for agent status/result"
        );
        self.account(
            Obligation::Agent(item.id),
            kind,
            reason,
            digest(item)?,
            expected_revision,
        )
    }
    fn account(
        &mut self,
        obligation: Obligation,
        kind: DispositionKind,
        reason: String,
        reviewed: String,
        expected_revision: u64,
    ) -> Result<()> {
        self.check_revision(expected_revision)?;
        validate_reason(&reason)?;
        let index = self
            .entries
            .iter()
            .position(|entry| entry.obligation == obligation)
            .context("obligation is not owned by this run")?;
        ensure!(
            self.entries[index].dispositions.len() < MAX_REVIEWS_PER_OBLIGATION,
            "too many disposition reviews"
        );
        let mut next = self.clone();
        next.bump()?;
        next.entries[index].dispositions.push(Disposition {
            kind,
            reason,
            reviewed,
        });
        next.to_json()?;
        *self = next;
        Ok(())
    }
    fn check_revision(&self, expected: u64) -> Result<()> {
        self.ensure_open()?;
        ensure!(
            self.revision == expected,
            "stale completion ledger revision"
        );
        Ok(())
    }
    fn bump(&mut self) -> Result<()> {
        self.revision = self
            .revision
            .checked_add(1)
            .context("completion revision overflow")?;
        Ok(())
    }

    /// Includes archived records. A missing ID stays unresolved even if the backing
    /// store was replaced with an empty one. All obligations are evaluated even when
    /// the diagnostic output cap is zero. Unrelated records do not affect the token.
    pub fn snapshot(
        &self,
        todos: &TodoList,
        agents: &AgentTree,
        max_unresolved: usize,
    ) -> Result<Readiness> {
        let mut snapshot = Readiness {
            run_id: self.run_id,
            revision: self.revision,
            fingerprint: String::new(),
            total: self.entries.len(),
            accounted: 0,
            completed: 0,
            incomplete: 0,
            incomplete_obligations: vec![],
            unresolved: vec![],
            omitted_unresolved: 0,
        };
        let mut fingerprints = BTreeMap::new();
        for entry in &self.entries {
            let record = match entry.obligation {
                Obligation::Todo(id) => todos.items.get(&id).map(|item| {
                    ensure!(item.id == id, "completion record identity mismatch");
                    Ok::<_, anyhow::Error>((
                        digest(item)?,
                        false,
                        item.status == TodoStatus::Completed,
                        entry
                            .dispositions
                            .last()
                            .is_some_and(|d| valid_todo(item, d.kind)),
                    ))
                }),
                Obligation::Agent(id) => agents.agents.get(&id).map(|item| {
                    ensure!(item.id == id, "completion record identity mismatch");
                    Ok::<_, anyhow::Error>((
                        digest(item)?,
                        !item.status.is_terminal(),
                        item.status == AgentStatus::Completed,
                        entry
                            .dispositions
                            .last()
                            .is_some_and(|d| valid_agent(item, d.kind)),
                    ))
                }),
            }
            .transpose()?;
            fingerprints.insert(
                entry.obligation,
                record
                    .as_ref()
                    .map(|r: &(String, bool, bool, bool)| r.0.clone()),
            );
            let unresolved = match (record, entry.dispositions.last()) {
                (None, _) => Some(UnresolvedReason::MissingRecord),
                (Some((_, true, _, _)), _) => Some(UnresolvedReason::ActiveAgent),
                (Some(_), None) => Some(UnresolvedReason::NeedsDisposition),
                (Some((current, _, _, _)), Some(d)) if current != d.reviewed => {
                    Some(UnresolvedReason::RecordChanged)
                }
                (Some((_, _, _, false)), _) => Some(UnresolvedReason::InvalidDisposition),
                (Some((_, _, success, true)), _) => {
                    snapshot.accounted += 1;
                    if success {
                        snapshot.completed += 1;
                    } else {
                        snapshot.incomplete += 1;
                        snapshot.incomplete_obligations.push(entry.obligation);
                    }
                    None
                }
            };
            if let Some(reason) = unresolved {
                if snapshot.unresolved.len() < max_unresolved {
                    snapshot.unresolved.push(Unresolved {
                        obligation: entry.obligation,
                        reason,
                        status: match entry.obligation {
                            Obligation::Todo(id) => todos
                                .items
                                .get(&id)
                                .map(|item| WorkStatus::Todo(item.status)),
                            Obligation::Agent(id) => agents
                                .agents
                                .get(&id)
                                .map(|item| WorkStatus::Agent(item.status.clone())),
                        },
                    });
                } else {
                    snapshot.omitted_unresolved += 1;
                }
            }
        }
        // Serialize entries as a sequence: enum keys are not JSON object keys.
        snapshot.fingerprint = digest(&(self, fingerprints.into_iter().collect::<Vec<_>>()))?;
        Ok(snapshot)
    }
}

fn validate_reason(reason: &str) -> Result<()> {
    ensure!(!reason.trim().is_empty(), "disposition reason is required");
    ensure!(
        reason.len() <= MAX_REASON_BYTES,
        "disposition reason exceeds byte limit"
    );
    ensure!(
        !reason.chars().any(char::is_control),
        "disposition reason contains control characters"
    );
    Ok(())
}
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}
fn valid_todo(item: &TodoItem, kind: DispositionKind) -> bool {
    match (item.status, kind) {
        (TodoStatus::Completed, DispositionKind::CompletedWithEvidence) => {
            item.blockers.is_empty() && item.evidence.iter().any(|e| !e.text.trim().is_empty())
        }
        (TodoStatus::Cancelled, DispositionKind::CancelledWithReason) => true,
        (TodoStatus::Blocked, DispositionKind::BlockedWithImpact) => {
            item.blockers.iter().any(|b| !b.trim().is_empty())
        }
        (
            TodoStatus::Pending | TodoStatus::InProgress | TodoStatus::Blocked,
            DispositionKind::DeferredWithImpact,
        ) => true,
        _ => false,
    }
}
fn valid_agent(item: &AgentRecord, kind: DispositionKind) -> bool {
    if !item.status.is_terminal() {
        return false;
    }
    match kind {
        DispositionKind::Incorporated => {
            item.status == AgentStatus::Completed
                && item.result.as_ref().is_some_and(|r| !r.trim().is_empty())
        }
        DispositionKind::FailureWithImpact => item.status != AgentStatus::Completed,
        DispositionKind::NotNeededWithReason => true,
        _ => false,
    }
}
