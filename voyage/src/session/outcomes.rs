//! Local presentation metadata; never provider messages or execution authority.
use super::*;
use crate::agent::{CompletionPhase, StopReason};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSummary {
    pub run_id: Uuid,
    pub phase: CompletionPhase,
    pub detail: Option<String>,
    pub readiness: Option<crate::completion::Readiness>,
    /// Half-open canonical message range; absent when compacted or ambiguous.
    pub message_start: Option<usize>,
    pub message_end: Option<usize>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    pub message_fingerprints: Vec<String>,
    #[serde(default)]
    pub partial_output: String,
}

fn fingerprint(message: &Message) -> String {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "role": message.role, "content": message.content,
        "tool_calls": message.tool_calls, "tool_call_id": message.tool_call_id,
    }))
    .expect("message fingerprint serialization");
    hex::encode(Sha256::digest(bytes))
}

impl Session {
    pub fn begin_run_summary(&mut self, run_id: Uuid) {
        let start = self.messages.len().saturating_sub(1);
        self.run_summaries.push(RunSummary {
            run_id,
            started_at: Some(Utc::now()),
            finished_at: None,
            phase: CompletionPhase::Provisional,
            detail: None,
            readiness: None,
            message_start: Some(start),
            message_end: Some(self.messages.len()),
            message_fingerprints: self.messages[start..].iter().map(fingerprint).collect(),
            partial_output: String::new(),
        });
    }

    /// Cumulative current-response partial text; never canonical provider history.
    pub fn set_run_partial_output(&mut self, text: &str) -> Result<()> {
        anyhow::ensure!(text.len() <= 1024 * 1024, "partial output exceeds one MiB");
        let summary = self
            .run_summaries
            .last_mut()
            .context("no active run summary")?;
        summary.partial_output = text.to_owned();
        Ok(())
    }

    pub fn update_run_summary(
        &mut self,
        phase: CompletionPhase,
        readiness: Option<crate::completion::Readiness>,
        detail: Option<String>,
    ) {
        if let Some(summary) = self.run_summaries.last_mut() {
            if matches!(
                phase,
                CompletionPhase::Completed
                    | CompletionPhase::Incomplete
                    | CompletionPhase::Failed
                    | CompletionPhase::Interrupted
            ) {
                summary.finished_at.get_or_insert_with(Utc::now);
            }
            summary.phase = phase;
            summary.readiness = readiness;
            summary.detail =
                detail.map(|s| s.chars().filter(|c| !c.is_control()).take(4000).collect());
        }
    }

    pub fn finish_run_summary(&mut self, stop: &StopReason) {
        self.extend_run_summary();
        if let Some(summary) = self.run_summaries.last_mut() {
            summary.partial_output.clear();
        }
        match stop {
            StopReason::Completed => {
                self.update_run_summary(CompletionPhase::Completed, None, None)
            }
            StopReason::Incomplete { reason, readiness } => self.update_run_summary(
                CompletionPhase::Incomplete,
                readiness.clone(),
                Some(reason.clone()),
            ),
        }
    }

    pub fn interrupt_run_summary(&mut self, detail: String) {
        self.extend_run_summary();
        let readiness = self.run_summaries.last().and_then(|s| s.readiness.clone());
        self.update_run_summary(CompletionPhase::Interrupted, readiness, Some(detail));
    }

    pub fn fail_run_summary(&mut self, detail: String) {
        self.extend_run_summary();
        let readiness = self.run_summaries.last().and_then(|s| s.readiness.clone());
        self.update_run_summary(CompletionPhase::Failed, readiness, Some(detail));
    }

    /// Refresh the current annotation range after a durable canonical checkpoint.
    pub fn refresh_active_run_summary(&mut self) {
        self.extend_run_summary();
    }

    fn extend_run_summary(&mut self) {
        if let Some(summary) = self.run_summaries.last_mut()
            && let Some(start) = summary
                .message_start
                .filter(|start| *start < self.messages.len())
        {
            summary.message_end = Some(self.messages.len());
            summary.message_fingerprints = self.messages[start..].iter().map(fingerprint).collect();
        }
    }

    /// Re-anchor classifications after compaction/recovery, without changing text.
    pub fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.reanchor_run_summaries();
    }

    pub fn compact(&mut self, retain: usize) -> usize {
        let removed = compact_messages(&mut self.messages, retain);
        self.reanchor_run_summaries();
        removed
    }

    pub(super) fn reanchor_run_summaries(&mut self) {
        let hashes: Vec<_> = self.messages.iter().map(fingerprint).collect();
        for summary in &mut self.run_summaries {
            let expected = &summary.message_fingerprints;
            let old = summary
                .message_start
                .zip(summary.message_end)
                .filter(|(start, end)| {
                    *start <= *end && *end <= hashes.len() && hashes[*start..*end] == *expected
                })
                .map(|(start, _)| start);
            let start = if old.is_some() {
                old
            } else if expected.is_empty() || expected.len() > hashes.len() {
                None
            } else {
                let mut matches = hashes
                    .windows(expected.len())
                    .enumerate()
                    .filter(|(_, window)| *window == expected);
                let first = matches.next().map(|(index, _)| index);
                if matches.next().is_none() {
                    first
                } else {
                    None
                }
            };
            summary.message_start = start;
            summary.message_end = start.map(|start| start + expected.len());
        }
    }

    pub fn assistant_classification(&self, index: usize) -> Option<&'static str> {
        let message = self.messages.get(index)?;
        if message.role != crate::Role::Assistant || !message.tool_calls.is_empty() {
            return None;
        }
        for summary in self.run_summaries.iter().rev() {
            let (Some(start), Some(end)) = (summary.message_start, summary.message_end) else {
                continue;
            };
            if index < start || index >= end || end > self.messages.len() {
                continue;
            }
            if summary.message_fingerprints.get(index - start) != Some(&fingerprint(message)) {
                continue;
            }
            let last = (start..end).rev().find(|i| {
                self.messages[*i].role == crate::Role::Assistant
                    && self.messages[*i].tool_calls.is_empty()
            });
            return Some(if last == Some(index) {
                match summary.phase {
                    CompletionPhase::Completed => "completed",
                    CompletionPhase::Incomplete => "incomplete",
                    CompletionPhase::Failed => "failed",
                    CompletionPhase::Interrupted => "interrupted",
                    _ => "provisional",
                }
            } else {
                "provisional"
            });
        }
        None
    }

    pub(super) fn recover_run_summaries(&mut self) {
        self.reanchor_run_summaries();
        for summary in &mut self.run_summaries {
            if matches!(
                summary.phase,
                CompletionPhase::Provisional | CompletionPhase::Reconciling
            ) {
                summary.phase = CompletionPhase::Interrupted;
                summary.detail = Some("Execution was not durably finalized before restart".into());
            }
        }
    }
}
