use super::*;

#[tokio::test]
async fn empty_shutdown_is_repeatable_and_closes_shared_admission() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let tool = ProcessTool::default();
    let clone = tool.clone();
    assert!(tool.shutdown(Duration::from_secs(1)).await.observation_complete);
    assert!(clone.shutdown(Duration::ZERO).await.observation_complete);
    assert!(clone.execute(json!({"action":"start","command":"echo forbidden"}), &ctx).await.is_err());
}

#[tokio::test]
async fn shutdown_reports_lock_timeout_without_waiting_forever() {
    let tool = ProcessTool::default();
    let lock = tool.processes.lock().unwrap();
    let start = std::time::Instant::now();
    let report = tool.shutdown(Duration::from_millis(20)).await;
    assert!(!report.observation_complete);
    assert!(report.failures.contains(&TerminalShutdownFailure::TimedOut));
    assert!(start.elapsed() < Duration::from_secs(1));
    drop(lock);
    assert!(tool.shutdown(Duration::from_secs(1)).await.observation_complete);
}

#[tokio::test]
async fn cancelled_shutdown_waiter_does_not_reopen_admission() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let tool = ProcessTool::default();
    let lock = tool.processes.lock().unwrap();
    let copy = tool.clone();
    let task = tokio::spawn(async move { copy.shutdown(Duration::from_secs(10)).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !tool.shutting_down.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
    }).await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(lock);
    assert!(tool.execute(json!({"action":"start","command":"echo forbidden"}), &ctx).await.is_err());
    assert!(tool.shutdown(Duration::from_secs(1)).await.observation_complete);
}

#[tokio::test]
async fn approval_finishing_after_shutdown_cannot_spawn() {
    struct Paused { entered: tokio::sync::Notify, release: tokio::sync::Notify }
    #[async_trait]
    impl Approver for Paused {
        async fn approve(&self, _: &crate::tools::ApprovalRequest) -> crate::tools::ApprovalOutcome {
            self.entered.notify_one(); self.release.notified().await;
            crate::tools::ApprovalOutcome::Approved
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = context(dir.path());
    ctx.policy = Arc::new(Policy::new(&Config { approval: ApprovalMode::Always, ..Config::default() }, dir.path().into()).unwrap());
    let paused = Arc::new(Paused { entered: tokio::sync::Notify::new(), release: tokio::sync::Notify::new() });
    ctx.approver = paused.clone();
    let tool = ProcessTool::default();
    let copy = tool.clone();
    let task = tokio::spawn(async move { copy.execute(json!({"action":"start","command":"echo should-not-start"}), &ctx).await });
    paused.entered.notified().await;
    assert!(tool.shutdown(Duration::from_secs(1)).await.observation_complete);
    paused.release.notify_one();
    assert!(task.await.unwrap().is_err());
    assert!(tool.metadata().unwrap().is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn shutdown_observes_descendants_in_other_job_groups_and_preserves_other_sessions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("child.py"), r#"import os,time
child=os.fork()
if child == 0:
 os.setpgid(0,0)
 while True: time.sleep(1)
open('ready','w').write(str(os.getpid())+' '+str(child))
while True: time.sleep(1)
"#).unwrap();
    let ctx = context(dir.path());
    let owned = ProcessTool::default();
    let other = ProcessTool::default();
    owned.execute(json!({"action":"start","command":"exec python3 child.py"}), &ctx).await.unwrap();
    other.execute(json!({"action":"start","command":"exec sleep 30"}), &ctx).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !dir.path().join("ready").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await.unwrap();
    let pids: Vec<u32> = std::fs::read_to_string(dir.path().join("ready")).unwrap().split_whitespace().map(|p| p.parse().unwrap()).collect();
    let report = owned.shutdown(Duration::from_secs(3)).await;
    assert!(report.observation_complete, "{report:?}");
    assert!(report.remaining.is_empty());
    assert!(!std::path::Path::new(&format!("/proc/{}",pids[0])).exists(), "direct child must be reaped");
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat",pids[1])) {
        assert_eq!(stat.rsplit_once(") ").unwrap().1.split_whitespace().next(),Some("Z"));
    }
    assert_eq!(other.metadata().unwrap()[0].state,"running");
    assert!(other.shutdown(Duration::from_secs(3)).await.observation_complete);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ordinary_status_reads_do_not_reap_session_leader_before_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let tool = ProcessTool::default();
    tool.execute(json!({"action":"start","command":"exit 0"}), &context(dir.path())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while tool.metadata().unwrap()[0].state == "running" { tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await.unwrap();
    let pid = tool.processes.lock().unwrap().values().next().unwrap().child.process_id().unwrap();
    assert!(std::path::Path::new(&format!("/proc/{pid}")).exists(), "leader identity stays reserved until explicit cleanup");
    assert!(tool.shutdown(Duration::from_secs(3)).await.observation_complete);
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}
