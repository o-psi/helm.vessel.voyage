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
    pub message_fingerprints: Vec<String>,
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
            phase: CompletionPhase::Provisional,
            detail: None,
            readiness: None,
            message_start: Some(start),
            message_end: Some(self.messages.len()),
            message_fingerprints: self.messages[start..].iter().map(fingerprint).collect(),
        });
    }

    pub fn update_run_summary(
        &mut self,
        phase: CompletionPhase,
        readiness: Option<crate::completion::Readiness>,
        detail: Option<String>,
    ) {
        if let Some(summary) = self.run_summaries.last_mut() {
            summary.phase = phase;
            summary.readiness = readiness;
            summary.detail =
                detail.map(|s| s.chars().filter(|c| !c.is_control()).take(4000).collect());
        }
    }

    pub fn finish_run_summary(&mut self, stop: &StopReason) {
        self.extend_run_summary();
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

#[cfg(test)]
mod tests {
    use super::*;
    fn session(root: &Path) -> Session {
        Session::new(root.into(), "fixture".into())
    }
    fn proposal(session: &mut Session, prompt: &str) -> Uuid {
        session
            .messages
            .push(Message::new(crate::Role::User, prompt));
        let id = Uuid::new_v4();
        session.begin_run_summary(id);
        session
            .messages
            .push(Message::new(crate::Role::Assistant, "premature final"));
        session
            .messages
            .push(Message::new(crate::Role::Assistant, "verified final"));
        id
    }

    #[tokio::test]
    async fn summaries_preserve_provisional_and_terminal_text_on_resume_export_and_branch() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path().join("sessions"));
        for stop in [
            StopReason::Completed,
            StopReason::Incomplete {
                reason: "remaining work".into(),
                readiness: None,
            },
        ] {
            let mut session = session(root.path());
            let run_id = proposal(&mut session, "work");
            session
                .completion_runs
                .push(crate::completion::runtime::RunReference {
                    session_id: session.id,
                    run_id,
                });
            let canonical = serde_json::to_value(&session.messages).unwrap();
            session.finish_run_summary(&stop);
            assert_eq!(session.assistant_classification(1), Some("provisional"));
            let expected = if matches!(stop, StopReason::Completed) {
                "completed"
            } else {
                "incomplete"
            };
            assert_eq!(session.assistant_classification(2), Some(expected));
            store.save(&mut session).await.unwrap();
            let restored = store.load(session.id).await.unwrap();
            assert_eq!(serde_json::to_value(&restored.messages).unwrap(), canonical);
            assert_eq!(restored.assistant_classification(1), Some("provisional"));
            assert_eq!(restored.assistant_classification(2), Some(expected));
            let branch = store.branch(&restored, None).await.unwrap();
            assert!(branch.completion_runs.is_empty());
            assert_eq!(branch.run_summaries[0].run_id, run_id);
            assert_eq!(branch.assistant_classification(2), Some(expected));
            let path = root.path().join("export.md");
            store.export_markdown(&restored, &path).await.unwrap();
            assert!(
                std::fs::read_to_string(path)
                    .unwrap()
                    .contains(&format!("classification: {expected}"))
            );
        }
    }

    #[tokio::test]
    async fn unsealed_run_loads_interrupted_clear_removes_annotations_and_legacy_still_loads() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path().join("sessions"));
        let mut session = session(root.path());
        proposal(&mut session, "work");
        session.extend_run_summary();
        session.update_run_summary(CompletionPhase::Reconciling, None, None);
        store.save(&mut session).await.unwrap();
        let mut restored = store.load(session.id).await.unwrap();
        assert_eq!(
            restored.run_summaries[0].phase,
            CompletionPhase::Interrupted
        );
        assert_eq!(restored.assistant_classification(2), Some("interrupted"));
        restored.clear_conversation();
        assert!(restored.run_summaries.is_empty());
        let mut legacy = serde_json::to_value(&restored).unwrap();
        legacy.as_object_mut().unwrap().remove("run_summaries");
        let legacy: Session = serde_json::from_value(legacy).unwrap();
        assert!(legacy.run_summaries.is_empty());
    }

    #[test]
    fn compaction_and_replacement_never_reclassify_unrelated_or_changed_messages() {
        let mut s = session(Path::new("/tmp"));
        proposal(&mut s, "first");
        s.finish_run_summary(&StopReason::Completed);
        proposal(&mut s, "second");
        s.finish_run_summary(&StopReason::Incomplete {
            reason: "blocked".into(),
            readiness: None,
        });
        let recent = s.messages[3..].to_vec();
        s.replace_messages(recent);
        assert_eq!(s.run_summaries[0].message_start, None);
        assert_eq!(s.run_summaries[1].message_start, Some(0));
        assert_eq!(s.assistant_classification(2), Some("incomplete"));
        s.messages[2].content = "different text".into();
        assert_eq!(s.assistant_classification(2), None);
        s.compact(2);
        assert!(
            s.run_summaries
                .iter()
                .all(|summary| summary.message_start.is_none())
        );
    }
}
