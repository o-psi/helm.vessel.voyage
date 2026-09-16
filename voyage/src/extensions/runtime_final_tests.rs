//! Parent wiring: #[cfg(test)] #[path = "runtime_final_tests.rs"] mod final_tests;
//! Mock launch adapters never spawn a process or use the host environment.
use super::*;
use std::sync::atomic::AtomicUsize;
use tokio_util::sync::CancellationToken;

struct RefusingLaunch {
    launches: Arc<AtomicUsize>,
}
#[async_trait]
impl sdk::LaunchAdapter for RefusingLaunch {
    async fn launch(&self, _: &sdk::Identity, _: Instant) -> Result<sdk::Launched> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("fixture refuses before spawning")
    }
}

struct NoHost;
#[async_trait]
impl sdk::Host for NoHost {
    async fn read(
        &self,
        _: &sdk::Identity,
        _: &str,
        _: u64,
        _: usize,
        _: Instant,
    ) -> Result<String> {
        anyhow::bail!("no host read capability in this fixture")
    }
    async fn progress(&self, _: &sdk::Identity, _: &str) -> Result<()> {
        anyhow::bail!("no extension launched")
    }
}

fn executor() -> (Arc<sdk::Executor>, Arc<AtomicUsize>) {
    let launches = Arc::new(AtomicUsize::new(0));
    let executor = sdk::Executor::new(
        json!({
            "tools":[{"name":"fixture", "description":"offline fixture",
                "input_schema":{"type":"object","additionalProperties":false},
                "output_schema":{"type":"object"}}],
            "commands":[],"lifecycle":[]
        }),
        vec!["execute".into()],
        Arc::new(RefusingLaunch {
            launches: launches.clone(),
        }),
    )
    .unwrap();
    (executor, launches)
}
fn invocation() -> sdk::Invocation {
    sdk::Invocation {
        identity: sdk::Identity {
            session: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            run: Uuid::new_v4(),
            invocation: Uuid::new_v4(),
            package: "fixture".into(),
            digest: "a".repeat(64),
        },
        kind: sdk::Kind::Tool,
        name: "fixture".into(),
        arguments: json!({}),
        deadline: Instant::now() + Duration::from_secs(2),
        cancellation: CancellationToken::new(),
    }
}

#[tokio::test]
async fn manager_registration_is_lazy_and_shutdown_closes_sdk_admission() {
    let manager = Manager::default();
    let (executor, launches) = executor();
    manager.add_executor(executor.clone()).unwrap();
    assert_eq!(launches.load(Ordering::SeqCst), 0);
    assert!(!executor.tasks_drained());
    manager.shutdown().await.unwrap();
    assert!(manager.state.lock().unwrap().closed);
    assert!(executor.start(invocation(), Arc::new(NoHost)).is_err());
    assert_eq!(launches.load(Ordering::SeqCst), 0);
    assert!(executor.tasks_drained());
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn manager_reconciles_refused_mock_launch_only_after_task_drain() {
    let manager = Manager::default();
    let (executor, launches) = executor();
    manager.add_executor(executor.clone()).unwrap();
    let completed = executor
        .start(invocation(), Arc::new(NoHost))
        .unwrap()
        .completion()
        .await;
    assert!(completed.result.is_err());
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    assert_eq!(completed.cleanup, sdk::Cleanup::Pending);
    manager.shutdown().await.unwrap();
    assert!(executor.tasks_drained());
    assert!(manager.state.lock().unwrap().children.is_empty());
}

#[tokio::test]
async fn manager_does_not_accept_new_executors_after_shutdown() {
    let manager = Manager::default();
    manager.shutdown().await.unwrap();
    let (executor, launches) = executor();
    assert!(manager.add_executor(executor).is_err());
    assert_eq!(launches.load(Ordering::SeqCst), 0);
    assert!(manager.state.lock().unwrap().executors.is_empty());
}

#[tokio::test]
async fn executor_limit_is_exact_and_does_not_launch_during_registration() {
    let manager = Manager::default();
    let mut counters = Vec::new();
    for _ in 0..128 {
        let (executor, launches) = executor();
        manager.add_executor(executor).unwrap();
        counters.push(launches);
    }
    let (extra, launches) = executor();
    assert!(manager.add_executor(extra).is_err());
    assert_eq!(manager.state.lock().unwrap().executors.len(), 128);
    assert_eq!(launches.load(Ordering::SeqCst), 0);
    assert!(
        counters
            .iter()
            .all(|count| count.load(Ordering::SeqCst) == 0)
    );
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_call_is_rejected_before_mock_launch() {
    let manager = Manager::default();
    let (executor, launches) = executor();
    manager.add_executor(executor.clone()).unwrap();
    let mut call = invocation();
    call.arguments = json!({"unexpected":"field"});
    assert!(executor.start(call, Arc::new(NoHost)).is_err());
    let mut call = invocation();
    call.name = "not-pinned".into();
    assert!(executor.start(call, Arc::new(NoHost)).is_err());
    let mut call = invocation();
    call.identity.digest = "not-a-digest".into();
    assert!(executor.start(call, Arc::new(NoHost)).is_err());
    assert_eq!(launches.load(Ordering::SeqCst), 0);
    manager.shutdown().await.unwrap();
}

#[test]
fn unknown_read_owner_never_runs_blocking_work() {
    let manager = Manager::default();
    manager.read_allowed.store(true, Ordering::Release);
    let ran = Arc::new(AtomicBool::new(false));
    let observed = ran.clone();
    assert!(
        manager
            .start_read(Uuid::new_v4(), move || {
                observed.store(true, Ordering::Release);
                Ok("should not run".into())
            })
            .is_err()
    );
    assert!(!ran.load(Ordering::Acquire));
}

#[test]
fn read_capability_can_only_be_restricted() {
    let manager = Manager::default();
    assert!(!manager.read_allowed.load(Ordering::Acquire));
    manager.restrict_host_read(true);
    assert!(!manager.read_allowed.load(Ordering::Acquire));
    manager.read_allowed.store(true, Ordering::Release);
    manager.restrict_host_read(true);
    assert!(manager.read_allowed.load(Ordering::Acquire));
    manager.restrict_host_read(false);
    manager.restrict_host_read(true);
    assert!(!manager.read_allowed.load(Ordering::Acquire));
}

#[tokio::test]
async fn worker_panic_is_sanitized_and_drained_exactly_once() {
    let read = OwnedRead {
        task: tokio::sync::Mutex::new(Some(tokio::spawn(async {
            panic!("private worker panic diagnostic");
            #[allow(unreachable_code)]
            Ok(String::new())
        }))),
    };
    let error = read.take().await.unwrap_err().to_string();
    assert_eq!(error, "host read worker interrupted");
    assert!(!error.contains("private"));
    assert!(read.task.lock().await.is_none());
    assert_eq!(
        read.take().await.unwrap_err().to_string(),
        "host read already drained"
    );
    read.drain().await;
}

#[tokio::test]
async fn cancelled_task_is_not_converted_to_successful_read() {
    let task = tokio::spawn(async {
        std::future::pending::<()>().await;
        Ok("unreachable".into())
    });
    task.abort();
    let read = OwnedRead {
        task: tokio::sync::Mutex::new(Some(task)),
    };
    assert_eq!(
        read.take().await.unwrap_err().to_string(),
        "host read worker interrupted"
    );
    assert!(read.task.lock().await.is_none());
}

#[tokio::test]
async fn draining_a_pending_read_waits_for_its_actual_completion() {
    let (send, receive) = tokio::sync::oneshot::channel::<()>();
    let read = Arc::new(OwnedRead {
        task: tokio::sync::Mutex::new(Some(tokio::spawn(async move {
            receive.await.unwrap();
            Ok("discarded result".into())
        }))),
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(10), read.drain())
            .await
            .is_err()
    );
    assert!(read.task.lock().await.is_some());
    send.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), read.drain())
        .await
        .unwrap();
    assert!(read.task.lock().await.is_none());
    assert!(read.take().await.is_err());
}

