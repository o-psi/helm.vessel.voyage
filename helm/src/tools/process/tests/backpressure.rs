//! A native child-process watchdog catches executor-blocking I/O that a Tokio
//! timeout on the blocked runtime cannot catch.
use super::*;

#[cfg(unix)]
#[test]
fn native_backpressure_never_blocks_other_terminals_or_shutdown() {
    const CHILD: &str = "HELM_PTY_BACKPRESSURE_CHILD";
    if let Ok(marker) = std::env::var(CHILD) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let dir = tempfile::tempdir().unwrap();
                let ctx = context(dir.path());
                let manager = ProcessTool::default();
                let mut ids = Vec::new();
                for _ in 0..2 {
                    let started = manager.execute(json!({"action":"start","command":r"stty raw -echo; printf ready; while :; do printf '\033[6n'; sleep 0.1; done"}), &ctx).await.unwrap();
                    ids.push(Uuid::parse_str(started.split_whitespace().last().unwrap()).unwrap());
                }
                let pids = manager.processes.lock().unwrap().values().filter_map(|p|p.child.process_id()).map(|p|p.to_string()).collect::<Vec<_>>().join("\n");
                std::fs::write(marker, pids).unwrap();
                for id in &ids {
                    tokio::time::timeout(Duration::from_secs(3), async {
                        loop {
                            if manager.read(*id, 4096).unwrap().contains("ready") { break; }
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    }).await.unwrap();
                }
                let writing = manager.clone();
                let id = TerminalId(ids[0]);
                let blocked = tokio::spawn(async move { InteractiveTerminals::write(&writing, id, vec![b'x'; 65536]).await });
                tokio::time::sleep(Duration::from_millis(20)).await;
                tokio::time::timeout(Duration::from_millis(200), async {
                    assert_eq!(InteractiveTerminals::list(&manager).await.unwrap().len(), 2);
                    InteractiveTerminals::resize(&manager, TerminalId(ids[1]), 100, 30).await.unwrap();
                    assert_eq!(InteractiveTerminals::snapshot(&manager, TerminalId(ids[1])).await.unwrap().cells.len(), 30);
                }).await.expect("a blocked terminal writer froze another terminal");
                blocked.abort();
                let _ = tokio::time::timeout(Duration::from_millis(200), blocked).await.expect("dropped caller stranded the executor");
                assert!(tokio::time::timeout(Duration::from_millis(100), InteractiveTerminals::write(&manager, id, vec![b'x'; 1024*1024])).await.unwrap().is_err());
                let writing = manager.clone();
                let context = ctx.clone();
                let cancellation = context.cancellation.clone();
                let model_id = ids[1];
                let model = tokio::spawn(async move { writing.execute(json!({"action":"write","id":model_id,"data":"x".repeat(65536)}), &context).await });
                tokio::time::sleep(Duration::from_millis(10)).await;
                assert!(!model.is_finished(), "model backpressure must remain active until cancellation");
                cancellation.cancel();
                assert!(tokio::time::timeout(Duration::from_millis(100), model).await.unwrap().unwrap().is_err());
                let report = manager.shutdown(Duration::from_secs(5)).await;
                #[cfg(target_os="linux")]
                assert!(report.observation_complete, "{report:?}");
                #[cfg(not(target_os="linux"))]
                assert!(!report.observation_complete, "native cleanup evidence must remain conservative");
            });
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("pids");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tools::process::tests::backpressure::native_backpressure_never_blocks_other_terminals_or_shutdown", "--nocapture"])
        .env(CHILD, &marker).spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if status.is_none_or(|status| !status.success()) {
        if let Ok(pids) = std::fs::read_to_string(marker) {
            for pid in pids.lines().filter_map(|s| s.parse::<i32>().ok()) {
                // Only the disposable PTY session leaders recorded by this child.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
        panic!("native PTY write watchdog failed: {status:?}");
    }
}
