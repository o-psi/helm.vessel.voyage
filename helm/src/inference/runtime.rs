use super::*;
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// Bound both outstanding blocking jobs and the caller's wait. A cancelled or
/// timed-out join does not undo a committed permit; its unknown record remains.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let slot = SLOTS
            .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(16)))
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Failure::Unavailable)?;
        tokio::task::spawn_blocking(move || {
            let _slot = slot;
            work()
        })
        .await
        .map_err(|_| Failure::Unavailable)?
    })
    .await
    .map_err(|_| Failure::Unavailable)?;
    result.map_err(|error| Failure::from_error(&error).into())
}

/// Historical operator queries never construct a provider or admit inference.
pub async fn read_history(
    workspace: PathBuf,
    session: Option<Uuid>,
    query: history::Query,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<history::History> {
    let token = cancel.clone();
    tokio::select! {biased; _=cancel.cancelled()=>Err(Failure::Unavailable.into()), result=blocking(move || {
        ensure!(!token.is_cancelled(),Failure::Unavailable);
        let path=Store::default_path();std::fs::create_dir_all(path.parent().context("inference store has no parent")?)?;
        let mut store=Store::open(path)?;let project=store.project(&workspace)?;
        let scope=if let Some(session)=session {ensure!(store.session_project(session)?==project,Failure::Invalid);Scope::Session(session)}else{Scope::Project(project)};
        store.history(scope,query,&token)
    })=>result}
}

#[derive(Clone)]
pub struct Accounting {
    store: Arc<Mutex<Store>>,
    project: Option<Uuid>,
    provider: String,
    compatibility: bool,
    agent: Option<Uuid>,
}
impl Accounting {
    async fn shared() -> Result<Arc<Mutex<Store>>> {
        blocking(|| {
            static STORES: OnceLock<Mutex<std::collections::HashMap<PathBuf, Weak<Mutex<Store>>>>> =
                OnceLock::new();
            let directory = Store::default_path();
            std::fs::create_dir_all(
                directory
                    .parent()
                    .context("inference store has no parent")?,
            )?;
            let mut stores = STORES
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| Failure::Unavailable)?;
            if let Some(store) = stores.get(&directory).and_then(Weak::upgrade) {
                return Ok(store);
            }
            let store = Arc::new(Mutex::new(Store::open(directory.clone())?));
            stores.insert(directory, Arc::downgrade(&store));
            Ok(store)
        })
        .await
    }
    async fn database<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.store.clone();
        blocking(move || {
            let mut store = store.lock().map_err(|_| Failure::Unavailable)?;
            work(&mut store)
        })
        .await
    }
    #[cfg(test)]
    pub(crate) fn test_store(&self) -> Arc<Mutex<Store>> {
        self.store.clone()
    }
    #[cfg(test)]
    pub(crate) fn fixture(store: Store, project: Uuid) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            project: Some(project),
            provider: "fixture".into(),
            compatibility: false,
            agent: None,
        }
    }
    pub async fn root(workspace: &Path, profile: &crate::config::ProviderProfile) -> Result<Self> {
        let mut accounting = Self {
            store: Self::shared().await?,
            project: None,
            provider: profile.id.into(),
            compatibility: profile.compatibility_bridge,
            agent: None,
        };
        let workspace = workspace.to_owned();
        accounting.project = Some(
            accounting
                .database(move |store| store.project(&workspace))
                .await?,
        );
        Ok(accounting)
    }
    pub async fn child(profile: &crate::config::ProviderProfile, agent: Uuid) -> Result<Self> {
        Ok(Self {
            store: Self::shared().await?,
            project: None,
            provider: profile.id.into(),
            compatibility: profile.compatibility_bridge,
            agent: Some(agent),
        })
    }
    pub async fn bind(&self, session: Uuid) -> Result<()> {
        let project = self.project;
        self.database(move |store| {
            if let Some(project) = project {
                store.bind_session(project, session)?;
            } else {
                store.session_project(session)?;
            }
            Ok(())
        })
        .await
    }
    pub async fn admit(
        &self,
        reference: &crate::completion::runtime::RunReference,
        model: &str,
        purpose: Purpose,
    ) -> Result<Permit> {
        let expected = self.project;
        let native = !self.compatibility;
        let attribution = Attribution {
            session: reference.session_id,
            run: reference.run_id,
            agent: self.agent,
            provider: self.provider.clone(),
            model: model.into(),
            purpose,
        };
        self.database(move |store| {
            let project = store.session_project(attribution.session)?;
            ensure!(
                expected.is_none_or(|expected| expected == project),
                Failure::Invalid
            );
            store.admit_native(&attribution, native)
        })
        .await
    }
    pub async fn report(
        &self,
        permit: &Permit,
        report: crate::provider::ReportedUsage,
    ) -> Result<()> {
        if permit.retained {
            let id = permit.id;
            self.database(move |store| store.report(id, report.input_tokens, report.output_tokens))
                .await?;
        }
        Ok(())
    }
    pub async fn finish(&self, permit: &Permit, outcome: AttemptOutcome) -> Result<()> {
        if permit.retained {
            let id = permit.id;
            self.database(move |store| store.finish_reported(id, outcome))
                .await?;
        }
        Ok(())
    }
    pub async fn history(
        &self,
        session: Uuid,
        project_scope: bool,
        query: history::Query,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<history::History> {
        let token = cancel.clone();
        tokio::select! {biased; _=cancel.cancelled()=>Err(Failure::Unavailable.into()), result=self.database(move |store| {
            ensure!(!token.is_cancelled(),Failure::Unavailable);
            let project=store.session_project(session)?;
            store.history(if project_scope {Scope::Project(project)}else{Scope::Session(session)},query,&token)
        })=>result}
    }
    pub async fn status(&self, session: Uuid) -> Result<Vec<Status>> {
        self.database(move |store| {
            let project = store.session_project(session)?;
            Ok(vec![
                store.inspect(Scope::Project(project))?,
                store.inspect(Scope::Session(session))?,
            ])
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_blocking_wait_keeps_later_commit_without_dispatching() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(temp.path().join("inference")).unwrap();
        let project = store.project(temp.path()).unwrap();
        let session = Uuid::new_v4();
        store.bind_session(project, session).unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let dispatched = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = dispatched.clone();
        let task = tokio::spawn(async move {
            let _permit = blocking(move || {
                let _ = started_tx.send(());
                release_rx.recv().unwrap();
                let permit = store.admit(&Attribution {
                    session,
                    run: Uuid::new_v4(),
                    agent: None,
                    provider: "fixture".into(),
                    model: "fixture".into(),
                    purpose: Purpose::Conversation,
                })?;
                let _ = finished_tx.send(());
                Ok(permit)
            })
            .await
            .unwrap();
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        started_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();
        finished_rx.await.unwrap();
        assert!(!dispatched.load(std::sync::atomic::Ordering::SeqCst));
        let rows = Store::open(temp.path().join("inference"))
            .unwrap()
            .attempts(Scope::Project(project), 0, 100)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, AttemptOutcome::Unknown);
    }
}
