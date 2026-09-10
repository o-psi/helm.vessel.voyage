use super::*;
use crate::{
    agent::SilentSink,
    attachment::journal::{SteeringActor, SteeringAdmission},
    model::{ModelRequest, ModelResponse, Role},
    provider::{ModelInfo, Provider, ProviderError},
    tools::{InteractionMode, Redactor, ToolContext, ToolRegistry, UnattendedApprover},
};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

type Request = (
    ModelRequest,
    oneshot::Sender<Result<&'static str, ProviderError>>,
);
struct Scripted(mpsc::UnboundedSender<Request>);
#[async_trait]
impl Provider for Scripted {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(vec![ModelInfo::minimal(
            crate::titles::title_model().unwrap(),
        )])
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let (send, receive) = oneshot::channel();
        self.0.send((request, send)).unwrap();
        let text = receive
            .await
            .map_err(|_| ProviderError::Request("fixture closed".into()))??;
        Ok(ModelResponse {
            message: Message::new(Role::Assistant, text),
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
            service_tier: None,
        })
    }
}

async fn fixture() -> (
    tempfile::TempDir,
    ManagedSessionOwner,
    RunOwner,
    Agent,
    mpsc::UnboundedReceiver<Request>,
) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("journal");
    let session = crate::session::Session::new(root.path().into(), "fixture".into());
    let mut journal = Journal::open(directory.clone()).unwrap();
    journal.create_session(&session).unwrap();
    drop(journal);
    let owner = ManagedSessionOwner::open(directory, session.id)
        .await
        .unwrap();
    let admission = owner
        .admit(TurnAdmission {
            coordination: None,
            operator_name: None,
            command_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            session_id: session.id,
            expected_revision: 0,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 60000,
            prompt: "Make voyage names describe my intent".into(),
            parts: vec![],
        })
        .await
        .unwrap();
    let Admission::New(run) = admission else {
        panic!("new turn required")
    };
    let config = crate::Config::default();
    let context = ToolContext {
        tool_call_id: None,
        artifact_scope: None,
        github: None,
        completion: None,
        policy: Arc::new(crate::policy::Policy::new(&config, root.path().into()).unwrap()),
        approver: Arc::new(UnattendedApprover { allow: false }),
        timeout: Duration::from_secs(3),
        max_output_bytes: 4096,
        environment: Default::default(),
        cancellation: CancellationToken::new(),
        execution_id: Uuid::new_v4(),
        interaction: InteractionMode::Unattended,
        redactor: Arc::new(Redactor::default()),
    };
    let (send, receive) = mpsc::unbounded_channel();
    let agent = Agent::new(
        Box::new(Scripted(send)),
        ToolRegistry::default(),
        context,
        Arc::new(SilentSink),
        "fixture".into(),
        "Fixture".into(),
        128,
        None,
    );
    (root, owner, run, agent, receive)
}
async fn name_is(owner: &ManagedSessionOwner, expected: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if owner.snapshot().await.unwrap().session.display_name() == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("title did not persist while execution was still pending");
}
fn steering(owner: &ManagedSessionOwner, run: &RunOwner, text: &str) -> SteeringAdmission {
    SteeringAdmission {
        coordination: None,
        receipt_id: Uuid::new_v4(),
        session_id: owner.session_id(),
        run_id: run.run_id,
        actor: SteeringActor {
            machine_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
        },
        expected_revision: 0,
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 60000,
        text: text.into(),
    }
}

