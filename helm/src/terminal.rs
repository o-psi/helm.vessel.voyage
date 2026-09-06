//! UI-facing contract for human attachment to interactive terminals.
//!
//! Attached bytes are deliberately opaque to the agent runtime. Implementations must
//! not copy input or output into model messages, sessions, approvals, or tracing.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TerminalId(pub Uuid);

impl std::fmt::Display for TerminalId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalState {
    Running,
    Exited { code: Option<i32> },
    Disconnected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalSummary {
    pub id: TerminalId,
    pub title: String,
    pub state: TerminalState,
}

/// A terminal-emulator snapshot, not an unprocessed PTY transcript. Runtime adapters
/// own ANSI/VT parsing and expose the currently visible screen through this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSnapshot {
    pub id: TerminalId,
    pub title: String,
    pub state: TerminalState,
    pub revision: u64,
    pub cells: Vec<Vec<TerminalCell>>,
    pub cursor: Option<(u16, u16)>,
    /// Total transcript bytes evicted from the bounded agent-read buffer.
    pub dropped_unread_bytes: u64,
    /// Human-only capture state; never serialize screen contents into model records.
    pub privacy: Option<TerminalPrivacy>,
}

/// Privacy omissions are distinct from eviction of the bounded model buffer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalPrivacy {
    pub discarded_unread_bytes: u64,
    pub suppressed_output_bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalCell {
    pub text: String,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underlined: bool,
    pub reversed: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    Changed(TerminalId),
    Added(TerminalId),
    Removed(TerminalId),
}

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("terminal {0} does not exist")]
    NotFound(TerminalId),
    #[error("terminal transport is closed")]
    Closed,
    #[error("terminal operation failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait InteractiveTerminals: Send + Sync {
    async fn list(&self) -> Result<Vec<TerminalSummary>, TerminalError>;
    /// Permanently disable model capture before returning a human-visible screen.
    /// Detachment must not reopen capture; already disclosed output is unchanged.
    async fn attach(&self, id: TerminalId) -> Result<TerminalSnapshot, TerminalError>;
    async fn snapshot(&self, id: TerminalId) -> Result<TerminalSnapshot, TerminalError>;
    async fn write(&self, id: TerminalId, bytes: Vec<u8>) -> Result<(), TerminalError>;
    async fn resize(&self, id: TerminalId, columns: u16, rows: u16) -> Result<(), TerminalError>;
    fn subscribe(&self) -> broadcast::Receiver<TerminalEvent>;
}

/// Default adapter used until a runtime terminal manager is installed.
pub struct NoInteractiveTerminals {
    events: broadcast::Sender<TerminalEvent>,
}

impl Default for NoInteractiveTerminals {
    fn default() -> Self {
        let (events, _) = broadcast::channel(1);
        Self { events }
    }
}

#[async_trait]
impl InteractiveTerminals for NoInteractiveTerminals {
    async fn list(&self) -> Result<Vec<TerminalSummary>, TerminalError> {
        Ok(Vec::new())
    }

    async fn attach(&self, id: TerminalId) -> Result<TerminalSnapshot, TerminalError> {
        Err(TerminalError::NotFound(id))
    }

    async fn snapshot(&self, id: TerminalId) -> Result<TerminalSnapshot, TerminalError> {
        Err(TerminalError::NotFound(id))
    }

    async fn write(&self, id: TerminalId, _: Vec<u8>) -> Result<(), TerminalError> {
        Err(TerminalError::NotFound(id))
    }

    async fn resize(&self, id: TerminalId, _: u16, _: u16) -> Result<(), TerminalError> {
        Err(TerminalError::NotFound(id))
    }

    fn subscribe(&self) -> broadcast::Receiver<TerminalEvent> {
        self.events.subscribe()
    }
}

/// Input splitter for a future plain/raw stdio frontend. Both detach chords
/// remain local; the caller must retain `helm_input` for subsequent Helm input.
#[derive(Default)]
pub struct PlainDetachFilter {
    detached: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PlainDetachChunk<'a> {
    pub terminal_input: &'a [u8],
    pub helm_input: &'a [u8],
    pub detached: bool,
}

impl PlainDetachFilter {
    pub fn push<'a>(&mut self, input: &'a [u8]) -> PlainDetachChunk<'a> {
        if self.detached {
            return PlainDetachChunk {
                terminal_input: &input[..0],
                helm_input: input,
                detached: true,
            };
        }
        if let Some(index) = input.iter().position(|byte| matches!(*byte, 0x14 | 0x1d)) {
            self.detached = true;
            PlainDetachChunk {
                terminal_input: &input[..index],
                helm_input: &input[index + 1..],
                detached: true,
            }
        } else {
            PlainDetachChunk {
                terminal_input: input,
                helm_input: &input[..0],
                detached: false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_plain_detach_chords_preserve_all_trailing_bytes_across_every_chunk_boundary() {
        for chord in [0x14, 0x1d] {
            let input = [
                b"before\xc3\xa9".as_slice(),
                &[chord],
                b"after\x14\x1d".as_slice(),
            ]
            .concat();
            for split in 0..=input.len() {
                let mut filter = PlainDetachFilter::default();
                let first = filter.push(&input[..split]);
                let second = filter.push(&input[split..]);
                assert_eq!(
                    [first.terminal_input, second.terminal_input].concat(),
                    b"before\xc3\xa9"
                );
                assert_eq!(
                    [first.helm_input, second.helm_input].concat(),
                    b"after\x14\x1d"
                );
                assert!(second.detached);
                assert_eq!(
                    filter.push(b"next"),
                    PlainDetachChunk {
                        terminal_input: b"",
                        helm_input: b"next",
                        detached: true
                    }
                );
                assert!(filter.push(b"").detached);
            }
        }
    }

    #[test]
    fn plain_filter_forwards_every_other_byte_without_transformation() {
        let input: Vec<_> = (0..=255)
            .filter(|byte| !matches!(*byte, 0x14 | 0x1d))
            .collect();
        let mut filter = PlainDetachFilter::default();
        let output = filter.push(&input);
        assert_eq!(output.terminal_input, input);
        assert!(output.helm_input.is_empty());
        assert!(!output.detached);
        assert!(!filter.push(b"").detached);
    }
}
