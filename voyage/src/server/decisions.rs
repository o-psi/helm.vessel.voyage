use super::*;
use crate::attachment::runtime::{RuntimeClock, SystemClock};
use crate::tools::{ApprovalOutcome, ApprovalRequest, Approver, Question, QuestionAnswer};
use async_trait::async_trait;
use serde_json::{Value, json};
/// A missing interface produces bounded refusal. Silence never grants authority.
#[derive(Clone)]
pub(super) struct Decisions {
    pub owner: ManagedSessionOwner,
    pub run: Uuid,
    pub incarnation: Uuid,
    pub cancel: CancellationToken,
    pub timeout: std::time::Duration,
}
impl Decisions {
    async fn request(
        &self,
        id: Uuid,
        request: Value,
        access: Option<(Arc<crate::policy::LiveAccess>, u64)>,
        cancellation: CancellationToken,
    ) -> Result<Value> {
        // The registration and its cleanup must stay ordered even if a tool or
        // worker future is dropped while the blocking journal insert is running.
        // Dropping the caller cancels this bounded task, not the database write.
        let lifetime = CancellationToken::new();
        let _guard = lifetime.clone().drop_guard();
        let decisions = self.clone();
        tokio::spawn(async move {
            let result = decisions
                .wait_for_decision(id, request, access, cancellation, lifetime)
                .await;
            let outcome = match &result {
                Ok(value) if value == "cancelled" => Some("cancelled"),
                Ok(value) if value == "expired" => Some("expired"),
                Ok(value) if value == "invalidated" => Some("invalidated"),
                Err(_) => Some("invalidated"),
                _ => None,
            };
            if let Some(outcome) = outcome {
                // A committed user response is never overwritten. This update
                // also emits the durable observation that clears Helm's prompt.
                decisions.owner.finish_decision(id, outcome).await?;
            }
            result
        })
        .await?
    }

    async fn wait_for_decision(
        &self,
        id: Uuid,
        request: Value,
        access: Option<(Arc<crate::policy::LiveAccess>, u64)>,
        cancellation: CancellationToken,
        lifetime: CancellationToken,
    ) -> Result<Value> {
        let stale = || {
            access
                .as_ref()
                .is_some_and(|(live, generation)| live.snapshot() != *generation)
        };
        let cancelled =
            || self.cancel.is_cancelled() || cancellation.is_cancelled() || lifetime.is_cancelled();
        if cancelled() {
            return Ok(json!("cancelled"));
        }
        if stale() {
            return Ok(json!("invalidated"));
        }
        let timeout = self.timeout.min(std::time::Duration::from_secs(120));
        let deadline = tokio::time::Instant::now() + timeout;
        let expires = SystemClock
            .now_ms()?
            .checked_add(i64::try_from(timeout.as_millis())?)
            .context("decision expiry overflow")?;
        self.owner
            .create_decision(self.run, self.incarnation, id, expires, request)
            .await?;
        loop {
            if cancelled() {
                return Ok(json!("cancelled"));
            }
            if stale() {
                return Ok(json!("invalidated"));
            }
            // The journal preserves answers committed before the deadline,
            // including when this reader resumes after that deadline.
            if let Some(response) = self.owner.decision_response(id).await? {
                return Ok(response);
            }
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return Ok(json!("cancelled")),
                _ = cancellation.cancelled() => return Ok(json!("cancelled")),
                _ = lifetime.cancelled() => return Ok(json!("cancelled")),
                _ = tokio::time::sleep_until(deadline) => {
                    return Ok(self.owner.decision_response(id).await?.unwrap_or(json!("expired")));
                },
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
            }
        }
    }
}
#[async_trait]
impl Approver for Decisions {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        match self
            .request(
                request.id,
                json!({"kind":"approval","approval":request}),
                request.access_generation.clone(),
                request.cancellation.clone(),
            )
            .await
        {
            Ok(response) if response == "approved" => ApprovalOutcome::Approved,
            Ok(response) if response == "denied" => ApprovalOutcome::Denied,
            Ok(response) if response == "expired" => ApprovalOutcome::Expired,
            Ok(response) if response == "cancelled" => ApprovalOutcome::Cancelled,
            Ok(response) if response == "invalidated" => ApprovalOutcome::Invalidated,
            Ok(_) => ApprovalOutcome::Unavailable,
            Err(_) => ApprovalOutcome::Unavailable,
        }
    }
    async fn ask_question(&self, question: &Question) -> QuestionAnswer {
        match self
            .request(
                Uuid::new_v4(),
                json!({"kind":"question","question":question}),
                None,
                self.cancel.clone(),
            )
            .await
        {
            Ok(response) => serde_json::from_value(response).unwrap_or(QuestionAnswer::Unavailable),
            Err(_) => QuestionAnswer::Unavailable,
        }
    }
}
