//! Bounded inspection evidence, separate from immutable final agent records.
use super::SubagentEvent;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    path::Path,
};
use uuid::Uuid;

pub(crate) const MAX_EVENTS: usize = 4096;
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryCursor {
    pub epoch: Uuid,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum HistoryNotice {
    Missing,
    Corrupt,
    Restarted,
    WriteFailed,
}

#[derive(Clone, Debug)]
pub struct HistoryStatus {
    pub cursor: HistoryCursor,
    pub first_sequence: Option<u64>,
    pub evicted_through: u64,
    pub notices: BTreeSet<HistoryNotice>,
    pub durable: bool,
    pub cursor_gap: bool,
}

#[derive(Clone, Debug)]
pub struct HistoryReplay {
    pub events: Vec<SubagentEvent>,
    pub status: HistoryStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EventHistory {
    version: u32,
    epoch: Uuid,
    sequence: u64,
    evicted_through: u64,
    notices: BTreeSet<HistoryNotice>,
    pub events: VecDeque<SubagentEvent>,
    #[serde(skip)]
    durable: bool,
    #[serde(skip)]
    blocked: bool,
    #[serde(skip)]
    event_bytes: usize,
}
impl EventHistory {
    pub fn open(path: Option<&Path>, limit: usize, prior_evidence: bool) -> Self {
        let mut value = Self {
            version: 1,
            epoch: Uuid::new_v4(),
            sequence: 0,
            evicted_through: 0,
            notices: BTreeSet::new(),
            events: VecDeque::new(),
            durable: false,
            blocked: false,
            event_bytes: 0,
        };
        if let Some(path) = path {
            match read(path) {
                Ok(Some(saved)) => {
                    value = saved;
                    if value.sequence > 0 || prior_evidence {
                        value.notices.insert(HistoryNotice::Restarted);
                    }
                }
                Ok(None) => {
                    if prior_evidence {
                        value.notices.insert(HistoryNotice::Missing);
                    }
                }
                Err(_) => {
                    value.notices.insert(HistoryNotice::Corrupt);
                    value.blocked = true;
                }
            }
            value.event_bytes = value.events.iter().map(event_size).sum();
            value.trim(limit);
            value.persist(path);
        }
        value
    }
    pub fn push(&mut self, mut event: SubagentEvent, limit: usize) -> SubagentEvent {
        if self.sequence == u64::MAX {
            self.epoch = Uuid::new_v4();
            self.sequence = 0;
            self.evicted_through = 0;
            self.events.clear();
            self.event_bytes = 0;
            self.notices.insert(HistoryNotice::Corrupt);
        }
        self.sequence += 1;
        event.sequence = self.sequence;
        self.event_bytes += event_size(&event);
        self.events.push_back(event.clone());
        self.trim(limit);
        event
    }
    fn trim(&mut self, limit: usize) {
        // Include JSON escaping in the byte budget, reserving the envelope.
        while self.events.len() > limit || self.event_bytes > MAX_BYTES as usize - 1024 {
            let event = self.events.pop_front().expect("nonempty oversized history");
            self.evicted_through = event.sequence;
            self.event_bytes -= event_size(&event);
        }
    }
    pub fn write_failed(&mut self) {
        self.durable = false;
        self.notices.insert(HistoryNotice::WriteFailed);
    }
    pub fn persist(&mut self, path: &Path) {
        // Never overwrite unreadable or corrupt evidence. An operator can move
        // it aside and restart after preserving it for recovery.
        self.durable = !self.blocked && write(path, self).is_ok();
        if !self.durable {
            self.write_failed();
        }
    }
    pub fn replay(&self, after: Option<HistoryCursor>) -> HistoryReplay {
        let cursor_gap = after.is_some_and(|c| {
            c.epoch != self.epoch || c.sequence < self.evicted_through || c.sequence > self.sequence
        });
        let sequence = if cursor_gap {
            0
        } else {
            after.map_or(0, |c| c.sequence)
        };
        HistoryReplay {
            events: self
                .events
                .iter()
                .filter(|e| e.sequence > sequence)
                .cloned()
                .collect(),
            status: HistoryStatus {
                cursor: HistoryCursor {
                    epoch: self.epoch,
                    sequence: self.sequence,
                },
                first_sequence: self.events.front().map(|e| e.sequence),
                evicted_through: self.evicted_through,
                notices: self.notices.clone(),
                durable: self.durable,
                cursor_gap,
            },
        }
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && !self.epoch.is_nil() && self.events.len() <= MAX_EVENTS,
            "invalid event history envelope"
        );
        let mut sequence = self.evicted_through;
        for event in &self.events {
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("invalid event sequence"))?;
            ensure!(event.sequence == sequence, "event history sequence gap");
            let text = match &event.kind {
                super::SubagentEventKind::Progress { text } => Some(text),
                super::SubagentEventKind::Completed { result } => Some(&result.summary),
                super::SubagentEventKind::Failed { error } => Some(error),
                _ => None,
            };
            ensure!(
                text.is_none_or(|s| s.len() <= 4120),
                "oversized event preview"
            );
        }
        ensure!(sequence == self.sequence, "invalid event history tail");
        Ok(())
    }
}

fn event_size(event: &SubagentEvent) -> usize {
    serde_json::to_vec(event)
        .expect("event serialization is infallible")
        .len()
        + 1 // Include the array separator (also conservative for the last event).
}

fn read(path: &Path) -> Result<Option<EventHistory>> {
    let Some(bytes) = super::history_storage::read(path, MAX_BYTES)? else {
        return Ok(None);
    };
    let history: EventHistory = serde_json::from_slice(&bytes)?;
    history.validate()?;
    Ok(Some(history))
}
fn write(path: &Path, value: &EventHistory) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "history exceeds size limit"
    );
    super::history_storage::write(path, &bytes)
}
