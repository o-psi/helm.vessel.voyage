use super::*;

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct FailedWait(Arc<AtomicBool>);
#[cfg(target_os = "linux")]
impl portable_pty::ChildKiller for FailedWait {
    fn kill(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(self.clone())
    }
}
#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
impl Child for FailedWait {
    fn try_wait(&mut self) -> io::Result<Option<portable_pty::ExitStatus>> {
        if self.0.load(Ordering::SeqCst) {
            Err(io::Error::other(
                "secret-human-input\x1b[2Jhostile-diagnostic",
            ))
        } else {
            Ok(Some(portable_pty::ExitStatus::with_exit_code(0)))
        }
    }
    fn wait(&mut self) -> io::Result<portable_pty::ExitStatus> {
        Err(io::Error::other("blocking wait forbidden"))
    }
    fn process_id(&self) -> Option<u32> {
        None
    }
    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}
#[cfg(target_os = "linux")]
#[tokio::test]
async fn wait_failure_retains_handle_and_reports_only_sanitized_codes_then_retries() {
    let tool = ProcessTool::default();
    let failed = Arc::new(AtomicBool::new(true));
    let id = Uuid::new_v4();
    let mut child = OwnedChild::new(Box::new(FailedWait(failed.clone())));
    child.session_observed = true;
    tool.pending.lock().unwrap().insert(id, child);
    // Assert the injected observer error directly; a 10 ms blocking-pool
    // scheduling deadline can return TimedOut before any handle is inspected.
    // Dedicated timeout tests cover that separate conservative outcome.
    let report = tool.shutdown_step(Instant::now() + Duration::from_secs(5));
    assert!(!report.observation_complete);
    assert_eq!(report.remaining, vec![TerminalId(id)]);
    assert!(
        report
            .failures
            .contains(&TerminalShutdownFailure::WaitFailed)
    );
    let encoded = serde_json::to_string(&report).unwrap();
    assert!(!encoded.contains("secret"));
    assert!(!encoded.contains("hostile"));
    assert!(!encoded.contains('\x1b'));
    assert!(tool.pending.lock().unwrap().contains_key(&id));
    failed.store(false, Ordering::SeqCst);
    assert!(
        tool.shutdown(Duration::from_secs(5))
            .await
            .observation_complete
    );
    assert!(tool.pending.lock().unwrap().is_empty());
}
#[tokio::test]
async fn poisoned_storage_is_unconfirmed_even_when_map_looks_empty() {
    let tool = ProcessTool::default();
    let map = tool.processes.clone();
    let _ = std::thread::spawn(move || {
        let _lock = map.lock().unwrap();
        panic!("fixture poison");
    })
    .join();
    // This checks a specific diagnostic, not the observer scheduling deadline.
    // Use the foreground cleanup budget so a loaded blocking pool can inspect it.
    let report = tool.shutdown(Duration::from_secs(5)).await;
    assert!(!report.observation_complete);
    assert!(report.failures.contains(&TerminalShutdownFailure::Poisoned));
}
#[cfg(target_os = "linux")]
#[tokio::test]
async fn failed_start_after_spawn_remains_observable_and_is_reaped() {
    let dir = tempfile::tempdir().unwrap();
    let tool = ProcessTool::default();
    let context = super::super::tests::context(dir.path());
    assert!(
        tool.start_after_spawn(
            "exec sleep 30".into(),
            None,
            None,
            BTreeMap::new(),
            24,
            80,
            &context,
            || Err(ToolError::Failed("fixture setup failure".into()))
        )
        .is_err()
    );
    let pid = tool
        .pending
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .process_id()
        .unwrap();
    let report = tool.shutdown(Duration::from_secs(3)).await;
    assert!(report.observation_complete, "{report:?}");
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

#[cfg(target_os = "linux")]
#[test]
fn remaining_report_is_bounded_and_failed_handles_are_retained() {
    let tool = ProcessTool::default();
    for _ in 0..70 {
        let mut child = OwnedChild::new(Box::new(FailedWait(Arc::new(AtomicBool::new(true)))));
        child.session_observed = true;
        tool.pending.lock().unwrap().insert(Uuid::new_v4(), child);
    }
    let report = tool.shutdown_step(Instant::now() + Duration::from_secs(1));
    assert!(!report.observation_complete);
    assert_eq!(report.remaining.len(), 64);
    assert_eq!(report.omitted_remaining, 6);
    assert_eq!(tool.pending.lock().unwrap().len(), 70);
}
