//! Retain transport children until their original process sessions are observed empty.
use super::{AppClient, ProviderError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

fn clients() -> &'static Mutex<Vec<Arc<AppClient>>> {
    static CLIENTS: OnceLock<Mutex<Vec<Arc<AppClient>>>> = OnceLock::new();
    CLIENTS.get_or_init(Mutex::default)
}

pub(super) fn capacity() -> Result<tokio::sync::OwnedSemaphorePermit, ProviderError> {
    static CAPACITY: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    CAPACITY
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(128)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ProviderError::Unavailable(
                "compatibility process capacity exhausted; cleanup required".into(),
            )
        })
}

pub(super) fn retain(client: Arc<AppClient>) -> Result<(), ProviderError> {
    clients()
        .lock()
        .map_err(|_| {
            ProviderError::Unavailable("compatibility resource registry unavailable".into())
        })?
        .push(client);
    Ok(())
}

pub(crate) async fn shutdown() -> anyhow::Result<()> {
    let retained = clients()
        .lock()
        .map_err(|_| anyhow::anyhow!("compatibility resource registry unavailable"))?
        .clone();
    let results = futures_util::future::join_all(
        retained
            .into_iter()
            .map(|client| tokio::task::spawn_blocking(move || observe(&client))),
    )
    .await;
    anyhow::ensure!(
        results
            .into_iter()
            .all(|result| matches!(result, Ok(Ok(())))),
        "compatibility process cleanup unconfirmed"
    );
    clients()
        .lock()
        .map_err(|_| anyhow::anyhow!("compatibility resource registry unavailable"))?
        .retain(|client| {
            !client
                .cleanup_observed
                .load(std::sync::atomic::Ordering::Acquire)
        });
    Ok(())
}

fn observe(client: &AppClient) -> anyhow::Result<()> {
    if client
        .cleanup_observed
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let identity = client
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("compatibility session identity unavailable"))?;
        let mut child = client
            .child
            .lock()
            .map_err(|_| anyhow::anyhow!("compatibility child unavailable"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match identity.kill_and_observe(deadline) {
                Ok(true) => break,
                Ok(false) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => anyhow::bail!("compatibility original session cleanup unconfirmed"),
            }
        }
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "compatibility child reap deadline elapsed"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        client
            .cleanup_observed
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = client;
        anyhow::bail!("compatibility process cleanup observation unsupported on this platform")
    }
}
