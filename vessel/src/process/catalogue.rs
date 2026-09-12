//! SQLite catalogue reads never wait for runtime processes. One bounded producer
//! refreshes metadata for all clients; failures retain details and back off.
use super::{database, registry, service::Supervisor};
use anyhow::Result;
use std::{sync::Arc, time::Duration};
use voyage_protocol::process::ProcessInfo;
impl Supervisor {
    pub(super) async fn catalogue(&self) -> Result<Vec<ProcessInfo>> {
        database::catalogue(&self.directory).await
    }
    pub(super) fn start_catalogue_refresh(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let supervisor = self.clone();
        tokio::spawn(async move {
            loop {
                let records: Vec<_> = match supervisor.registrations.lock().await {
                    Ok(records) => records.values().cloned().collect(),
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                };
                let mut work = tokio::task::JoinSet::new();
                for registration in records {
                    let root = supervisor.directory.clone();
                    work.spawn(async move {
                        let _ = database::refresh(&root, &registration).await;
                        let directory = registry::directory(&root, registration.session_id);
                        if directory.join("runtime.sock").exists()
                            && let Ok(info) = tokio::time::timeout(
                                Duration::from_millis(500),
                                super::routing::inspect(&directory, &registration),
                            )
                            .await
                        {
                            let _ = database::observe_process(&root, &registration, &info).await;
                        }
                    });
                    if work.len() >= 4 {
                        let _ = work.join_next().await;
                    }
                }
                while work.join_next().await.is_some() {}
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
    }
}
