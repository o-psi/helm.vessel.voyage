use super::*;
use crate::model::{Message, Role, Usage};

#[derive(Debug, Default)]
struct Observations {
    values: Mutex<Vec<RequestObservation>>,
    fail_revision: Option<u64>,
}
#[async_trait]
impl Observer for Observations {
    async fn record(&self, observation: RequestObservation) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.fail_revision != Some(observation.revision),
            "fixture storage failure"
        );
        self.values.lock().unwrap().push(observation);
        Ok(())
    }
}

fn request() -> ModelRequest {
    ModelRequest {
        model: "offline".into(),
        messages: Vec::new(),
        tools: Vec::new(),
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: None,
    }
}
fn response(input_tokens: u64, output_tokens: u64) -> ModelResponse {
    ModelResponse {
        service_tier: None,
        message: Message::new(Role::Assistant, "fixture"),
        usage: Usage {
            input_tokens,
            output_tokens,
        },
    }
}
fn meter() -> Arc<GoalMeter> {
    GoalMeter::new(100, Duration::from_secs(60))
}
fn report(input: Option<u64>, output: Option<u64>) -> ProviderStreamEvent {
    ProviderStreamEvent::UsageReported(ReportedUsage {
        input_tokens: input,
        output_tokens: output,
    })
}
struct Script {
    events: Mutex<Option<Vec<Result<ProviderStreamEvent, ProviderError>>>>,
    requests: Arc<Mutex<Vec<ModelRequest>>>,
}
#[async_trait]
impl Provider for Script {
    async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
        panic!("meter must observe explicit stream metadata")
    }
    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(Box::pin(futures_util::stream::iter(
            self.events.lock().unwrap().take().unwrap(),
        )))
    }
}
fn provider(
    meter: Arc<GoalMeter>,
    events: Vec<Result<ProviderStreamEvent, ProviderError>>,
) -> (MeteredProvider, Arc<Mutex<Vec<ModelRequest>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    (
        MeteredProvider {
            inner: Box::new(Script {
                events: Mutex::new(Some(events)),
                requests: requests.clone(),
            }),
            meter,
        },
        requests,
    )
}

#[tokio::test]
async fn cumulative_reports_and_local_children_share_one_total() {
    let m = meter();
    let (parent, requests) = provider(
        m.clone(),
        vec![
            Ok(report(Some(10), Some(0))),
            Ok(report(None, Some(5))),
            Ok(report(None, Some(5))),
            Ok(ProviderStreamEvent::Completed(Box::new(response(10, 5)))),
        ],
    );
    let (child, _) = provider(
        m.clone(),
        vec![
            Ok(report(Some(20), Some(3))),
            Ok(ProviderStreamEvent::Completed(Box::new(response(20, 3)))),
        ],
    );
    let (a, b) = tokio::join!(parent.complete(request()), child.complete(request()));
    a.unwrap();
    b.unwrap();
    let actual = m.measurement();
    assert_eq!(
        (actual.input_tokens, actual.output_tokens, actual.complete),
        (30, 8, true)
    );
    assert!(requests.lock().unwrap()[0].max_tokens.unwrap() <= 100);
}

#[tokio::test]
async fn zero_report_is_measured_but_missing_usage_is_unknown() {
    for explicit in [true, false] {
        let m = meter();
        let mut events = Vec::new();
        if explicit {
            events.push(Ok(report(Some(0), Some(0))));
        }
        events.push(Ok(ProviderStreamEvent::Completed(Box::new(response(0, 0)))));
        let (p, _) = provider(m.clone(), events);
        p.complete(request()).await.unwrap();
        assert_eq!(m.measurement().complete, explicit);
        assert_eq!(m.begin(&mut request()).is_ok(), explicit);
    }
}

