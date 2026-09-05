//! Session persistence before root final sealing and durable acceptance afterwards.
use super::{Session, SessionStore};
use crate::{
    agent::{CheckpointError, RunCheckpoint, StopReason},
    model::{Message, Usage},
};
use async_trait::async_trait;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Owns the authoritative session revision during an execution. Callers must not
/// concurrently save independent session copies; use a shared frontend callback
/// when accepting additional input while a run is active.
pub struct SessionCheckpoint {
    session: Mutex<Session>,
    store: Option<SessionStore>,
    run_id: Uuid,
    baseline_usage: Usage,
    initial_prefix: Vec<Message>,
}

impl SessionCheckpoint {
    pub fn new(session: Session, store: Option<SessionStore>, run_id: Uuid) -> Self {
        let baseline_usage = session.usage.clone();
        let initial_prefix = session
            .run_summaries
            .last()
            .and_then(|summary| summary.message_start)
            .map(|start| {
                session
                    .messages
                    .iter()
                    .take(start + 1)
                    .filter(|m| m.role != crate::model::Role::System)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Self {
            session: Mutex::new(session),
            store,
            run_id,
            baseline_usage,
            initial_prefix,
        }
    }

    /// Latest committed revision/history, with usage reset so frontend outcome or
    /// typed recovery adds the current run exactly once. This does not alter disk.
    pub async fn snapshot_for_finish(&self) -> Session {
        let mut session = self.session.lock().await.clone();
        session.usage = self.baseline_usage.clone();
        session
    }

    fn validate(&self, session: &Session) -> Result<(), CheckpointError> {
        if self.run_id.is_nil()
            || session.run_summaries.last().map(|s| s.run_id) != Some(self.run_id)
        {
            return Err(CheckpointError);
        }
        Ok(())
    }

    async fn persist(&self, next: &mut Session) -> Result<(), CheckpointError> {
        if let Some(store) = &self.store {
            store.save(next).await.map_err(|_| CheckpointError)?;
        }
        Ok(())
    }
}

#[async_trait]
impl RunCheckpoint for SessionCheckpoint {
    fn run_id(&self) -> Uuid {
        self.run_id
    }

    async fn canonical(&self, messages: &[Message], usage: &Usage) -> Result<(), CheckpointError> {
        let mut session = self.session.lock().await;
        self.validate(&session)?;
        let mut next = session.clone();
        // Reuse canonical receipt merging, but always add run usage to the fixed
        // pre-run baseline, never the previous checkpoint's cumulative usage.
        next.usage = self.baseline_usage.clone();
        let recovery = crate::agent::CanonicalRecovery {
            messages: messages
                .iter()
                .filter(|m| m.role != crate::model::Role::System)
                .cloned()
                .collect(),
            usage: usage.clone(),
        };
        next.recover_context_failure(&recovery)
            .map_err(|_| CheckpointError)?;
        // New assistant/tool messages can separate the previously queued inputs,
        // invalidating the old whole-range fingerprint. Retain the trusted start
        // only when the complete original prefix through its prompt still agrees.
        if !self.initial_prefix.is_empty()
            && next.messages.len() >= self.initial_prefix.len()
            && serde_json::to_value(&next.messages[..self.initial_prefix.len()]).ok()
                == serde_json::to_value(&self.initial_prefix).ok()
        {
            next.run_summaries
                .last_mut()
                .ok_or(CheckpointError)?
                .message_start = Some(self.initial_prefix.len() - 1);
        }
        next.set_run_partial_output("")
            .map_err(|_| CheckpointError)?;
        next.refresh_active_run_summary();
        self.persist(&mut next).await?;
        *session = next;
        Ok(())
    }

    async fn accepted(
        &self,
        messages: &[Message],
        usage: &Usage,
        reason: &StopReason,
    ) -> Result<(), CheckpointError> {
        let mut session = self.session.lock().await;
        self.validate(&session)?;
        let mut next = session.clone();
        // The preceding canonical callback established this exact history/usage.
        // Reject mismatched acknowledgement rather than classifying stale text.
        let input = self
            .baseline_usage
            .input_tokens
            .checked_add(usage.input_tokens)
            .ok_or(CheckpointError)?;
        let output = self
            .baseline_usage
            .output_tokens
            .checked_add(usage.output_tokens)
            .ok_or(CheckpointError)?;
        if next.usage.input_tokens != input
            || next.usage.output_tokens != output
            || serde_json::to_value(&next.messages).map_err(|_| CheckpointError)?
                != serde_json::to_value(messages).map_err(|_| CheckpointError)?
        {
            return Err(CheckpointError);
        }
        next.finish_run_summary(reason);
        self.persist(&mut next).await?;
        *session = next;
        Ok(())
    }

    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        let mut session = self.session.lock().await;
        self.validate(&session)?;
        let current = &session
            .run_summaries
            .last()
            .ok_or(CheckpointError)?
            .partial_output;
        // Fail before allocating or mutating when the response exceeds the bound.
        let length = current
            .len()
            .checked_add(text.len())
            .ok_or(CheckpointError)?;
        if length > 1024 * 1024 {
            return Err(CheckpointError);
        }
        let mut output = String::with_capacity(length);
        output.push_str(current);
        output.push_str(text);
        let mut next = session.clone();
        next.set_run_partial_output(&output)
            .map_err(|_| CheckpointError)?;
        self.persist(&mut next).await?;
        *session = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::{CanonicalRecovery, CompletionPhase, StopReason},
        model::{Role, SteeringStatus},
    };

    fn fixture(root: &std::path::Path, store: bool) -> (SessionCheckpoint, Option<SessionStore>) {
        let store = store.then(|| SessionStore::new(root.join("sessions")));
        let mut session = Session::new(root.into(), "fixture".into());
        session.usage = Usage {
            input_tokens: 100,
            output_tokens: 20,
        };
        session
            .messages
            .push(Message::new(Role::User, "accepted prompt"));
        let run = Uuid::new_v4();
        session.begin_run_summary(run);
        (SessionCheckpoint::new(session, store.clone(), run), store)
    }
    fn run_usage() -> Usage {
        Usage {
            input_tokens: 7,
            output_tokens: 3,
        }
    }

    #[tokio::test]
    async fn canonical_is_durable_before_ack_and_usage_is_added_once() {
        let root = tempfile::tempdir().unwrap();
        let (checkpoint, store) = fixture(root.path(), true);
        let mut messages = checkpoint.snapshot_for_finish().await.messages;
        messages.push(Message::new(Role::Assistant, "canonical proposal"));
        checkpoint.canonical(&messages, &run_usage()).await.unwrap();
        checkpoint.canonical(&messages, &run_usage()).await.unwrap();
        let mut finish = checkpoint.snapshot_for_finish().await;
        let durable = store.as_ref().unwrap().load(finish.id).await.unwrap();
        assert_eq!(durable.messages.len(), 2);
        assert_eq!(durable.messages[1].content, "canonical proposal");
        assert_eq!(durable.usage.input_tokens, 107);
        assert_eq!(durable.usage.output_tokens, 23);
        assert_eq!(finish.usage.input_tokens, 100);
        assert_eq!(finish.revision, durable.revision);
        assert_eq!(durable.run_summaries[0].phase, CompletionPhase::Interrupted);
        assert_eq!(durable.run_summaries[0].message_end, Some(2));
        checkpoint
            .accepted(&messages, &run_usage(), &StopReason::Completed)
            .await
            .unwrap();
        assert_eq!(
            checkpoint.snapshot_for_finish().await.run_summaries[0].phase,
            CompletionPhase::Completed
        );
        let accepted = store.as_ref().unwrap().load(finish.id).await.unwrap();
        assert_eq!(accepted.run_summaries[0].phase, CompletionPhase::Completed);
        finish = checkpoint.snapshot_for_finish().await;
        finish
            .recover_context_failure(&CanonicalRecovery {
                messages,
                usage: run_usage(),
            })
            .unwrap();
        assert_eq!(finish.usage.input_tokens, 107);
        store.unwrap().save(&mut finish).await.unwrap();
    }

    #[tokio::test]
    async fn partial_deltas_are_separate_durable_and_replaced_by_canonical_response() {
        let root = tempfile::tempdir().unwrap();
        let (checkpoint, store) = fixture(root.path(), true);
        checkpoint.partial("first ").await.unwrap();
        checkpoint.partial("世界").await.unwrap();
        let finish = checkpoint.snapshot_for_finish().await;
        let durable = store.unwrap().load(finish.id).await.unwrap();
        assert_eq!(durable.messages.len(), 1);
        assert_eq!(durable.run_summaries[0].partial_output, "first 世界");
        assert_eq!(durable.run_summaries[0].phase, CompletionPhase::Interrupted);
        let mut messages = finish.messages;
        messages.push(Message::new(Role::Assistant, "first 世界"));
        checkpoint.canonical(&messages, &run_usage()).await.unwrap();
        checkpoint.partial("next response").await.unwrap();
        let finish = checkpoint.snapshot_for_finish().await;
        assert_eq!(finish.messages.len(), 2);
        assert_eq!(finish.run_summaries[0].partial_output, "next response");
    }

    #[tokio::test]
    async fn no_save_remains_in_memory_and_overflow_rolls_back() {
        let root = tempfile::tempdir().unwrap();
        let (checkpoint, _) = fixture(root.path(), false);
        checkpoint.partial(&"x".repeat(1024 * 1024)).await.unwrap();
        assert!(checkpoint.partial("é").await.is_err());
        assert_eq!(
            checkpoint.snapshot_for_finish().await.run_summaries[0]
                .partial_output
                .len(),
            1024 * 1024
        );
        let messages = checkpoint.snapshot_for_finish().await.messages;
        assert!(
            checkpoint
                .canonical(
                    &messages,
                    &Usage {
                        input_tokens: u64::MAX,
                        output_tokens: 0
                    }
                )
                .await
                .is_err()
        );
        assert_eq!(
            checkpoint.snapshot_for_finish().await.run_summaries[0]
                .partial_output
                .len(),
            1024 * 1024
        );
        checkpoint.canonical(&messages, &run_usage()).await.unwrap();
        assert!(
            checkpoint.snapshot_for_finish().await.run_summaries[0]
                .partial_output
                .is_empty()
        );
        assert!(!root.path().join("sessions").exists());
    }

    #[tokio::test]
    async fn failed_save_rolls_back_canonical_partial_and_revision() {
        let root = tempfile::tempdir().unwrap();
        let (checkpoint, _) = fixture(root.path(), true);
        let before = checkpoint.snapshot_for_finish().await;
        std::fs::write(root.path().join("sessions"), "not a directory").unwrap();
        assert!(checkpoint.partial("must not appear").await.is_err());
        let mut messages = before.messages.clone();
        messages.push(Message::new(Role::Assistant, "must not appear"));
        assert!(checkpoint.canonical(&messages, &run_usage()).await.is_err());
        let after = checkpoint.snapshot_for_finish().await;
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.messages.len(), 1);
        assert!(after.run_summaries[0].partial_output.is_empty());
    }

