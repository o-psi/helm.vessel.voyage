//! Bounded observation projections. Negotiation is not authorization. Adapters
//! must recheck authenticated scope and current sharing before every disclosure.
use crate::attachment::{MAX_PROMPT_BYTES, label};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_REPLAY_EVENTS: usize = 128;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    SequencedEvents,
    Replay,
    ToolActivity,
    Usage,
    ManagedExecution,
    CoordinationControl,
}
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Features(Vec<Feature>);
impl Features {
    pub fn new(values: Vec<Feature>) -> Result<Self, &'static str> {
        let value = Self(values);
        value.validate()?;
        Ok(value)
    }
    pub fn with(mut self, feature: Feature) -> Result<Self, &'static str> {
        if !self.contains(feature) {
            self.0.push(feature);
        }
        self.validate()?;
        Ok(self)
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn contains(&self, feature: Feature) -> bool {
        self.0.contains(&feature)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.0.len() > 6
            || self
                .0
                .iter()
                .enumerate()
                .any(|(i, f)| self.0[..i].contains(f))
        {
            return Err("invalid feature set");
        }
        if self.0.iter().any(|f| *f != Feature::CoordinationControl)
            && !self.contains(Feature::SequencedEvents)
        {
            return Err("event feature dependency missing");
        }
        Ok(())
    }
    /// Intersection describes protocol support only, never a Capability grant.
    pub fn negotiate(&self, supported: &Self) -> Result<Self, &'static str> {
        self.validate()?;
        supported.validate()?;
        Self::new(
            self.0
                .iter()
                .copied()
                .filter(|f| supported.contains(*f))
                .collect(),
        )
    }
    pub fn confirm(&self, selected: &Self) -> Result<(), &'static str> {
        self.validate()?;
        selected.validate()?;
        if selected.0.iter().any(|f| !self.contains(*f)) {
            return Err("unoffered feature selected");
        }
        Ok(())
    }
    pub fn permits_event(&self, event: &RunEvent) -> Result<(), &'static str> {
        self.validate()?;
        let extra = match event {
            RunEvent::ToolStarted { .. } | RunEvent::ToolFinished { .. } => Feature::ToolActivity,
            RunEvent::Usage { .. } => Feature::Usage,
            RunEvent::Cleanup { .. } => Feature::ManagedExecution,
            _ => Feature::SequencedEvents,
        };
        if !self.contains(Feature::SequencedEvents) || !self.contains(extra) {
            return Err("event feature not negotiated");
        }
        Ok(())
    }
}
/// Zero means no observation; emitted events must use positive cursors. Signed
/// range matches the durable Journal's sequence representation.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct EventCursor(u64);
impl<'de> Deserialize<'de> for EventCursor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
impl EventCursor {
    pub fn new(value: u64) -> Result<Self, &'static str> {
        bound(value)?;
        Ok(Self(value))
    }
    pub fn get(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalState {
    Completed,
    Incomplete,
    Cancelled,
    Failed,
    Interrupted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}
/// Terminal model state is never proof of observed owned-resource cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupState {
    Pending,
    Observed,
    Unconfirmed,
    OperatorAttested,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunEvent {
    Accepted {
        command_id: Uuid,
        revision: u64,
    },
    Running {},
    CanonicalCheckpoint {
        revision: u64,
    },
    TextDelta {
        text: String,
    },
    ToolStarted {
        tool_call_id: Uuid,
        name: String,
    },
    ToolFinished {
        tool_call_id: Uuid,
        outcome: ToolOutcome,
    },
    /// Cumulative per-run counters. Receivers replace, never sum replayed usage.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    CancellationRequested {},
    Terminal {
        state: TerminalState,
    },
    /// Explicitly negotiated lifecycle observation; may follow the terminal event.
    Cleanup {
        state: CleanupState,
    },
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequencedEvent {
    pub cursor: EventCursor,
    pub run_id: Uuid,
    pub event: RunEvent,
}
impl SequencedEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        identity(self.run_id)?;
        bound(self.cursor.0)?;
        if self.cursor.0 == 0 {
            return Err("zero event cursor");
        }
        match &self.event {
            RunEvent::Accepted {
                command_id,
                revision,
            } => {
                identity(*command_id)?;
                bound(*revision)?;
            }
            RunEvent::CanonicalCheckpoint { revision } => bound(*revision)?,
            RunEvent::TextDelta { text } => {
                if text.is_empty() || text.len() > MAX_PROMPT_BYTES || text.contains('\0') {
                    return Err("invalid event text");
                }
            }
            RunEvent::ToolStarted { tool_call_id, name } => {
                identity(*tool_call_id)?;
                label(name).map_err(|_| "invalid tool name")?;
            }
            RunEvent::ToolFinished { tool_call_id, .. } => identity(*tool_call_id)?,
            RunEvent::Usage {
                input_tokens,
                output_tokens,
            } => {
                bound(*input_tokens)?;
                bound(*output_tokens)?;
            }
            _ => (),
        }
        Ok(())
    }
}
pub(crate) fn bound(value: u64) -> Result<(), &'static str> {
    if value > i64::MAX as u64 {
        Err("event cursor or counter out of range")
    } else {
        Ok(())
    }
}
pub(crate) fn identity(value: Uuid) -> Result<(), &'static str> {
    if value.is_nil() {
        Err("nil event identity")
    } else {
        Ok(())
    }
}
pub fn validate_replay(
    after: EventCursor,
    latest: EventCursor,
    events: &[SequencedEvent],
) -> Result<(), &'static str> {
    bound(after.0)?;
    bound(latest.0)?;
    if after > latest || events.len() > MAX_REPLAY_EVENTS {
        return Err("invalid replay bounds");
    }
    let mut cursor = after.0;
    for event in events {
        event.validate()?;
        if event.cursor.0 != cursor + 1 || event.cursor > latest {
            return Err("replay gap or stale event");
        }
        cursor = event.cursor.0;
    }
    if events.is_empty() && after != latest {
        return Err("missing replay requires snapshot");
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    Applied,
    Duplicate,
}
/// Receive-side ordering only. Does not authorize projections, execute effects,
/// certify snapshots, or validate a run's complete historical state machine.
pub struct EventSequence {
    connection_id: Uuid,
    session_id: Uuid,
    cursor: EventCursor,
    last: Option<SequencedEvent>,
    features: Features,
}
impl EventSequence {
    /// `cursor` must come from an already authorized applied observation/snapshot.
    pub fn new(
        connection_id: Uuid,
        session_id: Uuid,
        cursor: EventCursor,
        features: Features,
    ) -> Result<Self, &'static str> {
        identity(connection_id)?;
        identity(session_id)?;
        bound(cursor.0)?;
        features.validate()?;
        Ok(Self {
            connection_id,
            session_id,
            cursor,
            last: None,
            features,
        })
    }
    pub fn cursor(&self) -> EventCursor {
        self.cursor
    }
    fn scope(&self, connection_id: Uuid, session_id: Uuid) -> Result<(), &'static str> {
        if connection_id != self.connection_id || session_id != self.session_id {
            return Err("stale event scope");
        }
        Ok(())
    }
    pub fn observe(
        &mut self,
        connection_id: Uuid,
        session_id: Uuid,
        event: SequencedEvent,
    ) -> Result<Observation, &'static str> {
        self.scope(connection_id, session_id)?;
        event.validate()?;
        self.features.permits_event(&event.event)?;
        if self.last.as_ref() == Some(&event) {
            return Ok(Observation::Duplicate);
        }
        if event.cursor.0 != self.cursor.0 + 1 {
            return Err("event gap or stale cursor");
        }
        self.cursor = event.cursor;
        self.last = Some(event);
        Ok(Observation::Applied)
    }
    /// Validate the entire page before advancing, so invalid tails cannot partially
    /// apply. Returned observations are data only; replay never repeats tool calls.
    pub fn replay(
        &mut self,
        connection_id: Uuid,
        session_id: Uuid,
        after: EventCursor,
        latest: EventCursor,
        events: &[SequencedEvent],
    ) -> Result<(), &'static str> {
        self.scope(connection_id, session_id)?;
        if !self.features.contains(Feature::Replay) || after != self.cursor {
            return Err("replay not negotiated or stale base");
        }
        validate_replay(after, latest, events)?;
        for event in events {
            self.features.permits_event(&event.event)?;
        }
        if let Some(last) = events.last() {
            self.cursor = last.cursor;
            self.last = Some(last.clone());
        }
        Ok(())
    }
}
