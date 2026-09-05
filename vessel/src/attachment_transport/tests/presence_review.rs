//! Independent presence boundary regressions; synthetic registry entries have no
//! monitor task, so publication cannot accidentally rely on monitor scheduling.
use super::*;

async fn monitor_free_connection(f: &Fixture) -> Connection {
    let receipt = f
        .api
        .enrollment
        .attachment_current(f.machine, 1)
        .await
        .unwrap();
    let (send, _receiver) = mpsc::channel(1);
    Connection {
        id: Uuid::new_v4(),
        owner: receipt.owner_id,
        epoch: 1,
        features: Features::default(),
        cancel: CancellationToken::new(),
        send,
        heartbeat: watch::channel(tokio::time::Instant::now()).0,
    }
}

#[tokio::test]
async fn metadata_rechecks_expiry_without_monitor_and_does_not_refresh_it() {
    let f = Fixture::new().await;
    let connection = monitor_free_connection(&f).await;
    f.api
        .registry
        .lock()
        .await
        .insert(f.machine, connection.clone());
    let records = f.api.connections().await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].connection_id, connection.id);
    let expired = tokio::time::Instant::now() - f.api.limits.lease - Duration::from_secs(1);
    connection.heartbeat.send_replace(expired);
    assert!(f.api.connections().await.is_empty());
    assert!(!f.api.is_current(f.machine, connection.id).await);
    assert_eq!(*connection.heartbeat.borrow(), expired);
    // No task removed the entry: filtering is performed at the observation boundary.
    assert!(f.api.registry.lock().await.contains_key(&f.machine));
}

#[tokio::test]
async fn metadata_rechecks_revocation_without_monitor_or_socket_activity() {
    let mut f = Fixture::new().await;
    let connection = monitor_free_connection(&f).await;
    f.api
        .registry
        .lock()
        .await
        .insert(f.machine, connection.clone());
    assert_eq!(f.api.connections().await.len(), 1);
    f.store.revoke(f.machine, 1, Uuid::new_v4(), now()).unwrap();
    assert!(f.api.connections().await.is_empty());
    assert!(!f.api.is_current(f.machine, connection.id).await);
    assert!(connection.cancel.is_cancelled());
    assert!(f.api.registry.lock().await.contains_key(&f.machine));
}

#[tokio::test]
async fn metadata_rejects_wrong_owner_and_cancelled_generation() {
    let f = Fixture::new().await;
    let mut wrong = monitor_free_connection(&f).await;
    wrong.owner = Uuid::new_v4();
    f.api.registry.lock().await.insert(f.machine, wrong.clone());
    assert!(f.api.connections().await.is_empty());
    assert!(wrong.cancel.is_cancelled());

    let current = monitor_free_connection(&f).await;
    f.api
        .registry
        .lock()
        .await
        .insert(f.machine, current.clone());
    assert!(!f.api.is_current(f.machine, wrong.id).await);
    assert!(!current.cancel.is_cancelled());
    assert_eq!(f.api.connections().await[0].connection_id, current.id);
    current.cancel.cancel();
    assert!(f.api.connections().await.is_empty());
}

#[tokio::test]
async fn shutdown_fences_pending_admission_before_registry_lock_is_released() {
    let mut f = Fixture::new().await;
    let proof = f.proof();
    let mut socket = f.socket().await;
    let registry = f.api.registry.lock().await;
    socket
        .send(ClientMessage::Text(
            Frame::Authenticate {
                version: 2,
                proof,
                features: Features::default(),
            }
            .encode()
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let api = f.api.clone();
    let mut shutdown = tokio::spawn(async move { api.shutdown().await });
    // This latch is set before shutdown waits for our held registry lock. It is
    // a deterministic ordering barrier, independent of authentication scheduling.
    tokio::time::timeout(Duration::from_secs(2), f.api.closed.cancelled())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut shutdown)
            .await
            .is_err()
    );
    assert!(registry.is_empty());
    drop(registry);
    tokio::time::timeout(Duration::from_secs(2), shutdown)
        .await
        .unwrap()
        .unwrap();
    closed(&mut socket).await;
    assert!(f.api.registry.lock().await.is_empty());
    assert!(f.api.connections().await.is_empty());
    assert!(matches!(
        f.rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}
