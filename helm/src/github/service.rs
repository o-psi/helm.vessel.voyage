//! Current local authority, exact attended decisions, and single-send publication.
use super::{
    context,
    publication::{Action, Actor, Draft},
    repository::ObjectKind,
    store::{Operation, Owner, Receipt, State, Store},
    transport::Client,
};
use crate::{
    config::AccessMode,
    tools::{ApprovalOutcome, InteractionMode, ToolContext},
};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};
use uuid::Uuid;

pub(super) async fn database<T: Send + 'static>(
    work: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
) -> Result<T> {
    database_at(Store::default_path(), work).await
}
pub(super) async fn database_at<T: Send + 'static>(
    path: PathBuf,
    work: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
) -> Result<T> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let slot = SLOTS
            .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(8)))
            .clone()
            .acquire_owned()
            .await?;
        tokio::task::spawn_blocking(move || {
            let _slot = slot;
            std::fs::create_dir_all(
                path.parent()
                    .ok_or_else(|| anyhow::anyhow!("GitHub store parent unavailable"))?,
            )?;
            let mut store = Store::open(path)?;
            work(&mut store)
        })
        .await?
    })
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "GitHub operation storage deadline elapsed; inspect its receipt before proceeding"
        )
    })?;
    result.map_err(|_| anyhow::anyhow!("GitHub operation store unavailable, full, changed or corrupt; inspect and repair privately before proceeding"))
}

