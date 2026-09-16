use super::*;
#[tokio::test]
async fn cleanup_components_retry_only_finished_failures_and_never_duplicate_live_work() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = count.clone();
    let mut component = Component::new("fixture", move || {
        let seen = seen.clone();
        async move { seen.fetch_add(1, Ordering::SeqCst) > 0 }
    });
    component.advance().await;
    while !component.task.as_ref().unwrap().is_finished() {
        tokio::task::yield_now().await;
    }
    component.advance().await;
    assert!(!component.observed);
    assert_eq!(component.attempts, 1);
    component.next = Instant::now();
    component.advance().await;
    while !component.task.as_ref().unwrap().is_finished() {
        tokio::task::yield_now().await;
    }
    component.advance().await;
    assert!(component.observed);
    component.advance().await;
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert!(!component.blocked());
    let mut hanging = Component::new("pending", std::future::pending);
    hanging.advance().await;
    hanging.next = Instant::now();
    assert!(hanging.blocked());
    hanging.advance().await;
    assert_eq!(hanging.attempts, 1);
    hanging.task.take().unwrap().abort();
}
#[tokio::test]
async fn empty_cleanup_slot_reports_observed_but_locked_slot_does_not() {
    let slot = CleanupSlot::default();
    assert!(slot.advance(1).await.unwrap());
    assert!(slot.wait(Duration::from_millis(10), 1).await);
    slot.retry().await;
    let _guard = slot.0.lock().await;
    assert!(!slot.advance(1).await.unwrap());
    assert!(!slot.wait(Duration::from_millis(5), 1).await);
}
