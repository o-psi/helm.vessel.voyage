use super::*;
use crate::model::Role;
use crate::provider::{ProviderDelta, ProviderStreamEvent};
use async_trait::async_trait;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct ProviderFixture(u8);
#[async_trait]
impl Provider for ProviderFixture {
    async fn complete(
        &self,
        _: ModelRequest,
    ) -> Result<crate::model::ModelResponse, ProviderError> {
        unreachable!()
    }
    async fn stream(
        &self,
        _: ModelRequest,
    ) -> Result<crate::provider::ProviderStream, ProviderError> {
        let mut events = vec![];
        if self.0 == 1 {
            for text in ["hel", "lo"] {
                events.push(Ok(ProviderStreamEvent::Delta(ProviderDelta::Text(
                    text.into(),
                ))));
            }
        }
        if self.0 == 2 {
            events.push(Ok(ProviderStreamEvent::Delta(ProviderDelta::ToolCall {
                index: 0,
                id: Some("tool-fragment".into()),
                name: Some("example".into()),
                arguments: "{}".into(),
            })));
        }
        events.push(Ok(ProviderStreamEvent::Completed(
            crate::model::ModelResponse {
                message: Message::new(Role::Assistant, "hello"),
                usage: Usage {
                    input_tokens: 7,
                    output_tokens: 3,
                },
            },
        )));
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
struct Projection {
    run_id: uuid::Uuid,
    fragments: Mutex<Vec<String>>,
    canonical: Mutex<(Vec<Message>, Usage)>,
    accepted: AtomicUsize,
    behavior: u8,
    cancel: CancellationToken,
}
#[async_trait]
impl RunCheckpoint for Projection {
    fn run_id(&self) -> uuid::Uuid {
        self.run_id
    }
    async fn canonical(&self, messages: &[Message], usage: &Usage) -> Result<(), CheckpointError> {
        *self.canonical.lock().unwrap() = (messages.to_vec(), usage.clone());
        Ok(())
    }
    async fn unstreamed(&self, text: &str) -> Result<(), CheckpointError> {
        self.partial(text).await
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        self.fragments.lock().unwrap().push(text.into());
        match self.behavior {
            1 => Err(CheckpointError),
            2 => std::future::pending().await,
            3 => {
                self.cancel.cancel();
                std::future::pending().await
            }
            _ => Ok(()),
        }
    }
    async fn accepted(
        &self,
        _: &[Message],
        _: &Usage,
        _: &StopReason,
    ) -> Result<(), CheckpointError> {
        self.accepted.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[tokio::test]
async fn completed_only_and_streamed_text_have_one_durable_projection() {
    for streamed in [0, 1, 2] {
        let root = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let checkpoint = Projection {
            run_id: uuid::Uuid::new_v4(),
            fragments: Mutex::new(vec![]),
            canonical: Mutex::new((vec![], Usage::default())),
            accepted: AtomicUsize::new(0),
            behavior: 0,
            cancel: cancel.clone(),
        };
        let agent = super::tests::agent(Box::new(ProviderFixture(streamed)), &root);
        let result = agent
            .run_checkpointed(
                vec![],
                "question".into(),
                cancel,
                None,
                &checkpoint,
                agent.model(),
            )
            .await
            .unwrap();
        let fragments = checkpoint.fragments.lock().unwrap();
        assert_eq!(fragments.concat(), "hello");
        assert_eq!(fragments.len(), if streamed == 1 { 2 } else { 1 });
        assert_eq!(
            result
                .messages
                .iter()
                .filter(|m| m.role == Role::Assistant)
                .count(),
            1
        );
        assert_eq!(checkpoint.accepted.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn failed_cancelled_or_timed_out_fallback_preserves_known_canonical_usage_without_acceptance()
{
    for behavior in [1, 2, 3] {
        let root = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let checkpoint = Projection {
            run_id: uuid::Uuid::new_v4(),
            fragments: Mutex::new(vec![]),
            canonical: Mutex::new((vec![], Usage::default())),
            accepted: AtomicUsize::new(0),
            behavior,
            cancel: cancel.clone(),
        };
        let mut agent = super::tests::agent(Box::new(ProviderFixture(0)), &root);
        agent.context.timeout = Duration::from_millis(20);
        let result = agent
            .run_checkpointed(
                vec![],
                "question".into(),
                cancel,
                None,
                &checkpoint,
                agent.model(),
            )
            .await;
        if behavior == 3 {
            assert!(matches!(result, Err(AgentError::Cancelled)));
        } else {
            assert!(matches!(result, Err(AgentError::Checkpoint(_))));
        }
        let (messages, usage) = &*checkpoint.canonical.lock().unwrap();
        assert_eq!((usage.input_tokens, usage.output_tokens), (7, 3));
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.role == Role::Assistant && m.content == "hello")
                .count(),
            1
        );
        assert_eq!(checkpoint.accepted.load(Ordering::SeqCst), 0);
    }
}
