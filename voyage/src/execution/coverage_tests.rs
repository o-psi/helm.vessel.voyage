use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
#[tokio::test]
async fn cancellation_polling_retries_only_busy_and_stops_without_more_reads() {
    let stopped = tokio_util::sync::CancellationToken::new();
    let cancel = tokio_util::sync::CancellationToken::new();
    let failed = AtomicBool::new(false);
    let mut calls = 0;
    let result = poll_local_cancellation(
        || {
            calls += 1;
            let first = calls == 1;
            async move {
                if first {
                    Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                        None,
                    )
                    .into())
                } else {
                    Ok(true)
                }
            }
        },
        &stopped,
        &cancel,
        &failed,
    )
    .await
    .unwrap();
    assert!(result);
    assert_eq!(calls, 2);
    assert!(!cancel.is_cancelled() && !failed.load(Ordering::Relaxed));
    stopped.cancel();
    assert!(
        !poll_local_cancellation(
            || async { panic!("stopped poll must not read") },
            &stopped,
            &cancel,
            &failed
        )
        .await
        .unwrap()
    );
}
#[tokio::test]
async fn execution_interrupt_preserves_ready_result_and_bounds_unfinished_work() {
    let cancel = tokio_util::sync::CancellationToken::new();
    assert_eq!(
        await_execution(
            async { 7 },
            cancel.clone(),
            std::future::pending(),
            Duration::from_millis(10)
        )
        .await,
        Some(7)
    );
    assert_eq!(
        await_execution(
            std::future::pending::<u32>(),
            cancel.clone(),
            async {},
            Duration::from_millis(5)
        )
        .await,
        None
    );
    assert!(cancel.is_cancelled());
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let result = await_execution(
        async {
            tokio::time::sleep(Duration::from_millis(1)).await;
            9
        },
        cancel,
        std::future::pending(),
        Duration::from_millis(50),
    )
    .await;
    assert_eq!(result, Some(9));
}