    #[tokio::test]
    async fn queued_receipts_survive_and_applied_receipts_do_not_duplicate() {
        let root = tempfile::tempdir().unwrap();
        let (checkpoint, _) = fixture(root.path(), false);
        let queued = Message::steering("late input");
        checkpoint
            .session
            .lock()
            .await
            .messages
            .push(queued.clone());
        let canonical = vec![Message::new(Role::User, "accepted prompt")];
        checkpoint
            .canonical(&canonical, &run_usage())
            .await
            .unwrap();
        let mut applied = queued;
        applied.steering.as_mut().unwrap().status = SteeringStatus::Applied;
        let canonical = vec![
            canonical[0].clone(),
            Message::new(Role::Assistant, "proposal before steering"),
            applied,
        ];
        checkpoint
            .canonical(&canonical, &run_usage())
            .await
            .unwrap();
        let session = checkpoint.snapshot_for_finish().await;
        assert_eq!(session.messages.len(), 3);
        assert_eq!(session.run_summaries[0].message_start, Some(0));
        assert_eq!(session.run_summaries[0].message_end, Some(3));
        assert_eq!(
            session.messages[2].steering.as_ref().unwrap().status,
            SteeringStatus::Applied
        );
    }

    #[tokio::test]
    async fn initial_prompt_and_failed_acceptance_are_honestly_durable() {
        let root = tempfile::tempdir().unwrap();
        let (checkpoint, store) = fixture(root.path(), true);
        let store = store.unwrap();
        let initial = checkpoint.snapshot_for_finish().await;
        checkpoint
            .canonical(&initial.messages, &Usage::default())
            .await
            .unwrap();
        let durable = store.load(initial.id).await.unwrap();
        assert_eq!(durable.messages[0].content, "accepted prompt");
        assert_eq!(durable.usage.input_tokens, 100);
        let mut messages = initial.messages;
        messages.push(Message::new(Role::Assistant, "proposal"));
        checkpoint.canonical(&messages, &run_usage()).await.unwrap();
        let before = checkpoint.snapshot_for_finish().await;
        let mut conflicting = store.load(initial.id).await.unwrap();
        conflicting.name = Some("concurrent update".into());
        store.save(&mut conflicting).await.unwrap();
        assert!(
            checkpoint
                .accepted(&messages, &run_usage(), &StopReason::Completed)
                .await
                .is_err()
        );
        let after = checkpoint.snapshot_for_finish().await;
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.run_summaries[0].phase, CompletionPhase::Provisional);
        assert_eq!(
            store.load(initial.id).await.unwrap().name.as_deref(),
            Some("concurrent update")
        );
    }

    #[tokio::test]
    async fn wrong_run_fails_before_persistence() {
        let root = tempfile::tempdir().unwrap();
        let (mut checkpoint, _) = fixture(root.path(), true);
        checkpoint.run_id = Uuid::new_v4();
        assert!(checkpoint.partial("text").await.is_err());
        assert!(checkpoint.canonical(&[], &Usage::default()).await.is_err());
        assert!(!root.path().join("sessions").exists());
    }
}