pub struct Service {
    client: Client,
    context: ToolContext,
    owner: Owner,
    directory: PathBuf,
}
impl Service {
    pub async fn logs(
        &self,
        object: super::repository::Object,
        job: u64,
    ) -> Result<super::logs::Log> {
        self.current()?;
        let mut log =
            super::logs::read(&self.client, object, job, &self.context.cancellation).await?;
        log.text = self.redact(&log.text);
        self.current()?;
        Ok(log)
    }
    pub fn new(mut context: ToolContext, session: Option<Uuid>) -> Result<Self> {
        context.policy.check_current()?;
        ensure!(
            context.policy.effective().rules().github_enabled,
            "GitHub capability is disabled by current local authority"
        );
        let token = context.github.as_ref().map(|credential|credential.expose().to_owned()).ok_or_else(|| anyhow::anyhow!("GitHub capability unavailable; enable github_enabled and provide HELM_GITHUB_TOKEN"))?;
        context.redactor = Arc::new(
            context
                .redactor
                .with_additional(super::credential_forms(&token)),
        );
        let reference = context
            .completion
            .as_ref()
            .map(|completion| completion.reference());
        ensure!(
            session.is_none_or(|session| reference
                .as_ref()
                .is_none_or(|reference| reference.session_id == session)),
            "GitHub voyage identity does not match its run"
        );
        let owner = Owner::new(
            context.policy.workspace(),
            reference
                .as_ref()
                .map(|reference| reference.session_id)
                .or(session),
            reference.as_ref().map(|reference| reference.run_id),
        )?;
        Ok(Self {
            client: Client::new(token)?.with_policy(context.policy.clone()),
            context,
            owner,
            directory: Store::default_path(),
        })
    }
    #[cfg(test)]
    pub(crate) fn fixture(
        context: ToolContext,
        session: Option<Uuid>,
        origin: reqwest::Url,
        directory: PathBuf,
    ) -> Result<Self> {
        let token = context
            .github
            .as_ref()
            .map(|credential| credential.expose().to_owned())
            .ok_or_else(|| anyhow::anyhow!("fixture needs dedicated synthetic token"))?;
        let policy = context.policy.clone();
        let mut service = Self::new(context, session)?;
        service.client = Client::fixture(token, origin)?.with_policy(policy);
        service.directory = directory;
        Ok(service)
    }
    pub fn owner(&self) -> &Owner {
        &self.owner
    }
    pub fn workspace(&self) -> PathBuf {
        self.context.policy.workspace().to_owned()
    }
    fn current(&self) -> Result<()> {
        self.context.policy.check_current()?;
        ensure!(
            self.context.policy.effective().rules().github_enabled,
            "GitHub capability is disabled by current local authority"
        );
        ensure!(
            !self.context.cancellation.is_cancelled(),
            "GitHub operation cancelled"
        );
        Ok(())
    }
    fn write_allowed(&self) -> Result<()> {
        self.current()?;
        ensure!(
            self.context.policy.access_mode() != crate::config::AccessMode::ReadOnly,
            "GitHub journal mutation is denied in read-only mode"
        );
        Ok(())
    }
    async fn database<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.current()?;
        let policy = self.context.policy.clone();
        let cancel = self.context.cancellation.clone();
        let result = database_at(self.directory.clone(), move |store| {
            policy.check_current()?;
            ensure!(!cancel.is_cancelled(), "GitHub operation cancelled");
            work(store)
        })
        .await?;
        self.current()?;
        Ok(result)
    }
    fn secret_free(&self, draft: &Draft) -> Result<()> {
        let token = self
            .context
            .github
            .as_ref()
            .map(|credential| credential.expose())
            .expect("validated credential");
        fn contains(value: &Value, token: &str, redactor: &crate::tools::Redactor) -> bool {
            match value {
                Value::String(value) => value.contains(token) || redactor.contains_secret(value),
                Value::Array(values) => values.iter().any(|value| contains(value, token, redactor)),
                Value::Object(values) => values
                    .values()
                    .any(|value| contains(value, token, redactor)),
                _ => false,
            }
        }
        ensure!(
            !contains(&serde_json::to_value(draft)?, token, &self.context.redactor),
            "GitHub publication contains a current configured secret"
        );
        Ok(())
    }
    pub async fn actor(&self) -> Result<Actor> {
        self.context.policy.check_current()?;
        let data = self
            .client
            .get("/user", &self.context.cancellation)
            .await?
            .json()?;
        let actor = Actor {
            id: data["id"]
                .as_u64()
                .filter(|id| *id > 0)
                .ok_or_else(|| anyhow::anyhow!("GitHub authenticated actor is unavailable"))?,
            login: data["login"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("GitHub authenticated actor is unavailable"))?
                .into(),
        };
        ensure!(
            !actor.login.is_empty()
                && actor.login.len() <= 100
                && actor
                    .login
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-[]".contains(&byte)),
            "GitHub returned invalid authenticated actor"
        );
        Ok(actor)
    }
    pub async fn read(&self, request: context::Read) -> Result<context::Page> {
        self.context.policy.check_current()?;
        context::read(&self.client, request, &self.context.cancellation).await
    }
    pub fn project(&self, value: &Value, maximum: usize) -> Result<String> {
        let text = serde_json::to_string_pretty(&self.redact_value(value))?;
        bounded_projection(text, maximum)
    }
    pub fn redact(&self, text: &str) -> String {
        self.context.redactor.redact(text).replace(
            self.context
                .github
                .as_ref()
                .map(|credential| credential.expose())
                .expect("validated credential"),
            "[REDACTED]",
        )
    }
    fn redact_value(&self, value: &Value) -> Value {
        super::redact_value(value, &self.context.redactor)
    }
    pub async fn prepare(&self, draft: Draft) -> Result<Operation> {
        self.write_allowed()?;
        draft.validate()?;
        self.secret_free(&draft)?;
        let actor = self.actor().await?;
        let (head, base) = self.validate_draft(&draft).await?;
        let policy = self.context.policy.effective().digest().to_owned();
        let owner = self.owner.clone();
        self.database(move |store| store.prepare(draft, actor, policy, head, base, owner))
            .await
    }
    pub async fn inspect(&self, id: Uuid) -> Result<Operation> {
        let owner = self.owner.clone();
        self.database(move |store| store.inspect(id, &owner)).await
    }
    pub async fn list(&self, offset: u32) -> Result<Vec<Operation>> {
        let owner = self.owner.clone();
        self.database(move |store| store.list(&owner, offset)).await
    }
    pub async fn cancel(&self, id: Uuid, digest: String) -> Result<Operation> {
        self.write_allowed()?;
        let owner = self.owner.clone();
        self.database(move |store| store.cancel(id, &digest, &owner))
            .await
    }
    pub async fn forget(&self, id: Uuid, digest: String) -> Result<()> {
        self.write_allowed()?;
        let owner = self.owner.clone();
        self.database(move |store| store.forget(id, &digest, &owner))
            .await
    }
    pub async fn reconcile(&self, id: Uuid, digest: &str, remote_id: u64) -> Result<Operation> {
        self.write_allowed()?;
        ensure!(
            self.context.interaction == InteractionMode::Attended,
            "GitHub uncertain receipt reconciliation requires an attended operator"
        );
        ensure!(
            remote_id > 0 && remote_id <= i64::MAX as u64,
            "invalid GitHub receipt ID"
        );
        let operation = self.inspect(id).await?;
        self.secret_free(&operation.draft)?;
        ensure!(
            operation.state == State::Sending && operation.digest == digest,
            "GitHub operation is not the exact uncertain record"
        );
        ensure!(
            operation.actor == self.actor().await?,
            "GitHub reconciliation account differs from the original actor"
        );
        let path = match &operation.draft.action {
            Action::Comment { .. } => format!(
                "/repos/{}/issues/comments/{remote_id}",
                operation.draft.object.repository.slug()
            ),
            Action::Review { .. } => format!(
                "/repos/{}/pulls/{}/reviews/{remote_id}",
                operation.draft.object.repository.slug(),
                operation.draft.object.number
            ),
        };
        let data = self
            .client
            .get(&path, &self.context.cancellation)
            .await?
            .json()?;
        let mut receipt = validate_receipt(&operation, &data)?;
        ensure!(
            receipt.id == remote_id,
            "GitHub receipt identity differs from the requested candidate"
        );
        ensure!(
            !super::value_has_secret(&serde_json::to_value(&receipt)?, &self.context.redactor),
            "GitHub exact receipt preview contains a configured secret"
        );
        let preview = format!(
            "{}\n\nCandidate: {}\nMatching actor and content alone do NOT prove this was the uncertain request. Explicitly adopt this candidate as an operator-confirmed receipt? No request will be resent.",
            exact_preview(&operation, &self.context.redactor)?,
            receipt.url
        );
        self.confirm("github.reconcile", &operation, preview)
            .await?;
        receipt.evidence = super::store::ReceiptEvidence::OperatorAdopted;
        let expected = operation.clone();
        self.database(move |store| store.publish(&expected, receipt))
            .await
    }
    pub async fn dispose(&self, id: Uuid, digest: &str, note: String) -> Result<Operation> {
        self.write_allowed()?;
        ensure!(
            !note.trim().is_empty() && note.len() <= 1024,
            "GitHub disposition requires a bounded note"
        );
        let operation = self.inspect(id).await?;
        self.secret_free(&operation.draft)?;
        ensure!(
            operation.state == State::Sending && operation.digest == digest,
            "GitHub operation is not the exact uncertain record"
        );
        ensure!(
            !self.context.redactor.contains_secret(&note),
            "GitHub exact disposition preview contains a configured secret"
        );
        let preview = format!(
            "{}\n\nClose this uncertain record with the following explicit operator disposition? This does not prove whether GitHub accepted it and never resends it.\n{}",
            exact_preview(&operation, &self.context.redactor)?,
            serde_json::to_string(&note)?
        );
        self.confirm("github.dispose", &operation, preview).await?;
        let owner = self.owner.clone();
        let digest = digest.to_owned();
        self.database(move |store| store.dispose(id, &digest, &owner, note))
            .await
    }
    async fn confirm(&self, action: &str, operation: &Operation, preview: String) -> Result<()> {
        self.write_allowed()?;
        ensure!(
            self.context.interaction == InteractionMode::Attended,
            "GitHub decision requires an attended operator"
        );
        ensure!(
            !self.context.redactor.contains_secret(&preview)
                && !preview.contains(
                    self.context
                        .github
                        .as_ref()
                        .map(|credential| credential.expose())
                        .expect("validated credential")
                ),
            "GitHub exact preview contains a configured secret"
        );
        let request = self
            .context
            .approval(action, operation.draft.object.url(), preview);
        let decision = tokio::select! {
            biased;
            _ = self.context.cancellation.cancelled() => anyhow::bail!("GitHub decision cancelled"),
            decision = tokio::time::timeout(Duration::from_secs(900), self.context.approver.approve(&request)) => decision.unwrap_or(ApprovalOutcome::Unavailable),
        };
        ensure!(
            decision == ApprovalOutcome::Approved,
            "GitHub decision was not approved"
        );
        self.current()?;
        Ok(())
    }
    async fn validate_draft(
        &self,
        draft: &Draft,
    ) -> Result<(Option<String>, Option<context::Base>)> {
        let detail =
            context::details(&self.client, &draft.object, &self.context.cancellation).await?;
        let head = if draft.object.kind == ObjectKind::PullRequest {
            Some(context::sha(&detail["head"]["sha"])?)
        } else {
            None
        };
        let base = if head.is_some() {
            Some(context::base(&detail)?)
        } else {
            None
        };
        if let Action::Review {
            commit_id,
            comments,
            ..
        } = &draft.action
        {
            ensure!(
                head.as_deref() == Some(commit_id),
                "pull request head changed; prepare a new exact review"
            );
            ensure!(
                detail["state"].as_str() == Some("open"),
                "pull request is no longer open for this review"
            );
            let mut remaining = comments.iter().collect::<Vec<_>>();
            let mut page = 1;
            while !remaining.is_empty() {
                let data = self
                    .client
                    .get(
                        &format!(
                            "/repos/{}/pulls/{}/files?per_page=100&page={page}",
                            draft.object.repository.slug(),
                            draft.object.number
                        ),
                        &self.context.cancellation,
                    )
                    .await?
                    .json()?;
                let files = data
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("GitHub returned malformed changed files"))?;
                ensure!(files.len() <= 100, "GitHub file page exceeds limit");
                for file in files {
                    let Some(path) = file["filename"].as_str() else {
                        anyhow::bail!("GitHub changed file path is missing")
                    };
                    for comment in remaining.iter().filter(|comment| comment.path == path) {
                        let patch = file["patch"].as_str().ok_or_else(|| {
                            anyhow::anyhow!(
                                "inline review patch is unavailable; do not guess coordinates"
                            )
                        })?;
                        super::publication::validate_inline(comment, patch)?;
                    }
                    remaining.retain(|comment| comment.path != path);
                }
                ensure!(
                    remaining.is_empty() || (!files.is_empty() && page < 30),
                    "inline review file is unavailable within GitHub's file limit"
                );
                page += 1;
            }
            let current =
                context::details(&self.client, &draft.object, &self.context.cancellation).await?;
            ensure!(
                context::sha(&current["head"]["sha"])? == *commit_id,
                "pull request changed during inline validation"
            );
            ensure!(
                Some(context::base(&current)?) == base,
                "pull request base changed during inline validation"
            );
        }
        Ok((head, base))
    }
    fn mutation_allowed(&self) -> Result<()> {
        self.context.policy.check_current()?;
        ensure!(
            self.context.policy.access_mode() != AccessMode::ReadOnly,
            "GitHub publication is denied in read-only mode"
        );
        ensure!(
            self.context.interaction == InteractionMode::Attended,
            "GitHub publication requires exact attended confirmation; unattended allow flags do not authorize it"
        );
        ensure!(
            !self.context.cancellation.is_cancelled(),
            "GitHub operation cancelled"
        );
        Ok(())
    }
    pub async fn publish(&self, id: Uuid, digest: &str) -> Result<Operation> {
        self.mutation_allowed()?;
        let operation = self.inspect(id).await?;
        self.secret_free(&operation.draft)?;
        ensure!(
            operation.digest == digest
                && operation.state == State::Prepared
                && operation.expires_at > chrono::Utc::now(),
            "GitHub preview changed, expired or already sent"
        );
        ensure!(
            operation.policy_digest == self.context.policy.effective().digest(),
            "local policy changed; prepare a new GitHub publication"
        );
        if self.owner.run.is_some() {
            ensure!(
                operation.owner.run == self.owner.run,
                "model publication belongs to another run; use exact operator review"
            );
        }
        ensure!(
            operation.actor == self.actor().await?,
            "GitHub authenticated account changed"
        );
        ensure!(
            (
                operation.observed_head.clone(),
                operation.observed_base.clone()
            ) == self.validate_draft(&operation.draft).await?,
            "GitHub object head changed since preparation"
        );
        let preview = exact_preview(&operation, &self.context.redactor)?;
        ensure!(
            !self.context.redactor.contains_secret(&preview)
                && !preview.contains(
                    self.context
                        .github
                        .as_ref()
                        .map(|credential| credential.expose())
                        .expect("validated credential")
                ),
            "GitHub exact preview contains a configured secret"
        );
        let request =
            self.context
                .approval("github.publish", operation.draft.object.url(), preview);
        let remaining = (operation.expires_at - chrono::Utc::now())
            .to_std()
            .unwrap_or_default();
        let decision = tokio::select! {
            biased;
            _ = self.context.cancellation.cancelled() => anyhow::bail!("GitHub publication cancelled before approval"),
            decision = tokio::time::timeout(remaining, self.context.approver.approve(&request)) => decision.unwrap_or(ApprovalOutcome::Unavailable),
        };
        ensure!(
            decision == ApprovalOutcome::Approved,
            "GitHub publication was not approved"
        );
        self.mutation_allowed()?;
        ensure!(
            operation.actor == self.actor().await?,
            "GitHub authenticated account changed after approval"
        );
        ensure!(
            (
                operation.observed_head.clone(),
                operation.observed_base.clone()
            ) == self.validate_draft(&operation.draft).await?,
            "GitHub object head changed after approval"
        );
        self.mutation_allowed()?;
        let expected = operation.clone();
        let sending = self
            .database(move |store| store.begin_send(&expected))
            .await?;
        // After this durable barrier every failure is uncertain, even if it occurs
        // before send. No later caller is allowed to replay this operation.
        self.mutation_allowed()?;
        let response = self
            .client
            .request(
                reqwest::Method::POST,
                &sending.draft.path(),
                Some(&sending.draft.body()),
                &self.context.cancellation,
            )
            .await?;
        super::transport::check_status(&response)?;
        let expected_status = match sending.draft.action {
            Action::Comment { .. } => 201,
            Action::Review { .. } => 200,
        };
        ensure!(
            response.status.as_u16() == expected_status,
            "GitHub publication response is uncertain; inspect the operation"
        );
        let response = response.json()?;
        let receipt = validate_receipt(&sending, &response)?;
        let expected = sending.clone();
        let receipt_url = receipt.url.clone();
        match self
            .database(move |store| store.publish(&expected, receipt))
            .await
        {
            Ok(operation) => Ok(operation),
            Err(_) => anyhow::bail!(
                "GitHub accepted operation {} at {}; local receipt confirmation failed. Do not resend; reconcile this exact returned identity.",
                sending.id,
                receipt_url
            ),
        }
    }
}

