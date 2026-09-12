//! Provider attempts are not tool outcomes. No partial response is replayed.
use super::*;
use crate::provider::{ProviderDelta, ProviderStreamEvent};
use futures_util::StreamExt;
use voyage_protocol::provider_attempt::{AttemptPhase, ProviderAttempt, RetryDecision};

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

impl Agent {
    fn retry_decision(
        &self,
        error: &ProviderError,
        record: &ProviderAttempt,
        elapsed: Duration,
        delay: Duration,
    ) -> (RetryDecision, Option<Duration>) {
        if record.text_observed || record.tool_fragment_observed {
            return (RetryDecision::PartialResponse, None);
        }
        if !error.is_retryable() {
            return (RetryDecision::NonRetryable, None);
        }
        if record.attempt >= record.limit {
            return (RetryDecision::AttemptsExhausted, None);
        }
        if error
            .retry_after()
            .is_some_and(|wait| wait > self.retry.max_delay)
        {
            return (RetryDecision::ServerDelayLimit, None);
        }
        let wait = error.retry_after().unwrap_or_else(|| {
            retry::jittered(delay.min(self.retry.max_delay), self.retry_jitter.sample())
        });
        if elapsed.saturating_add(wait) >= self.retry.max_elapsed {
            return (RetryDecision::ElapsedBudget, None);
        }
        (RetryDecision::RetryScheduled, Some(wait))
    }

    async fn record_provider_attempt(
        &self,
        checkpoint: Option<&dyn RunCheckpoint>,
        record: &ProviderAttempt,
    ) -> Result<(), AgentError> {
        if let Some(checkpoint) = checkpoint {
            tokio::time::timeout(self.context.timeout, checkpoint.provider_attempt(record))
                .await
                .map_err(|_| CheckpointError)??;
        }
        Ok(())
    }

