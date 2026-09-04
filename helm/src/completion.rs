//! Provider-neutral completion-readiness contract.
//!
//! This module does not accept final responses or mutate task/agent stores. A caller
//! must persist the ledger and coordinate record reads, membership changes, and
//! final acceptance in one serialized runtime boundary. A snapshot is not a lock.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    subagent::{AgentId, AgentRecord, AgentStatus, AgentTree},
    todo::{TodoId, TodoItem, TodoList, TodoStatus},
};

const VERSION: u32 = 1;
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
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    MissingRecord,
    ActiveAgent,
    NeedsDisposition,
    RecordChanged,
    InvalidDisposition,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Unresolved {
    pub obligation: Obligation,
    pub reason: UnresolvedReason,
}

/// Deliberately contains no transcript, task titles, results or secret-bearing
/// evidence. Fetch individual owned records through normal policy/redaction paths.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Readiness {
    pub run_id: RunId,
    pub revision: u64,
    pub fingerprint: String,
    pub total: usize,
    pub accounted: usize,
    /// Accounted for is not synonymous with successfully completed.
    pub completed: usize,
    pub incomplete: usize,
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
        Self {
            version: VERSION,
            run_id: RunId::default(),
            revision: 0,
            entries: vec![],
        }
    }
    pub fn run_id(&self) -> RunId {
        self.run_id
    }
    pub fn revision(&self) -> u64 {
        self.revision
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
        }
        ensure!(
            bytes.len() <= MAX_LEDGER_BYTES,
            "completion ledger exceeds byte limit"
        );
        let wire: Wire = serde_json::from_slice(bytes).context("malformed completion ledger")?;
        ensure!(
            wire.version == VERSION,
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
        Ok(Self {
            version: wire.version,
            run_id: wire.run_id,
            revision: wire.revision,
            entries: wire.entries,
        })
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
                    }
                    None
                }
            };
            if let Some(reason) = unresolved {
                if snapshot.unresolved.len() < max_unresolved {
                    snapshot.unresolved.push(Unresolved {
                        obligation: entry.obligation,
                        reason,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        subagent::{AgentBudget, AgentPolicy, ApprovalPolicy},
        todo::{Priority, TimelineEntry, TodoScope},
    };
    use chrono::Utc;
    use std::collections::BTreeSet;

    fn todo(status: TodoStatus) -> TodoItem {
        let now = Utc::now();
        TodoItem {
            id: TodoId::new(),
            title: "private unrelated title".into(),
            description: String::new(),
            status,
            priority: Priority::Normal,
            order: 0,
            dependencies: BTreeSet::new(),
            blockers: if status == TodoStatus::Blocked {
                vec!["waiting for operator".into()]
            } else {
                vec![]
            },
            assignees: BTreeSet::new(),
            notes: vec![],
            progress: vec![],
            evidence: vec![TimelineEntry {
                at: now,
                author: None,
                text: "test fixture verified".into(),
            }],
            created_at: now,
            updated_at: now,
            completed_at: None,
            archived_at: None,
        }
    }
    fn agent(status: AgentStatus) -> AgentRecord {
        let now = Utc::now();
        let budget = AgentBudget {
            max_tokens: 10,
            max_runtime_secs: 30,
            max_children: 1,
            max_terminals: 1,
        };
        AgentRecord {
            id: AgentId::new(),
            parent_id: None,
            name: "child".into(),
            task: "private task".into(),
            status,
            policy: AgentPolicy {
                readable_roots: vec![],
                writable_roots: vec![],
                allowed_tools: BTreeSet::new(),
                approval: ApprovalPolicy::Deny,
                budget: budget.clone(),
            },
            budget,
            worktree: None,
            branch: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
            recent_progress: vec![],
            result: Some("private result".into()),
            error: None,
        }
    }
    fn records() -> (TodoList, AgentTree) {
        (
            TodoList {
                version: crate::todo::STORE_VERSION,
                revision: 0,
                scope: TodoScope::workspace("/fixture".into()),
                items: BTreeMap::new(),
            },
            AgentTree::default(),
        )
    }
    fn snapshot(ledger: &RunLedger, todo: &TodoItem) -> Readiness {
        let (mut todos, agents) = records();
        todos.items.insert(todo.id, todo.clone());
        ledger.snapshot(&todos, &agents, 10).unwrap()
    }

    #[test]
    fn empty_runs_and_unrelated_records_are_independent() {
        let ledger = RunLedger::new();
        let (mut todos, mut agents) = records();
        let before = ledger.snapshot(&todos, &agents, 10).unwrap();
        let item = todo(TodoStatus::Pending);
        todos.items.insert(item.id, item);
        let child = agent(AgentStatus::Running);
        agents.agents.insert(child.id, child);
        assert_eq!(before, ledger.snapshot(&todos, &agents, 10).unwrap());
        assert!(before.ready());
        assert_ne!(ledger.run_id(), RunLedger::new().run_id());
        let json = serde_json::to_string(&before).unwrap();
        assert!(!json.contains("private"));
    }

    #[test]
    fn todo_status_disposition_matrix_preserves_unfinished_work() {
        let kinds = [
            DispositionKind::CompletedWithEvidence,
            DispositionKind::CancelledWithReason,
            DispositionKind::BlockedWithImpact,
            DispositionKind::DeferredWithImpact,
            DispositionKind::Incorporated,
            DispositionKind::FailureWithImpact,
            DispositionKind::NotNeededWithReason,
        ];
        for status in [
            TodoStatus::Pending,
            TodoStatus::InProgress,
            TodoStatus::Blocked,
            TodoStatus::Completed,
            TodoStatus::Cancelled,
        ] {
            for kind in kinds {
                let item = todo(status);
                let original = item.clone();
                let mut ledger = RunLedger::new();
                ledger.adopt(Obligation::Todo(item.id), 0).unwrap();
                assert!(!snapshot(&ledger, &item).ready());
                let expected = matches!(
                    (status, kind),
                    (
                        TodoStatus::Completed,
                        DispositionKind::CompletedWithEvidence
                    ) | (TodoStatus::Cancelled, DispositionKind::CancelledWithReason)
                        | (TodoStatus::Blocked, DispositionKind::BlockedWithImpact)
                        | (
                            TodoStatus::Pending | TodoStatus::InProgress | TodoStatus::Blocked,
                            DispositionKind::DeferredWithImpact
                        )
                );
                assert_eq!(
                    ledger
                        .account_todo(&item, kind, "reviewed impact".into(), 1)
                        .is_ok(),
                    expected,
                    "{status:?} {kind:?}"
                );
                let ready = snapshot(&ledger, &item);
                assert_eq!(ready.ready(), expected);
                assert_eq!(
                    ready.completed,
                    usize::from(expected && status == TodoStatus::Completed)
                );
                assert_eq!(
                    ready.incomplete,
                    usize::from(expected && status != TodoStatus::Completed)
                );
                assert_eq!(item, original);
            }
        }
    }

    #[test]
    fn agent_matrix_never_equates_terminal_failure_with_success() {
        let kinds = [
            DispositionKind::Incorporated,
            DispositionKind::FailureWithImpact,
            DispositionKind::NotNeededWithReason,
            DispositionKind::CompletedWithEvidence,
        ];
        for status in [
            AgentStatus::Queued,
            AgentStatus::Running,
            AgentStatus::Waiting,
            AgentStatus::Completed,
            AgentStatus::Failed,
            AgentStatus::Cancelled,
            AgentStatus::Interrupted,
            AgentStatus::TimedOut,
        ] {
            for kind in kinds {
                let item = agent(status.clone());
                let mut ledger = RunLedger::new();
                ledger.adopt(Obligation::Agent(item.id), 0).unwrap();
                let expected = status.is_terminal()
                    && (kind == DispositionKind::NotNeededWithReason
                        || (kind == DispositionKind::Incorporated
                            && status == AgentStatus::Completed)
                        || (kind == DispositionKind::FailureWithImpact
                            && status != AgentStatus::Completed));
                assert_eq!(
                    ledger
                        .account_agent(&item, kind, "reviewed impact".into(), 1)
                        .is_ok(),
                    expected,
                    "{status:?} {kind:?}"
                );
                let (todos, mut agents) = records();
                agents.agents.insert(item.id, item);
                let ready = ledger.snapshot(&todos, &agents, 10).unwrap();
                assert_eq!(ready.ready(), expected);
                assert_eq!(
                    ready.incomplete,
                    usize::from(expected && status != AgentStatus::Completed)
                );
                if !status.is_terminal() {
                    assert_eq!(ready.unresolved[0].reason, UnresolvedReason::ActiveAgent);
                }
            }
        }
    }

    #[test]
    fn evidence_results_and_reasons_are_required() {
        let mut item = todo(TodoStatus::Completed);
        item.evidence.clear();
        let mut ledger = RunLedger::new();
        ledger.adopt(Obligation::Todo(item.id), 0).unwrap();
        assert!(
            ledger
                .account_todo(
                    &item,
                    DispositionKind::CompletedWithEvidence,
                    "claimed done".into(),
                    1
                )
                .is_err()
        );
        item.evidence = todo(TodoStatus::Completed).evidence;
        for reason in [
            "".into(),
            " \t".into(),
            "bad\x1btext".into(),
            "日".repeat(MAX_REASON_BYTES / 3 + 1),
        ] {
            assert!(
                ledger
                    .account_todo(&item, DispositionKind::CompletedWithEvidence, reason, 1)
                    .is_err()
            );
            assert_eq!(ledger.revision(), 1);
        }
        let mut child = agent(AgentStatus::Completed);
        child.result = None;
        ledger.adopt(Obligation::Agent(child.id), 1).unwrap();
        assert!(
            ledger
                .account_agent(
                    &child,
                    DispositionKind::Incorporated,
                    "read result".into(),
                    2
                )
                .is_err()
        );
        item.status = TodoStatus::Blocked;
        item.blockers.clear();
        assert!(
            ledger
                .account_todo(
                    &item,
                    DispositionKind::BlockedWithImpact,
                    "blocked".into(),
                    2
                )
                .is_err()
        );
    }

    #[test]
    fn archive_delete_and_reopen_cannot_hide_obligations() {
        let mut item = todo(TodoStatus::Completed);
        let mut ledger = RunLedger::new();
        ledger.adopt(Obligation::Todo(item.id), 0).unwrap();
        ledger
            .account_todo(
                &item,
                DispositionKind::CompletedWithEvidence,
                "verified".into(),
                1,
            )
            .unwrap();
        assert!(snapshot(&ledger, &item).ready());
        item.archived_at = Some(Utc::now());
        assert_eq!(
            snapshot(&ledger, &item).unresolved[0].reason,
            UnresolvedReason::RecordChanged
        );
        ledger
            .account_todo(
                &item,
                DispositionKind::CompletedWithEvidence,
                "verified archived evidence".into(),
                2,
            )
            .unwrap();
        assert!(snapshot(&ledger, &item).ready());
        let (todos, agents) = records();
        let missing = ledger.snapshot(&todos, &agents, 10).unwrap();
        assert_eq!(
            missing.unresolved[0].reason,
            UnresolvedReason::MissingRecord
        );
        assert!(!missing.ready());
        item.status = TodoStatus::Pending;
        assert!(!snapshot(&ledger, &item).ready());
    }

    #[test]
    fn changed_result_membership_and_evidence_invalidate_tokens() {
        let item = todo(TodoStatus::Completed);
        let mut ledger = RunLedger::new();
        ledger.adopt(Obligation::Todo(item.id), 0).unwrap();
        ledger
            .account_todo(
                &item,
                DispositionKind::CompletedWithEvidence,
                "verified".into(),
                1,
            )
            .unwrap();
        let first = snapshot(&ledger, &item);
        let mut edited = item.clone();
        edited.evidence[0].text = "retracted".into();
        assert_ne!(snapshot(&ledger, &edited).fingerprint, first.fingerprint);
        assert!(!snapshot(&ledger, &edited).ready());
        let child = agent(AgentStatus::Completed);
        ledger.adopt(Obligation::Agent(child.id), 2).unwrap();
        assert_ne!(snapshot(&ledger, &item).fingerprint, first.fingerprint);
        ledger
            .account_agent(
                &child,
                DispositionKind::Incorporated,
                "result included".into(),
                3,
            )
            .unwrap();
        let (mut todos, mut agents) = records();
        todos.items.insert(item.id, item);
        agents.agents.insert(child.id, child.clone());
        assert!(ledger.snapshot(&todos, &agents, 10).unwrap().ready());
        agents.agents.get_mut(&child.id).unwrap().result = Some("late replacement".into());
        assert!(!ledger.snapshot(&todos, &agents, 10).unwrap().ready());
    }

    #[test]
    fn nested_children_followups_and_stale_mutations() {
        let mut ledger = RunLedger::new();
        let parent = AgentId::new();
        let child = AgentId::new();
        let followup = AgentId::new();
        assert!(ledger.adopt_child(parent, child, 0).is_err());
        ledger.adopt(Obligation::Agent(parent), 0).unwrap();
        assert!(ledger.adopt_child(parent, child, 0).is_err());
        ledger.adopt_child(parent, child, 1).unwrap();
        ledger.adopt_child(child, followup, 2).unwrap();
        assert!(ledger.adopt_child(parent, parent, 3).is_err());
        ledger.adopt(Obligation::Agent(child), 3).unwrap();
        assert_eq!(ledger.revision(), 3); // duplicate adoption cannot reset review
        let (todos, agents) = records();
        assert_eq!(ledger.snapshot(&todos, &agents, 10).unwrap().total, 3);
        assert!(
            RunLedger::new()
                .snapshot(&todos, &agents, 10)
                .unwrap()
                .ready()
        );
    }

    #[test]
    fn bounded_output_never_treats_omitted_work_as_ready() {
        let mut ledger = RunLedger::new();
        for revision in 0..MAX_OBLIGATIONS {
            ledger
                .adopt(Obligation::Todo(TodoId::new()), revision as u64)
                .unwrap();
        }
        assert!(
            ledger
                .adopt(Obligation::Todo(TodoId::new()), MAX_OBLIGATIONS as u64)
                .is_err()
        );
        let (todos, agents) = records();
        for cap in [0, 1, 17, MAX_OBLIGATIONS] {
            let snapshot = ledger.snapshot(&todos, &agents, cap).unwrap();
            assert!(!snapshot.ready());
            assert_eq!(snapshot.unresolved.len(), cap);
            assert_eq!(snapshot.omitted_unresolved, MAX_OBLIGATIONS - cap);
        }
    }

    #[test]
    fn roundtrip_and_fail_closed_migration() {
        let item = todo(TodoStatus::Pending);
        let mut ledger = RunLedger::new();
        ledger.adopt(Obligation::Todo(item.id), 0).unwrap();
        ledger
            .account_todo(
                &item,
                DispositionKind::DeferredWithImpact,
                "requires operator input".into(),
                1,
            )
            .unwrap();
        let bytes = ledger.to_json().unwrap();
        let restored = RunLedger::from_json(&bytes).unwrap();
        assert_eq!(ledger, restored);
        assert_eq!(snapshot(&ledger, &item), snapshot(&restored, &item));
        for bytes in [
            b"".as_slice(),
            b"{}",
            b"null",
            &vec![b' '; MAX_LEDGER_BYTES + 1],
        ] {
            assert!(RunLedger::from_json(bytes).is_err());
        }
        let value = serde_json::to_value(&ledger).unwrap();
        for field in ["version", "run_id", "revision", "entries"] {
            let mut invalid = value.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(RunLedger::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut invalid = value.clone();
        invalid["version"] = serde_json::json!(VERSION + 1);
        assert!(RunLedger::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
        let mut invalid = value.clone();
        invalid["entries"]
            .as_array_mut()
            .unwrap()
            .push(value["entries"][0].clone());
        assert!(RunLedger::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
        let mut invalid = value;
        invalid["entries"][0]["dispositions"][0]["reviewed"] = serde_json::json!("bad");
        assert!(RunLedger::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }

    #[test]
    fn review_history_is_retained_and_capacity_failure_is_atomic() {
        let item = todo(TodoStatus::Pending);
        let mut ledger = RunLedger::new();
        ledger.adopt(Obligation::Todo(item.id), 0).unwrap();
        for n in 0..MAX_REVIEWS_PER_OBLIGATION {
            ledger
                .account_todo(
                    &item,
                    DispositionKind::DeferredWithImpact,
                    format!("review {n}: operator input still required"),
                    ledger.revision(),
                )
                .unwrap();
        }
        let before = ledger.clone();
        assert!(
            ledger
                .account_todo(
                    &item,
                    DispositionKind::DeferredWithImpact,
                    "one more review".into(),
                    ledger.revision()
                )
                .is_err()
        );
        assert_eq!(ledger, before);
        let restored = RunLedger::from_json(&ledger.to_json().unwrap()).unwrap();
        assert_eq!(
            restored.entries[0].dispositions.len(),
            MAX_REVIEWS_PER_OBLIGATION
        );
        assert!(
            restored.entries[0].dispositions[0]
                .reason
                .starts_with("review 0:")
        );
    }

    #[test]
    fn mismatched_store_identity_fails_instead_of_using_another_record() {
        let item = todo(TodoStatus::Completed);
        let mut ledger = RunLedger::new();
        ledger.adopt(Obligation::Todo(item.id), 0).unwrap();
        let (mut todos, mut agents) = records();
        todos.items.insert(item.id, todo(TodoStatus::Completed));
        assert!(ledger.snapshot(&todos, &agents, 10).is_err());
        let child = agent(AgentStatus::Completed);
        ledger = RunLedger::new();
        ledger.adopt(Obligation::Agent(child.id), 0).unwrap();
        agents
            .agents
            .insert(child.id, agent(AgentStatus::Completed));
        assert!(ledger.snapshot(&todos, &agents, 10).is_err());
    }

    #[test]
    fn revision_overflow_does_not_mutate_membership() {
        let mut ledger = RunLedger::new();
        ledger.revision = u64::MAX;
        let before = ledger.clone();
        assert!(
            ledger
                .adopt(Obligation::Todo(TodoId::new()), u64::MAX)
                .is_err()
        );
        assert_eq!(ledger, before);
    }
}
