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
    async fn request(&self, id: Uuid, request: Value) -> Result<Value> {
        let timeout = self.timeout.min(std::time::Duration::from_secs(120));
        let expires = SystemClock
            .now_ms()?
            .checked_add(i64::try_from(timeout.as_millis())?)
            .context("decision expiry overflow")?;
        self.owner
            .create_decision(self.run, self.incarnation, id, expires, request)
            .await?;
        tokio::time::timeout(timeout,async{loop{tokio::select!{_ = self.cancel.cancelled()=>anyhow::bail!("decision cancelled"),_=tokio::time::sleep(std::time::Duration::from_millis(50))=>{if let Some(response)=self.owner.decision_response(id).await?{return Ok(response)}}}}}).await?
    }
}
#[async_trait]
impl Approver for Decisions {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        match self
            .request(request.id, json!({"kind":"approval","approval":request}))
            .await
        {
            Ok(response) if response == "approved" => ApprovalOutcome::Approved,
            Ok(_) => ApprovalOutcome::Denied,
            Err(_) => ApprovalOutcome::Unavailable,
        }
    }
    async fn ask_question(&self, question: &Question) -> QuestionAnswer {
        match self
            .request(
                Uuid::new_v4(),
                json!({"kind":"question","question":question}),
            )
            .await
        {
            Ok(response) => serde_json::from_value(response).unwrap_or(QuestionAnswer::Unavailable),
            Err(_) => QuestionAnswer::Unavailable,
        }
    }
}