    pub(super) async fn provider_request_attempts(
        &self,
        request: ModelRequest,
        cancel: &CancellationToken,
        checkpoint: Option<&dyn RunCheckpoint>,
        partial_output: &mut String,
        reference: Option<&crate::completion::runtime::RunReference>,
    ) -> Result<
        (
            crate::model::ModelResponse,
            Option<crate::inference::Permit>,
        ),
        AgentError,
    > {
        let request_id = uuid::Uuid::new_v4();
        let window = tokio::time::Instant::now();
        let mut delay = self.retry.initial_delay;
        for attempt in 1..=self.retry.max_attempts.max(1) {
            if cancel.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            self.check_current_policy()?;
            self.context
                .policy
                .check_execution_authority()
                .map_err(|_| {
                    AgentError::Policy("foreground execution authority unavailable".into())
                })?;
            let permit = self
                .inference_admit(
                    reference,
                    &request.model,
                    crate::inference::Purpose::Conversation,
                )
                .await?;
            let started = tokio::time::Instant::now();
            let mut record = ProviderAttempt {
                retry: voyage_protocol::provider_attempt::RetryObservation {
                    response_timeout_ms: Some(millis(self.retry.response_timeout)),
                    stream_idle_ms: Some(millis(self.retry.stream_idle)),
                    max_delay_ms: Some(millis(self.retry.max_delay)),
                    max_elapsed_ms: Some(millis(self.retry.max_elapsed)),
                    ..Default::default()
                },
                request_id,
                attempt_id: permit
                    .as_ref()
                    .map(|p| p.id)
                    .unwrap_or_else(uuid::Uuid::new_v4),
                provider: self
                    .inference_provider
                    .as_ref()
                    .map(|p| p.profile().id)
                    .unwrap_or("custom")
                    .into(),
                model: self
                    .context
                    .redactor
                    .redact(&request.model)
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(256)
                    .collect(),
                attempt,
                limit: self.retry.max_attempts.max(1),
                started_at_ms: millis(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default(),
                ),
                duration_ms: 0,
                phase: AttemptPhase::Dispatch,
                category: None,
                http_status: None,
                text_observed: false,
                tool_fragment_observed: false,
                retry_delay_ms: None,
                decision: RetryDecision::InFlight,
            };
            // Durable intent precedes the external request; failure here never dispatches.
            self.record_provider_attempt(checkpoint, &record).await?;
            if attempt > 1 && window.elapsed() >= self.retry.max_elapsed {
                record.decision = RetryDecision::ElapsedBudget;
                record.retry.elapsed_ms = Some(millis(window.elapsed()));
                self.record_provider_attempt(checkpoint, &record).await?;
                self.inference_finish(permit.as_ref(), crate::inference::AttemptOutcome::Failed)
                    .await?;
                return Err(ProviderError::Timeout(
                    "retry admission window elapsed before dispatch".into(),
                )
                .into());
            }
            let outcome = self
                .provider_attempt_stream(
                    &request,
                    permit.as_ref(),
                    cancel,
                    checkpoint,
                    partial_output,
                    &mut record,
                )
                .await;
            record.duration_ms = millis(started.elapsed());
            match outcome {
                Ok(response) => {
                    record.decision = RetryDecision::Completed;
                    self.record_provider_attempt(checkpoint, &record).await?;
                    return Ok((response, permit));
                }
                Err(AgentError::Provider(error)) => {
                    record.category = Some(error.category().into());
                    record.http_status = error.http_status().or(record.http_status);
                    if let Some(id) = error
                        .upstream_request_id()
                        .filter(|id| !self.context.redactor.contains_secret(id))
                    {
                        record.retry.upstream_request_id = Some(id.to_owned());
                    }
                    record.retry.server_delay_ms = error.retry_after().map(millis);
                    record.retry.elapsed_ms = Some(millis(window.elapsed()));
                    record.retry.failure_phase = Some(record.phase.clone());
                    record.retry.eligible = error.is_retryable()
                        && !record.text_observed
                        && !record.tool_fragment_observed;
                    record.retry.provider_code = error.safe_code().map(str::to_owned);
                    let (decision, wait) =
                        self.retry_decision(&error, &record, window.elapsed(), delay);
                    record.decision = decision;
                    record.retry_delay_ms = wait.map(millis);
                    self.record_provider_attempt(checkpoint, &record).await?;
                    // Failed provider outcome does not mean unbilled, no usage, or a tool failure.
                    if let Err(error) = self
                        .inference_finish(permit.as_ref(), crate::inference::AttemptOutcome::Failed)
                        .await
                    {
                        record.decision = RetryDecision::LocalFailure;
                        self.record_provider_attempt(checkpoint, &record).await?;
                        return Err(error);
                    }
                    let Some(wait) = wait else {
                        // An explicit context rejection after any delta is interruption,
                        // not permission for the outer compaction loop to replay the request.
                        return Err(
                            if (record.text_observed || record.tool_fragment_observed)
                                && error.is_context_length()
                            {
                                ProviderError::Incomplete.into()
                            } else {
                                error.into()
                            },
                        );
                    };
                    self.sink
                        .emit(AgentEvent::ProviderRetry {
                            attempt,
                            delay: wait,
                            error: error.public_failure_reason().into(),
                        })
                        .await;
                    let backoff_start = tokio::time::Instant::now();
                    let stop = self.retry_wait(wait, cancel).await;
                    record.retry.backoff_ms = Some(millis(backoff_start.elapsed()));
                    record.retry.elapsed_ms = Some(millis(window.elapsed()));
                    if let Err(stop) = stop {
                        record.phase = AttemptPhase::Backoff;
                        record.decision = match &stop {
                            AgentError::Cancelled => RetryDecision::Cancelled,
                            _ => RetryDecision::PolicyRevoked,
                        };
                        self.record_provider_attempt(checkpoint, &record).await?;
                        return Err(stop);
                    }
                    // Scheduler delay must not cause a late retry beyond the admitted window.
                    if window.elapsed() >= self.retry.max_elapsed {
                        record.phase = AttemptPhase::Backoff;
                        record.decision = RetryDecision::ElapsedBudget;
                        self.record_provider_attempt(checkpoint, &record).await?;
                        return Err(error.into());
                    }
                    self.record_provider_attempt(checkpoint, &record).await?;
                    delay = delay.saturating_mul(2).min(self.retry.max_delay);
                }
                Err(error) => {
                    record.decision = match &error {
                        AgentError::Cancelled => RetryDecision::Cancelled,
                        AgentError::Policy(_) => RetryDecision::PolicyRevoked,
                        _ => RetryDecision::LocalFailure,
                    };
                    self.record_provider_attempt(checkpoint, &record).await?;
                    return Err(error);
                }
            }
        }
        unreachable!("bounded attempt loop returns on its final attempt")
    }