#[tokio::test]
async fn cancellation_and_incomplete_stream_retain_known_lower_bound_and_prevent_retry() {
    for cancel in [true, false] {
        let m = meter();
        let (p, requests) = provider(m.clone(), vec![Ok(report(Some(7), Some(2)))]);
        let mut stream = p.stream(request()).await.unwrap();
        stream.next().await.unwrap().unwrap();
        if !cancel {
            assert!(stream.next().await.unwrap().is_err());
        }
        drop(stream);
        let actual = m.measurement();
        assert_eq!(
            (actual.input_tokens, actual.output_tokens, actual.complete),
            (7, 2, false)
        );
        assert!(p.stream(request()).await.is_err());
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn dropping_unpolled_dispatched_stream_is_uncertain() {
    let m = meter();
    let (p, requests) = provider(m.clone(), vec![]);
    let stream = p.stream(request()).await.unwrap();
    assert!(!m.measurement().complete);
    drop(stream);
    assert!(!m.measurement().complete);
    assert!(p.stream(request()).await.is_err());
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn missing_and_conflicting_reports_preserve_known_tokens() {
    for events in [
        vec![Ok(ProviderStreamEvent::Completed(Box::new(response(8, 3))))],
        vec![
            Ok(report(Some(8), Some(1))),
            Ok(ProviderStreamEvent::Completed(Box::new(response(8, 3)))),
        ],
        vec![Ok(report(Some(8), Some(3))), Ok(report(Some(4), Some(1)))],
    ] {
        let m = meter();
        let (p, _) = provider(m.clone(), events);
        let _ = p.complete(request()).await;
        let actual = m.measurement();
        assert_eq!(
            (actual.input_tokens, actual.output_tokens, actual.complete),
            (8, 3, false)
        );
        assert!(p.stream(request()).await.is_err());
    }
}

#[tokio::test]
async fn token_and_time_limits_refuse_before_dispatch() {
    let m = GoalMeter::new(10, Duration::from_secs(60));
    let (p, requests) = provider(
        m.clone(),
        vec![
            Ok(report(Some(7), Some(3))),
            Ok(ProviderStreamEvent::Completed(Box::new(response(7, 3)))),
        ],
    );
    p.complete(request()).await.unwrap();
    assert_eq!(requests.lock().unwrap()[0].max_tokens, Some(10));
    let error = p.stream(request()).await.err().unwrap();
    assert_eq!(error.safe_code(), Some("goal_budget_stopped"));
    assert!(!error.is_retryable());
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(m.measurement().complete);
    let (expired, requests) = provider(GoalMeter::new(100, Duration::ZERO), vec![]);
    assert!(expired.stream(request()).await.is_err());
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn configuration_cannot_serialize_or_forge_continuation_budget() {
    let mut config = Config {
        goal_meter: Some(meter()),
        ..Config::default()
    };
    let serialized = serde_json::to_value(&config).unwrap();
    assert!(serialized.get("goal_meter").is_none());
    let mut forged = serialized;
    forged["goal_meter"] = serde_json::json!({"token_allowance":999999999});
    let decoded: Config = serde_json::from_value(forged).unwrap();
    assert!(decoded.goal_meter.is_none());
    let child = config.clone();
    assert!(Arc::ptr_eq(
        child.goal_meter.as_ref().unwrap(),
        config.goal_meter.as_ref().unwrap()
    ));
    config.apply_override("temperature", "0.5").unwrap();
    assert!(Arc::ptr_eq(
        child.goal_meter.as_ref().unwrap(),
        config.goal_meter.as_ref().unwrap()
    ));
}

#[test]
fn counter_overflow_cannot_wrap_to_a_fresh_budget() {
    let m = GoalMeter::new(u64::MAX, Duration::from_secs(60));
    let mut a = m.begin(&mut request()).unwrap();
    let mut b = m.begin(&mut request()).unwrap();
    a.observe(ReportedUsage {
        input_tokens: Some(u64::MAX - 1),
        output_tokens: Some(0),
    })
    .unwrap();
    assert!(
        b.observe(ReportedUsage {
            input_tokens: Some(2),
            output_tokens: Some(0)
        })
        .is_err()
    );
    assert_eq!(m.measurement().input_tokens, u64::MAX - 1);
    assert!(!m.measurement().complete);
    assert!(m.begin(&mut request()).is_err());
}

#[tokio::test]
async fn provider_failure_fences_retries_before_the_failed_stream_is_dropped() {
    for events in [vec![Err(ProviderError::Connection)], vec![]] {
        let m = meter();
        let (p, requests) = provider(m.clone(), events);
        let mut stream = p.stream(request()).await.unwrap();
        assert!(stream.next().await.unwrap().is_err());
        assert!(p.stream(request()).await.is_err());
        assert!(!m.measurement().complete);
        assert_eq!(requests.lock().unwrap().len(), 1);
        drop(stream);
    }
}

#[tokio::test]
async fn durable_checkpoint_failure_stops_dispatch_or_delivery_without_retry() {
    for failed in 0..=2 {
        let observations = Arc::new(Observations {
            fail_revision: Some(failed),
            ..Default::default()
        });
        let m = GoalMeter::with_observer(100, Duration::from_secs(60), Some(observations.clone()));
        let (p, requests) = provider(
            m.clone(),
            vec![
                Ok(report(Some(8), Some(3))),
                Ok(ProviderStreamEvent::Completed(Box::new(response(8, 3)))),
            ],
        );
        let error = p.complete(request()).await.unwrap_err();
        assert_eq!(error.safe_code(), Some("goal_usage_checkpoint_failed"));
        assert_eq!(requests.lock().unwrap().len(), usize::from(failed != 0));
        assert_eq!(observations.values.lock().unwrap().len(), failed as usize);
        assert!(!m.measurement().complete);
        assert!(p.stream(request()).await.is_err());
        assert_eq!(requests.lock().unwrap().len(), usize::from(failed != 0));
    }
}

#[tokio::test]
async fn durable_observations_bracket_dispatch_usage_and_completion() {
    let observations = Arc::new(Observations::default());
    let m = GoalMeter::with_observer(100, Duration::from_secs(60), Some(observations.clone()));
    let (p, _) = provider(
        m.clone(),
        vec![
            Ok(report(Some(8), Some(3))),
            Ok(ProviderStreamEvent::Completed(Box::new(response(8, 3)))),
        ],
    );
    let mut stream = p.stream(request()).await.unwrap();
    assert_eq!(observations.values.lock().unwrap().len(), 1);
    stream.next().await.unwrap().unwrap();
    assert_eq!(observations.values.lock().unwrap().len(), 2);
    stream.next().await.unwrap().unwrap();
    drop(stream);
    let values = observations.values.lock().unwrap();
    assert_eq!(values.len(), 3);
    assert_eq!(
        (
            values[0].revision,
            values[0].input_tokens,
            values[0].complete
        ),
        (0, None, false)
    );
    assert_eq!(
        (
            values[1].revision,
            values[1].input_tokens,
            values[1].output_tokens,
            values[1].complete
        ),
        (1, Some(8), Some(3), false)
    );
    assert_eq!(
        values[2],
        RequestObservation {
            revision: 2,
            complete: true,
            ..values[1].clone()
        }
    );
    assert!(m.measurement().complete);
}

#[tokio::test]
async fn cancellation_retains_an_open_durable_request_with_known_usage() {
    let observations = Arc::new(Observations::default());
    let m = GoalMeter::with_observer(100, Duration::from_secs(60), Some(observations.clone()));
    let (p, _) = provider(m.clone(), vec![Ok(report(Some(8), Some(3)))]);
    let mut stream = p.stream(request()).await.unwrap();
    stream.next().await.unwrap().unwrap();
    drop(stream);
    assert!(!m.measurement().complete);
    let values = observations.values.lock().unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[1].input_tokens, Some(8));
    assert!(!values[1].complete);
}

#[tokio::test]
async fn exhausted_goal_blocks_tool_policy_without_weakening_original_authority() {
    #[derive(Debug)]
    struct Upstream(std::sync::atomic::AtomicBool);
    impl crate::policy::ExecutionAuthority for Upstream {
        fn check(&self) -> anyhow::Result<()> {
            anyhow::ensure!(
                !self.0.load(std::sync::atomic::Ordering::SeqCst),
                "upstream revoked"
            );
            Ok(())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let m = GoalMeter::new(10, Duration::from_secs(60));
    let upstream = Arc::new(Upstream(std::sync::atomic::AtomicBool::new(false)));
    let policy = crate::policy::Policy::new(&Config::default(), root.path().to_path_buf())
        .unwrap()
        .with_execution_authority(m.execution_authority(Some(upstream.clone())));
    policy.check_execution_authority().unwrap();
    upstream.0.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        policy
            .check_execution_authority()
            .unwrap_err()
            .to_string()
            .contains("upstream revoked")
    );
    upstream.0.store(false, std::sync::atomic::Ordering::SeqCst);
    let (p, _) = provider(
        m.clone(),
        vec![
            Ok(report(Some(7), Some(3))),
            Ok(ProviderStreamEvent::Completed(Box::new(response(7, 3)))),
        ],
    );
    p.complete(request()).await.unwrap();
    assert!(
        policy
            .check_execution_authority()
            .unwrap_err()
            .to_string()
            .contains("Goal token limit")
    );
    assert!(m.measurement().complete);
    assert!(GoalMeter::new(10, Duration::ZERO).check_budget().is_err());
}

#[tokio::test]
async fn deadline_requests_cancellation_then_waits_for_cleanup_completion() {
    let cancel = tokio_util::sync::CancellationToken::new();
    let child = cancel.clone();
    let m = GoalMeter::new(10, Duration::ZERO);
    let work = async move {
        child.cancelled().await;
        tokio::task::yield_now().await;
        "cleanup observed"
    };
    assert_eq!(
        m.finish_with_deadline(work, cancel.clone()).await,
        "cleanup observed"
    );
    assert!(cancel.is_cancelled());
    let cancel = tokio_util::sync::CancellationToken::new();
    assert_eq!(
        meter()
            .finish_with_deadline(async { "completed" }, cancel.clone())
            .await,
        "completed"
    );
    assert!(!cancel.is_cancelled());
}

#[derive(Debug)]
struct Allocations {
    parent: uuid::Uuid,
    run: uuid::Uuid,
    receipts: Mutex<Vec<ExecutionUsage>>,
    closures: Mutex<Vec<uuid::Uuid>>,
    fail: bool,
}
#[async_trait]
impl Observer for Allocations {
    async fn close_allocation(
        &self,
        _target: uuid::Uuid,
        id: uuid::Uuid,
        _proof: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.closures.lock().unwrap().push(id);
        Ok(())
    }
    async fn record(&self, _: RequestObservation) -> anyhow::Result<()> {
        Ok(())
    }
    async fn allocate(&self, request: AllocationRequest) -> anyhow::Result<ExecutionBudget> {
        anyhow::ensure!(!self.fail, "fixture allocation checkpoint failure");
        Ok(ExecutionBudget {
            command_id: request.command_id,
            session_id: request.session_id,
            parent_session_id: self.parent,
            parent_run_id: self.run,
            tokens: request.tokens,
            elapsed_ms: request.elapsed_ms,
            expires_at_ms: request.expires_at_ms,
        })
    }
    async fn settle_allocation(&self, _: uuid::Uuid, usage: ExecutionUsage) -> anyhow::Result<()> {
        self.receipts.lock().unwrap().push(usage);
        Ok(())
    }
}
fn allocation_observer(fail: bool) -> Arc<Allocations> {
    Arc::new(Allocations {
        parent: uuid::Uuid::new_v4(),
        run: uuid::Uuid::new_v4(),
        receipts: Mutex::new(Vec::new()),
        closures: Mutex::new(Vec::new()),
        fail,
    })
}
fn child_usage(budget: ExecutionBudget, input_tokens: u64, output_tokens: u64) -> ExecutionUsage {
    ExecutionUsage {
        session_id: budget.session_id,
        budget,
        run_id: uuid::Uuid::new_v4(),
        input_tokens,
        output_tokens,
        elapsed_ms: 1,
        complete: true,
        cleanup_observed: true,
    }
}
#[tokio::test]
async fn concurrent_goal_allocations_share_allowance_and_receipts_are_once_only() {
    let observer = allocation_observer(false);
    let meter = GoalMeter::with_observer(100, Duration::from_secs(60), Some(observer.clone()));
    let target = uuid::Uuid::new_v4();
    let (first, second) = tokio::join!(
        meter.allocate(uuid::Uuid::new_v4(), target, uuid::Uuid::new_v4()),
        meter.allocate(uuid::Uuid::new_v4(), target, uuid::Uuid::new_v4())
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.tokens + second.tokens, 75);
    assert!(!meter.measurement().complete);
    let mut local_request = request();
    let mut attempt = meter.begin(&mut local_request).unwrap();
    assert_eq!(local_request.max_tokens, Some(25));
    attempt
        .observe(ReportedUsage {
            input_tokens: Some(2),
            output_tokens: Some(1),
        })
        .unwrap();
    attempt.finish(&response(2, 1)).await.unwrap();
    assert!(meter.observe_allocation_once(first.command_id));
    assert!(!meter.observe_allocation_once(first.command_id));
    let usage = child_usage(first.clone(), 10, 3);
    meter
        .settle_allocation(target, usage.clone())
        .await
        .unwrap();
    meter
        .settle_allocation(target, usage.clone())
        .await
        .unwrap();
    assert_eq!(observer.receipts.lock().unwrap().len(), 1);
    assert_eq!(
        meter
            .allocate(first.command_id, target, first.session_id)
            .await
            .unwrap(),
        first
    );
    assert!(
        meter
            .allocate(first.command_id, uuid::Uuid::new_v4(), first.session_id)
            .await
            .is_err()
    );
    let mut conflict = usage;
    conflict.input_tokens += 1;
    assert!(meter.settle_allocation(target, conflict).await.is_err());
    meter
        .settle_allocation(target, child_usage(second, 7, 4))
        .await
        .unwrap();
    let measured = meter.measurement();
    assert!(measured.complete);
    assert_eq!((measured.input_tokens, measured.output_tokens), (19, 8));
    tokio::time::timeout(
        Duration::from_millis(100),
        meter.wait_allocations(tokio_util::sync::CancellationToken::new()),
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn failed_or_unknown_goal_allocation_cannot_refund_an_allowance() {
    let meter = GoalMeter::with_observer(
        100,
        Duration::from_secs(60),
        Some(allocation_observer(true)),
    );
    assert!(
        meter
            .allocate(
                uuid::Uuid::new_v4(),
                uuid::Uuid::new_v4(),
                uuid::Uuid::new_v4()
            )
            .await
            .is_err()
    );
    assert!(!meter.measurement().complete);
    assert!(meter.check_budget().is_err());
    tokio::time::timeout(
        Duration::from_millis(100),
        meter.wait_allocations(tokio_util::sync::CancellationToken::new()),
    )
    .await
    .unwrap();
    let meter = GoalMeter::with_observer(
        100,
        Duration::from_secs(60),
        Some(allocation_observer(false)),
    );
    let target = uuid::Uuid::new_v4();
    let child = meter
        .allocate(uuid::Uuid::new_v4(), target, uuid::Uuid::new_v4())
        .await
        .unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    meter.wait_allocations(cancel).await;
    assert!(!meter.measurement().complete);
    let mut usage = child_usage(child, 5, 2);
    usage.complete = false;
    meter.settle_allocation(target, usage).await.unwrap();
    assert_eq!(
        (
            meter.measurement().input_tokens,
            meter.measurement().output_tokens
        ),
        (5, 2)
    );
    assert!(meter.check_budget().is_err());
}

#[tokio::test]
async fn goal_permanent_non_admission_counts_zero_once_without_fabricating_a_child_run() {
    let observer = allocation_observer(false);
    let meter = GoalMeter::with_observer(100, Duration::from_secs(60), Some(observer.clone()));
    let target = uuid::Uuid::new_v4();
    let child = meter
        .allocate(uuid::Uuid::new_v4(), target, uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert!(!meter.measurement().complete);
    assert!(
        meter
            .close_allocation(
                target,
                child.command_id,
                serde_json::json!({"status":"unknown","command_id":child.command_id})
            )
            .await
            .is_err()
    );
    let proof = serde_json::json!({"status":"not_admitted","command_id":child.command_id});
    meter
        .close_allocation(target, child.command_id, proof.clone())
        .await
        .unwrap();
    meter
        .close_allocation(target, child.command_id, proof)
        .await
        .unwrap();
    assert_eq!(observer.closures.lock().unwrap().len(), 1);
    assert!(observer.receipts.lock().unwrap().is_empty());
    assert!(meter.measurement().complete);
    assert_eq!(meter.measurement().input_tokens, 0);
    assert!(
        meter
            .settle_allocation(target, child_usage(child, 1, 1))
            .await
            .is_err()
    );
    let next = meter
        .allocate(uuid::Uuid::new_v4(), target, uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(next.tokens, 50);
}
#[derive(Debug)]
struct HeldImport {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait]
impl Observer for HeldImport {
    async fn record(&self, _: RequestObservation) -> anyhow::Result<()> {
        Ok(())
    }
    async fn allocate(&self, r: AllocationRequest) -> anyhow::Result<ExecutionBudget> {
        Ok(ExecutionBudget {
            command_id: r.command_id,
            session_id: r.session_id,
            parent_session_id: uuid::Uuid::new_v4(),
            parent_run_id: uuid::Uuid::new_v4(),
            tokens: r.tokens,
            elapsed_ms: r.elapsed_ms,
            expires_at_ms: r.expires_at_ms,
        })
    }
    async fn settle_allocation(&self, _: uuid::Uuid, _: ExecutionUsage) -> anyhow::Result<()> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(())
    }
}
#[tokio::test]
async fn goal_terminal_measurement_waits_for_durable_import_and_in_memory_totals_together() {
    let observer = Arc::new(HeldImport {
        started: Default::default(),
        release: Default::default(),
    });
    let meter = GoalMeter::with_observer(100, Duration::from_secs(60), Some(observer.clone()));
    let target = uuid::Uuid::new_v4();
    let child = meter
        .allocate(uuid::Uuid::new_v4(), target, uuid::Uuid::new_v4())
        .await
        .unwrap();
    let importer = meter.clone();
    let import = tokio::spawn(async move {
        importer
            .settle_allocation(target, child_usage(child, 12, 3))
            .await
            .unwrap()
    });
    observer.started.notified().await;
    let reader = meter.clone();
    let final_read = tokio::spawn(async move {
        let _guard = reader.settlement_guard().await;
        reader.measurement()
    });
    tokio::task::yield_now().await;
    assert!(!final_read.is_finished());
    observer.release.notify_one();
    import.await.unwrap();
    let measured = final_read.await.unwrap();
    assert!(measured.complete);
    assert_eq!((measured.input_tokens, measured.output_tokens), (12, 3));
}