fn bounded_projection(text: String, maximum: usize) -> Result<String> {
    if text.len() <= maximum {
        return Ok(text);
    }
    let mut end = maximum.min(text.len());
    loop {
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let encoded = serde_json::to_string(
            &serde_json::json!({"incomplete":true,"reason":"local output byte limit; use operator CLI or narrower context","excerpt":&text[..end]}),
        )?;
        if encoded.len() <= maximum {
            return Ok(encoded);
        }
        ensure!(
            end > 0,
            "GitHub output budget is too small for an honest incomplete result"
        );
        end = end.saturating_sub(encoded.len().saturating_sub(maximum).max(1));
    }
}

fn exact_preview(operation: &Operation, redactor: &crate::tools::Redactor) -> Result<String> {
    ensure!(
        !super::value_has_secret(&serde_json::to_value(operation)?, redactor),
        "GitHub exact preview contains a configured secret"
    );
    json_preview(
        &serde_json::json!({"host":"github.com","actor":operation.actor,"object":operation.draft.object.url(),"head":operation.observed_head,"base":operation.observed_base,"request":operation.draft.body(),"digest":operation.digest,"expires_at":operation.expires_at,"notice":"Publishing sends this exact content to GitHub and may notify repository participants. Client revalidation cannot provide a server-side atomic head/base precondition."}),
    )
}