    async fn provider_attempt_stream(
        &self,
        request: &ModelRequest,
        permit: Option<&crate::inference::Permit>,
        cancel: &CancellationToken,
        checkpoint: Option<&dyn RunCheckpoint>,
        partial_output: &mut String,
        record: &mut ProviderAttempt,
    ) -> Result<crate::model::ModelResponse, AgentError> {
        self.check_current_policy()?;
        self.context
            .policy
            .check_execution_authority()
            .map_err(|_| AgentError::Policy("foreground execution authority unavailable".into()))?;
        let mut stream = self
            .provider_wait(
                self.provider.stream(request.clone()),
                self.retry.response_timeout,
                cancel,
            )
            .await??;
        record.phase = AttemptPhase::Stream;
        self.record_provider_attempt(checkpoint, record).await?;
        let mut pending_text = String::new();
        loop {
            let event = self
                .provider_wait(stream.next(), self.retry.stream_idle, cancel)
                .await?;
            match event {
                Some(Ok(ProviderStreamEvent::ResponseMetadata { status, request_id })) => {
                    record.http_status = Some(status);
                    record.retry.upstream_request_id =
                        request_id.filter(|id| !self.context.redactor.contains_secret(id));
                    self.record_provider_attempt(checkpoint, record).await?;
                }
                Some(Ok(ProviderStreamEvent::UsageReported(report))) => {
                    self.inference_report(permit, report).await?
                }
                Some(Ok(ProviderStreamEvent::Delta(delta))) => {
                    match &delta {
                        ProviderDelta::Text(text) if text.is_empty() => continue,
                        ProviderDelta::Text(_) if !record.text_observed => {
                            record.text_observed = true;
                            self.record_provider_attempt(checkpoint, record).await?;
                        }
                        ProviderDelta::ToolCall { .. } if !record.tool_fragment_observed => {
                            record.tool_fragment_observed = true;
                            self.record_provider_attempt(checkpoint, record).await?;
                        }
                        _ => {}
                    }
                    if let ProviderDelta::Text(text) = delta {
                        pending_text.push_str(&text);
                        let end = self.context.redactor.stable_prefix(&pending_text, false);
                        let safe = self
                            .context
                            .redactor
                            .redact_public_prefix(&pending_text[..end]);
                        pending_text.drain(..end);
                        self.project_provider_text(safe, checkpoint, partial_output)
                            .await?;
                    }
                }
                Some(Ok(ProviderStreamEvent::Completed(mut response))) => {
                    if let Some((_, resolution)) = self.resolved_inference.lock().await.as_mut() {
                        resolution.service.provider_reported = response
                            .service_tier
                            .as_ref()
                            .filter(|tier| !self.context.redactor.contains_secret(tier))
                            .cloned();
                    }
                    crate::provider::redact_message(&mut response.message, &self.context.redactor)?;
                    // Only a completed response flushes the private withheld suffix.
                    let safe = self.context.redactor.redact_public_prefix(&pending_text);
                    self.project_provider_text(safe, checkpoint, partial_output)
                        .await?;
                    return Ok(*response);
                }
                Some(Err(error)) => return Err(error.into()),
                None => {
                    return Err(ProviderError::InvalidResponse(
                        "provider stream ended without completion".into(),
                    )
                    .into());
                }
            }
        }
    }

    /// Poll one response/event with a real deadline and continuous authority checks.
    /// Only decoded provider events reset idle time; raw bytes and SSE comments do not.
    async fn provider_wait<F: std::future::Future>(
        &self,
        future: F,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<F::Output, AgentError> {
        let work = tokio::time::timeout(timeout, future);
        tokio::pin!(work);
        loop {
            self.check_current_policy()?;
            self.context
                .policy
                .check_execution_authority()
                .map_err(|_| {
                    AgentError::Policy("foreground execution authority unavailable".into())
                })?;
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                value = &mut work => return value.map_err(|_| ProviderError::Timeout("provider response/event deadline elapsed".into()).into()),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {},
            }
        }
    }

    async fn retry_wait(
        &self,
        wait: Duration,
        cancel: &CancellationToken,
    ) -> Result<(), AgentError> {
        let sleep = tokio::time::sleep(wait);
        tokio::pin!(sleep);
        loop {
            self.check_current_policy()?;
            self.context
                .policy
                .check_execution_authority()
                .map_err(|_| {
                    AgentError::Policy("foreground execution authority unavailable".into())
                })?;
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                _ = &mut sleep => return Ok(()),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {},
            }
        }
    }
}
