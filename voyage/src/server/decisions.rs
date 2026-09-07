use super::*;
use crate::attachment::runtime::{RuntimeClock, SystemClock};
use crate::tools::{ApprovalOutcome, ApprovalRequest, Approver, Question, QuestionAnswer};
use async_trait::async_trait;
use serde_json::{Value, json};
/// A missing interface produces bounded refusal. Silence never grants authority.
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
        access: Option<&(Arc<crate::policy::LiveAccess>, u64)>,
    ) -> Result<Value> {
        let stale = || access.is_some_and(|(live, generation)| live.snapshot() != *generation);
        if stale() {
            return Ok(json!("invalidated"));
        }
        let timeout = self.timeout.min(std::time::Duration::from_secs(120));
        let expires = SystemClock
            .now_ms()?
            .checked_add(i64::try_from(timeout.as_millis())?)
            .context("decision expiry overflow")?;
        self.owner
            .create_decision(self.run, self.incarnation, id, expires, request)
            .await?;
        let result = tokio::time::timeout(timeout, async {
            loop {
                if stale() {
                    // Also catches requests inserted just after the access transaction.
                    self.owner.dismiss_approval(id).await?;
                    return Ok(json!("invalidated"));
                }
                tokio::select! {
                    _ = self.cancel.cancelled() => return Ok(json!("cancelled")),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                        if let Some(response) = self.owner.decision_response(id).await? { return Ok(response); }
                    }
                }
            }
        }).await;
        match result {
            Ok(response) => response,
            Err(_) => {
                if self.cancel.is_cancelled() {
                    return Ok(json!("cancelled"));
                }
                if stale() {
                    self.owner.dismiss_approval(id).await?;
                    return Ok(json!("invalidated"));
                }
                // A response may have committed before expiry but after the last poll.
                // Preserve that durable answer instead of misreporting an expiry.
                Ok(self
                    .owner
                    .decision_response(id)
                    .await?
                    .unwrap_or(json!("expired")))
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
                request.access_generation.as_ref(),
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
            )
            .await
        {
            Ok(response) => serde_json::from_value(response).unwrap_or(QuestionAnswer::Unavailable),
            Err(_) => QuestionAnswer::Unavailable,
        }
    }
}
