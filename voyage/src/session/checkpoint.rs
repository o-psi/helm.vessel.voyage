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

    /// Preserve committed usage when cleanup fails without canonical recovery.
    /// Reset to the pre-run baseline only when the caller has a complete outcome
    /// or recovery whose cumulative usage it will add exactly once.
    pub async fn snapshot_after_run(
        &self,
        result: &Result<crate::agent::AgentOutcome, crate::agent::AgentError>,
    ) -> Session {
        let mut session = self.session.lock().await.clone();
        if result.is_ok()
            || result
                .as_ref()
                .err()
                .is_some_and(|error| error.recovery().is_some())
        {
            session.usage = self.baseline_usage.clone();
        }
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

    async fn working_context(&self) -> Result<crate::context::WorkingContext, CheckpointError> {
        let session = self.session.lock().await;
        self.validate(&session)?;
        session
            .working_context
            .validate(&session.messages)
            .map_err(|_| CheckpointError)?;
        Ok(session.working_context.clone())
    }

    async fn save_working_context(
        &self,
        context: &crate::context::WorkingContext,
    ) -> Result<(), CheckpointError> {
        let mut session = self.session.lock().await;
        self.validate(&session)?;
        context
            .validate(&session.messages)
            .map_err(|_| CheckpointError)?;
        let mut next = session.clone();
        next.working_context = context.clone();
        self.persist(&mut next).await?;
        *session = next;
        Ok(())
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
mod working_context_tests {
    use super::*;
    use crate::model::Role;

    #[tokio::test]
    async fn working_projection_checkpoint_survives_reload_without_replacing_canonical() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let mut session = Session::new(directory.path().to_path_buf(), "fixture".into());
        for index in 0..12 {
            session
                .messages
                .push(Message::new(Role::User, format!("question {index}")));
            session.messages.push(Message::new(
                Role::Assistant,
                format!("answer {index}: {}", "source details ".repeat(1000)),
            ));
        }
        let run_id = Uuid::new_v4();
        session.begin_run_summary(run_id);
        store.save(&mut session).await.unwrap();
        let id = session.id;
        let canonical = serde_json::to_value(&session.messages).unwrap();
        let anchors = session.run_summaries[0].message_fingerprints.clone();
        let start = session.run_summaries[0].message_start;
        let end = session.run_summaries[0].message_end;
        let revision = session.revision;
        let checkpoint = SessionCheckpoint::new(session, Some(store.clone()), run_id);
        let mut context = checkpoint.working_context().await.unwrap();
        let source = checkpoint.snapshot_for_finish().await;
        assert!(context.compact(&source.messages, 4).unwrap() > 0);
        checkpoint.save_working_context(&context).await.unwrap();
        let loaded = store.load(id).await.unwrap();
        assert!(loaded.revision > revision);
        assert_eq!(serde_json::to_value(&loaded.messages).unwrap(), canonical);
        assert_eq!(loaded.run_summaries[0].message_fingerprints, anchors);
        assert_eq!(loaded.run_summaries[0].message_start, start);
        assert_eq!(loaded.run_summaries[0].message_end, end);
        assert_eq!(
            serde_json::to_value(&loaded.working_context).unwrap(),
            serde_json::to_value(&context).unwrap()
        );
        assert!(
            serde_json::to_vec(&loaded.working_context.project(&loaded.messages).unwrap())
                .unwrap()
                .len()
                < serde_json::to_vec(&loaded.messages).unwrap().len()
        );
    }

    #[tokio::test]
    async fn wrong_run_cannot_read_or_save_projection() {
        let mut session = Session::new(".".into(), "fixture".into());
        session.messages.push(Message::new(Role::User, "hello"));
        session.begin_run_summary(Uuid::new_v4());
        let checkpoint = SessionCheckpoint::new(session, None, Uuid::new_v4());
        assert!(checkpoint.working_context().await.is_err());
        assert!(
            checkpoint
                .save_working_context(&Default::default())
                .await
                .is_err()
        );
    }
}
