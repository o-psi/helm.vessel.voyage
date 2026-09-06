//! Equal jitter is bounded by the exponential ceiling. Explicit server delays
//! bypass jitter and are never shortened to fit the local retry wait budget.
use std::time::Duration;

pub trait RetryJitter: Send + Sync {
    fn sample(&self) -> u32;
}
pub(super) struct RandomJitter;
impl RetryJitter for RandomJitter {
    fn sample(&self) -> u32 {
        uuid::Uuid::new_v4().as_fields().0
    }
}
pub(super) fn jittered(ceiling: Duration, sample: u32) -> Duration {
    let upper = ceiling.as_nanos();
    let lower = upper / 2;
    // Duration::MAX nanoseconds times u32::MAX fits in u128.
    let nanos = lower + (upper - lower) * u128::from(sample) / u128::from(u32::MAX);
    Duration::new(
        (nanos / 1_000_000_000) as u64,
        (nanos % 1_000_000_000) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn injected_endpoints_and_extreme_duration_are_bounded() {
        for ceiling in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_millis(500),
            Duration::MAX,
        ] {
            assert_eq!(jittered(ceiling, 0).as_nanos(), ceiling.as_nanos() / 2);
            assert_eq!(jittered(ceiling, u32::MAX), ceiling);
            assert!(jittered(ceiling, u32::MAX / 2) <= ceiling);
        }
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    use crate::{
        agent::{AgentError, AgentEvent, EventSink, RetryPolicy},
        model::{Message, ModelRequest, ModelResponse, Role, Usage},
        provider::{Provider, ProviderError},
    };
    use async_trait::async_trait;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio_util::sync::CancellationToken;
    struct Samples {
        values: Vec<u32>,
        calls: AtomicUsize,
    }
    impl RetryJitter for Samples {
        fn sample(&self) -> u32 {
            self.values[self.calls.fetch_add(1, Ordering::SeqCst)]
        }
    }
    struct Flaky {
        calls: Arc<AtomicUsize>,
        failures: usize,
        after: Option<Duration>,
    }
    #[async_trait]
    impl Provider for Flaky {
        async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) < self.failures {
                Err(ProviderError::RateLimit {
                    message: "offline throttle".into(),
                    retry_after: self.after,
                })
            } else {
                Ok(ModelResponse {
                    message: Message::new(Role::Assistant, "done"),
                    usage: Usage {
                        input_tokens: 7,
                        output_tokens: 3,
                    },
                })
            }
        }
    }
    struct Events {
        waits: Mutex<Vec<Duration>>,
        cancel: Option<CancellationToken>,
    }
    #[async_trait]
    impl EventSink for Events {
        async fn emit(&self, event: AgentEvent) {
            if let AgentEvent::ProviderRetry { delay, .. } = event {
                self.waits.lock().unwrap().push(delay);
                if let Some(cancel) = &self.cancel {
                    cancel.cancel();
                }
            }
        }
    }
    #[tokio::test(start_paused = true)]
    async fn injected_jitter_obeys_exponential_ceiling_and_records_known_usage() {
        let directory = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let samples = Arc::new(Samples {
            values: vec![0, u32::MAX, u32::MAX / 2],
            calls: AtomicUsize::new(0),
        });
        let events = Arc::new(Events {
            waits: Mutex::new(Vec::new()),
            cancel: None,
        });
        let mut agent = crate::agent::tests::agent(
            Box::new(Flaky {
                calls: calls.clone(),
                failures: 3,
                after: None,
            }),
            &directory,
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 4,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(250),
        })
        .with_retry_jitter(samples.clone());
        agent.sink = events.clone();
        let result = agent.run(vec![], "retry".into()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert_eq!(
            *events.waits.lock().unwrap(),
            [
                Duration::from_millis(50),
                Duration::from_millis(200),
                jittered(Duration::from_millis(250), u32::MAX / 2)
            ]
        );
        assert_eq!(samples.calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            (result.usage.input_tokens, result.usage.output_tokens),
            (7, 3)
        );
    }
    #[tokio::test(start_paused = true)]
    async fn explicit_server_delay_is_not_jittered_or_shortened_and_cancellation_stops_retry() {
        for (delay, cancel_retry) in [(100, false), (300, false), (100, true)] {
            let directory = tempfile::tempdir().unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let cancel = CancellationToken::new();
            let samples = Arc::new(Samples {
                values: vec![],
                calls: AtomicUsize::new(0),
            });
            let events = Arc::new(Events {
                waits: Mutex::new(Vec::new()),
                cancel: cancel_retry.then(|| cancel.clone()),
            });
            let mut agent = crate::agent::tests::agent(
                Box::new(Flaky {
                    calls: calls.clone(),
                    failures: 1,
                    after: Some(Duration::from_millis(delay)),
                }),
                &directory,
            )
            .with_retry_policy(RetryPolicy {
                max_attempts: 3,
                initial_delay: Duration::from_millis(50),
                max_delay: Duration::from_millis(200),
            })
            .with_retry_jitter(samples.clone());
            agent.sink = events.clone();
            let result = agent.run_with_cancel(vec![], "retry".into(), cancel).await;
            if delay > 200 {
                assert!(matches!(
                    result,
                    Err(AgentError::Provider(ProviderError::RateLimit { .. }))
                ));
                assert!(events.waits.lock().unwrap().is_empty());
                assert_eq!(calls.load(Ordering::SeqCst), 1);
            } else {
                assert_eq!(
                    *events.waits.lock().unwrap(),
                    [Duration::from_millis(delay)]
                );
                if cancel_retry {
                    assert!(matches!(result, Err(AgentError::Cancelled)));
                } else {
                    assert!(result.is_ok());
                }
                assert_eq!(
                    calls.load(Ordering::SeqCst),
                    if cancel_retry { 1 } else { 2 }
                );
            }
            assert_eq!(samples.calls.load(Ordering::SeqCst), 0);
        }
    }
}
