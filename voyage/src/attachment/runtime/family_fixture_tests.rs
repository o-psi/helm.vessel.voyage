//! Actual ordinary owner/journal state, with no provider account or host changes.
use super::*;
use crate::{
    model::{ModelRequest, ModelResponse, Role},
    provider::{Provider, ProviderError},
    session::Session,
    tools::ToolRegistry,
};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) struct Fixture {
    pub root: tempfile::TempDir,
    pub owner: ManagedSessionOwner,
    pub actor: super::super::local_actor::LocalActor,
    pub incarnation: Uuid,
}
impl Fixture {
    pub async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("journal");
        let session = Session::new(root.path().into(), "fixture".into());
        let mut journal = Journal::open(directory.clone()).unwrap();
        journal.create_session(&session).unwrap();
        drop(journal);
        let owner = ManagedSessionOwner::open(directory, session.id)
            .await
            .unwrap();
        owner.initialize_process_commands().await.unwrap();
        owner.initialize_session_resources().await.unwrap();
        let actor = super::super::local_actor::LocalActor {
            installation_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
        };
        owner
            .initialize_command_bindings(actor.principal_id)
            .await
            .unwrap();
        let incarnation = Uuid::new_v4();
        owner
            .bind_notification_incarnation(incarnation)
            .await
            .unwrap();
        Self {
            root,
            owner,
            actor,
            incarnation,
        }
    }
    pub async fn request(&self) -> TurnAdmission {
        TurnAdmission {
            budget: None,
            coordination: None,
            operator_name: None,
            command_id: Uuid::new_v4(),
            machine_id: self.actor.installation_id,
            principal_id: self.actor.principal_id,
            session_id: self.owner.session_id(),
            expected_revision: self.owner.snapshot().await.unwrap().revision,
            expires_at_ms: now() + 120_000,
            prompt: "Original owned user input 世界".into(),
            parts: vec![],
        }
    }
    pub async fn admit(&self) -> (TurnAdmission, RunOwner) {
        let request = self.request().await;
        let Admission::New(run) = self.owner.admit(request.clone()).await.unwrap() else {
            panic!("fresh run")
        };
        (request, run)
    }
    pub async fn running(&self) -> (TurnAdmission, RunOwner) {
        let (request, mut run) = self.admit().await;
        run.register_local_cleanup().await.unwrap();
        run.start_operator().await.unwrap();
        (request, run)
    }
}
pub(super) fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub(super) fn expiry() -> u64 {
    (now() + 120_000) as u64
}

#[derive(Debug)]
pub(super) struct Authority {
    pub valid: AtomicBool,
    pub checks: AtomicUsize,
}
impl Authority {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            valid: AtomicBool::new(true),
            checks: AtomicUsize::new(0),
        })
    }
}
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> anyhow::Result<()> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(
            self.valid.load(Ordering::SeqCst),
            "owned synthetic authority revoked"
        );
        Ok(())
    }
}
pub(super) enum Reply {
    Text(&'static str),
    Fail,
}
pub(super) struct Scripted {
    pub calls: Arc<AtomicUsize>,
    pub reply: std::sync::Mutex<Option<Reply>>,
}
#[async_trait]
impl Provider for Scripted {
    async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut message = Message::new(Role::Assistant, "");
        match self
            .reply
            .lock()
            .unwrap()
            .take()
            .expect("no inference replay")
        {
            Reply::Text(text) => message.content = text.into(),
            Reply::Fail => {
                return Err(ProviderError::InvalidResponse(
                    "synthetic-private-provider-diagnostic".into(),
                ));
            }
        }
        Ok(ModelResponse {
            message,
            usage: Usage::default(),
            service_tier: None,
        })
    }
}
pub(super) fn agent(
    root: &std::path::Path,
    model: &str,
    reply: Reply,
) -> (Agent, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::new(
        Box::new(Scripted {
            calls: calls.clone(),
            reply: std::sync::Mutex::new(Some(reply)),
        }),
        ToolRegistry::default(),
        crate::tools::reliability_tests::context(root),
        Arc::new(crate::agent::SilentSink),
        model.into(),
        "Owned runtime fixture".into(),
        32,
        None,
    );
    (agent, calls)
}