#[tokio::test]
async fn submission_and_queued_steering_refresh_without_completion_or_duplicate_inference() {
    let (_root, owner, mut run, agent, mut requests) = fixture().await;
    let handle = run.enable_steering(Arc::new(|_, _, _| Ok(()))).unwrap();
    let request = steering(
        &owner,
        &run,
        "Update on every user message, including steering",
    );
    let checkpoint = run.checkpoint();
    tokio::time::timeout(
        Duration::from_secs(5),
        checkpoint.with_title_updates(&agent, CancellationToken::new(), async {
            let (first, reply) = requests.recv().await.unwrap();
            assert!(first.messages[1].content.contains("Make voyage names"));
            reply.send(Ok("Name voyages by intent")).unwrap();
            name_is(&owner, "Name voyages by intent").await;
            assert!(matches!(
                run.record().await.unwrap().state,
                RunState::Accepted
            ));
            assert!(!handle.submit(request.clone()).await.unwrap().duplicate);
            let (second, reply) = requests.recv().await.unwrap();
            assert!(second.messages[1].content.contains("including steering"));
            reply.send(Ok("Refresh names on user input")).unwrap();
            name_is(&owner, "Refresh names on user input").await;
            assert!(handle.submit(request).await.unwrap().duplicate);
            assert!(
                tokio::time::timeout(Duration::from_millis(40), requests.recv())
                    .await
                    .is_err()
            );
            // Title projections must not apply queued steering to canonical history.
            assert_eq!(owner.snapshot().await.unwrap().session.messages.len(), 1);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn newer_intent_prevents_stale_title_and_drains_after_execution() {
    let (_root, owner, mut run, agent, mut requests) = fixture().await;
    let fallback = owner.snapshot().await.unwrap().session.display_name();
    let handle = run.enable_steering(Arc::new(|_, _, _| Ok(()))).unwrap();
    let request = steering(&owner, &run, "Actually name the backup task");
    let checkpoint = run.checkpoint();
    tokio::time::timeout(
        Duration::from_secs(5),
        checkpoint.with_title_updates(&agent, CancellationToken::new(), async {
            let (_, reply) = requests.recv().await.unwrap();
            handle.submit(request).await.unwrap();
            reply.send(Ok("Old intent title")).unwrap();
            let (_, reply) = requests.recv().await.unwrap();
            assert_eq!(
                owner.snapshot().await.unwrap().session.display_name(),
                fallback
            );
            reply.send(Ok("Back up project files")).unwrap();
            // The borrowed worker must finish this pending request before returning.
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        owner.snapshot().await.unwrap().session.display_name(),
        "Back up project files"
    );
}

#[tokio::test]
async fn failed_title_is_not_retried_but_next_user_message_triggers_one() {
    let (_root, owner, mut run, agent, mut requests) = fixture().await;
    let fallback = owner.snapshot().await.unwrap().session.display_name();
    let handle = run.enable_steering(Arc::new(|_, _, _| Ok(()))).unwrap();
    let request = steering(&owner, &run, "Shorten intent titles");
    let checkpoint = run.checkpoint();
    tokio::time::timeout(
        Duration::from_secs(5),
        checkpoint.with_title_updates(&agent, CancellationToken::new(), async {
            let (_, reply) = requests.recv().await.unwrap();
            reply
                .send(Err(ProviderError::Request("offline failure".into())))
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(40), requests.recv())
                    .await
                    .is_err()
            );
            assert_eq!(
                owner.snapshot().await.unwrap().session.display_name(),
                fallback
            );
            handle.submit(request).await.unwrap();
            let (_, reply) = requests.recv().await.unwrap();
            reply.send(Ok("Shorten user intent titles")).unwrap();
            name_is(&owner, "Shorten user intent titles").await;
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_stops_inflight_title_without_changing_name() {
    let (_root, owner, run, agent, mut requests) = fixture().await;
    let fallback = owner.snapshot().await.unwrap().session.display_name();
    let checkpoint = run.checkpoint();
    let cancel = CancellationToken::new();
    let mut pending = None;
    tokio::time::timeout(
        Duration::from_secs(5),
        checkpoint.with_title_updates(&agent, cancel.clone(), async {
            pending = Some(requests.recv().await.unwrap().1);
            cancel.cancel();
        }),
    )
    .await
    .unwrap();
    assert!(pending.unwrap().is_closed());
    assert_eq!(
        owner.snapshot().await.unwrap().session.display_name(),
        fallback
    );
}

#[tokio::test]
async fn real_run_refreshes_from_steering_while_main_provider_is_blocked() {
    let (_root, owner, mut run, agent, mut requests) = fixture().await;
    let handle = run.enable_steering(Arc::new(|_, _, _| Ok(()))).unwrap();
    let request = steering(&owner, &run, "Use a shorter name for user intent");
    let cancel = CancellationToken::new();
    let driver = async {
        let mut main_reply = None;
        for _ in 0..2 {
            let (request, reply) = requests.recv().await.unwrap();
            if request.model == "fixture" {
                main_reply = Some(reply);
            } else {
                reply.send(Ok("Name voyages by user intent")).unwrap();
            }
        }
        name_is(&owner, "Name voyages by user intent").await;
        handle.submit(request).await.unwrap();
        let (request, reply) = requests.recv().await.unwrap();
        assert_ne!(request.model, "fixture");
        assert!(request.messages[1].content.contains("shorter name"));
        reply.send(Ok("Shorten user intent names")).unwrap();
        name_is(&owner, "Shorten user intent names").await;
        // The main model has not replied, and steering is not yet canonical.
        assert_eq!(owner.snapshot().await.unwrap().session.messages.len(), 1);
        assert!(!main_reply.as_ref().unwrap().is_closed());
        cancel.cancel();
        main_reply
    };
    let (result, _main_reply) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(run.execute(&agent, cancel.clone(), None), driver)
    })
    .await
    .unwrap();
    assert!(result.is_err());
    assert_eq!(run.record().await.unwrap().state, RunState::Cancelled);
    let directory = _root.path().join("journal");
    let id = owner.session_id();
    drop(handle);
    drop(run);
    drop(owner);
    let reopened = Journal::open(directory).unwrap();
    assert_eq!(
        reopened.load_session(id).unwrap().session.display_name(),
        "Shorten user intent names"
    );
}