#[tokio::test]
async fn concurrent_read_consumers_cannot_duplicate_the_result() {
    let read = Arc::new(OwnedRead {
        task: tokio::sync::Mutex::new(Some(tokio::spawn(async { Ok("once".into()) }))),
    });
    let (left, right) = tokio::join!(read.take(), read.take());
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert_eq!(left.or(right).unwrap(), "once");
    assert!(read.task.lock().await.is_none());
}

#[test]
fn confidential_value_is_detected_across_progress_and_result_boundaries() {
    let redactor = crate::tools::Redactor::new(vec!["private-token".into()]);
    for (progress, value) in [
        (vec!["private-".into()], json!("token")),
        (vec!["pri".into(), "vate-".into()], json!(["to", "ken"])),
        (vec![], json!({"a":"private-","b":"token"})),
        (vec![], json!({"private-":"token"})),
        (vec![], json!([{"a":"pri"}, ["vate-", {"b":"token"}]])),
    ] {
        assert!(confidential(&redactor, &progress, &value).is_err());
    }
}

#[test]
fn confidential_numeric_boolean_and_null_fragments_are_checked() {
    for (secret, value) in [
        ("pin1234", json!(["pin", 1234])),
        ("flagtrue", json!(["flag", true])),
        ("flagfalse", json!(["flag", false])),
        ("joined", json!(["join", null, "ed"])),
        ("keyvalue", json!({"key":"value"})),
    ] {
        let redactor = crate::tools::Redactor::new(vec![secret.into()]);
        assert!(confidential(&redactor, &[], &value).is_err());
    }
}

#[test]
fn confidentiality_budget_includes_decoded_keys_and_progress() {
    let redactor = crate::tools::Redactor::new(Vec::<String>::new());
    let boundary = "x".repeat(2 * 1024 * 1024);
    assert!(confidential(&redactor, &[], &json!(boundary)).is_ok());
    assert!(confidential(&redactor, &["x".into()], &json!(boundary)).is_err());
    assert!(confidential(&redactor, &[], &json!({"k":boundary})).is_err());
    assert!(confidential(&redactor, &["x".repeat(2 * 1024 * 1024 + 1)], &json!(null)).is_err());
}

#[test]
fn ordinary_structured_results_remain_accepted() {
    let redactor = crate::tools::Redactor::new(vec!["absent-confidential-value".into()]);
    for value in [
        json!(null),
        json!(false),
        json!(42),
        json!("safe"),
        json!([]),
        json!({"result":{"items":["alpha", 2, true, null]},"ok":true}),
    ] {
        confidential(&redactor, &["safe progress".into()], &value).unwrap();
    }
}
