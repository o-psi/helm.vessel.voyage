use super::*;
use serde_json::Value;
use voyage_protocol::process::RuntimeCommand;
impl ManagedSessionOwner {
    pub(crate) async fn finish_decision(
        &self,
        id: Uuid,
        outcome: &'static str,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.finish_decision(guard, id, outcome)
        })
        .await?
    }

    pub(crate) async fn create_decision(
        &self,
        run: Uuid,
        incarnation: Uuid,
        id: Uuid,
        expires: i64,
        request: Value,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.create_decision(guard, run, incarnation, id, expires, request)
        })
        .await?
    }
    pub(crate) async fn decisions(&self, incarnation: Uuid) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.decisions(incarnation, SystemClock.now_ms()?)
        })
        .await?
    }
    pub(crate) async fn decision_response(&self, id: Uuid) -> anyhow::Result<Option<Value>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.decision_response(id, SystemClock.now_ms()?)
        })
        .await?
    }
    pub(crate) async fn respond_decision(
        &self,
        incarnation: Uuid,
        command: RuntimeCommand,
        authorize: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.respond_decision(guard, incarnation, &command, move || {
                authorize()?;
                SystemClock.now_ms()
            })
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn queued_response_rechecks_authority_after_owner_lock() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("journal");
        let session = crate::session::Session::new(root.path().into(), "fixture".into());
        let mut journal = Journal::open(directory.clone()).unwrap();
        journal.create_session(&session).unwrap();
        drop(journal);
        let owner = ManagedSessionOwner::open(directory, session.id)
            .await
            .unwrap();
        let current = Arc::new(AtomicBool::new(true));
        let checked = Arc::new(AtomicBool::new(false));
        let authority = current.clone();
        let observed = checked.clone();
        let command = RuntimeCommand::Respond {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: 1000,
            run_id: Uuid::new_v4(),
            decision_id: Uuid::new_v4(),
            response: serde_json::json!("approved"),
        };
        // Deterministically hold the actual owner mutex while polling the response
        // into its blocking task. No timing sleeps or production test hooks.
        let store = owner.store.lock().unwrap();
        let response = owner.respond_decision(Uuid::new_v4(), command, move || {
            observed.store(true, Ordering::SeqCst);
            anyhow::ensure!(authority.load(Ordering::SeqCst), "responder grant revoked");
            Ok(())
        });
        tokio::pin!(response);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(std::future::Future::poll(response.as_mut(), &mut context).is_pending());
        assert!(!checked.load(Ordering::SeqCst));
        current.store(false, Ordering::SeqCst);
        drop(store);
        let error = response.await.unwrap_err();
        assert!(checked.load(Ordering::SeqCst));
        assert!(
            error.to_string().contains("responder grant revoked"),
            "{error:#}"
        );
    }
}
