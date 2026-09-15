use super::*;
#[tokio::test]
async fn read_workers_are_drained_exactly_once_and_failures_remain_failures() {
    let read = OwnedRead {
        task: tokio::sync::Mutex::new(Some(tokio::spawn(async { Ok("offline fixture".into()) }))),
    };
    assert_eq!(read.take().await.unwrap(), "offline fixture");
    assert!(read.take().await.is_err());
    read.drain().await;
    let failed = OwnedRead {
        task: tokio::sync::Mutex::new(Some(tokio::spawn(async {
            anyhow::bail!("fixture failure")
        }))),
    };
    assert!(
        failed
            .take()
            .await
            .unwrap_err()
            .to_string()
            .contains("fixture failure")
    );
    let drained = OwnedRead {
        task: tokio::sync::Mutex::new(Some(tokio::spawn(async { Ok("discarded".into()) }))),
    };
    drained.drain().await;
    assert!(drained.take().await.is_err());
}
#[tokio::test]
async fn closed_manager_refuses_reads_and_restrictions_are_monotonic() {
    let manager = Manager::default();
    manager.read_allowed.store(true, Ordering::Release);
    manager.restrict_host_read(false);
    manager.restrict_host_read(true);
    assert!(!manager.read_allowed.load(Ordering::Acquire));
    assert!(
        manager
            .start_read(Uuid::new_v4(), || panic!("unowned work must not run"))
            .is_err()
    );
    manager.shutdown().await.unwrap();
    manager.shutdown().await.unwrap();
    assert!(manager.state.lock().unwrap().closed);
    assert!(
        manager
            .start_read(Uuid::new_v4(), || panic!("closed work must not run"))
            .is_err()
    );
}