pub(super) fn json_preview(value: &Value) -> Result<String> {
    let text = serde_json::to_string_pretty(value)?;
    // JSON quotes preserve newlines/controls. Escape invisible directional and
    // format characters which otherwise change the apparent approved content.
    Ok(text.chars().flat_map(|ch| {
        if matches!(ch, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}') { format!("\\u{:04x}", ch as u32).chars().collect::<Vec<_>>() } else { vec![ch] }
    }).collect())
}

fn validate_receipt(operation: &Operation, value: &Value) -> Result<Receipt> {
    let id = value["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| anyhow::anyhow!("GitHub publication has no trustworthy receipt identity"))?;
    ensure!(
        value["user"]["id"].as_u64() == Some(operation.actor.id),
        "GitHub publication receipt actor differs"
    );
    let marker = match &operation.draft.action {
        Action::Comment { body } => {
            ensure!(
                value["body"].as_str() == Some(body),
                "GitHub comment receipt differs from approved content"
            );
            "issuecomment"
        }
        Action::Review {
            body,
            commit_id,
            event,
            ..
        } => {
            ensure!(
                value["body"].as_str() == Some(body)
                    && value["commit_id"].as_str() == Some(commit_id),
                "GitHub review receipt differs from approved content"
            );
            let state = match event {
                super::publication::ReviewEvent::Comment => "COMMENTED",
                super::publication::ReviewEvent::Approve => "APPROVED",
                super::publication::ReviewEvent::RequestChanges => "CHANGES_REQUESTED",
            };
            ensure!(
                value["state"].as_str() == Some(state),
                "GitHub review receipt has a different event"
            );
            "pullrequestreview"
        }
    };
    let url = format!("{}#{marker}-{id}", operation.draft.object.url());
    ensure!(
        value["html_url"]
            .as_str()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(&url)),
        "GitHub receipt URL differs from the approved object"
    );
    Ok(Receipt {
        id,
        url,
        evidence: super::store::ReceiptEvidence::ApiResponse,
    })
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    #[test]
    fn final_encoded_projection_never_exceeds_budget() {
        for input in [
            "\\\"".repeat(2048),
            "\0\n\t".repeat(2048),
            "🧭日本語".repeat(2048),
        ] {
            for maximum in [0, 1, 64, 128, 256, 511, 1024] {
                let result = bounded_projection(input.clone(), maximum);
                if let Ok(result) = result {
                    assert!(result.len() <= maximum);
                    let value: Value = serde_json::from_str(&result).unwrap();
                    assert_eq!(value["incomplete"], true);
                } else {
                    assert!(maximum < 256);
                }
            }
        }
    }
}
