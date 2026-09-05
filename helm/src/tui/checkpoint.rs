//! UI-thread persistence acknowledgements serialize checkpoints with operator input.
use super::{App, bridge::UiEvent};
use crate::{
    agent::{CanonicalRecovery, CheckpointError, RunCheckpoint, StopReason},
    model::{Message, Role, Usage},
    session::SessionStore,
};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(super) struct State {
    pub run_id: Uuid,
    pub baseline: Usage,
    prefix: Vec<Message>,
    canonical_saved: bool,
    accepted: bool,
}
impl State {
    pub fn new(session: &crate::session::Session, run_id: Uuid) -> Self {
        let prefix = session
            .run_summaries
            .last()
            .and_then(|s| s.message_start)
            .map(|start| {
                session
                    .messages
                    .iter()
                    .take(start + 1)
                    .filter(|m| m.role != Role::System)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Self {
            run_id,
            baseline: session.usage.clone(),
            prefix,
            canonical_saved: false,
            accepted: false,
        }
    }
}

pub(super) enum Update {
    Canonical(Vec<Message>, Usage),
    Partial(String),
    Accepted(Vec<Message>, Usage, StopReason),
}
pub struct Request {
    run_id: Uuid,
    update: Update,
    response: oneshot::Sender<Result<(), CheckpointError>>,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckpointRequest")
            .field("run_id", &self.run_id)
            .finish_non_exhaustive()
    }
}
pub(super) struct UiCheckpoint {
    pub run_id: Uuid,
    pub tx: mpsc::UnboundedSender<UiEvent>,
    pub cancel: CancellationToken,
}
impl UiCheckpoint {
    async fn send(&self, update: Update) -> Result<(), CheckpointError> {
        if self.cancel.is_cancelled() {
            return Err(CheckpointError);
        }
        let (response, receive) = oneshot::channel();
        self.tx
            .send(UiEvent::Checkpoint(Request {
                run_id: self.run_id,
                update,
                response,
            }))
            .map_err(|_| CheckpointError)?;
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(CheckpointError),
            result = receive => result.map_err(|_| CheckpointError)?,
        }
    }
}
#[async_trait]
impl RunCheckpoint for UiCheckpoint {
    fn run_id(&self) -> Uuid {
        self.run_id
    }
    async fn canonical(&self, messages: &[Message], usage: &Usage) -> Result<(), CheckpointError> {
        self.send(Update::Canonical(messages.to_vec(), usage.clone()))
            .await
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        self.send(Update::Partial(text.to_owned())).await
    }
    async fn accepted(
        &self,
        messages: &[Message],
        usage: &Usage,
        reason: &StopReason,
    ) -> Result<(), CheckpointError> {
        self.send(Update::Accepted(
            messages.to_vec(),
            usage.clone(),
            reason.clone(),
        ))
        .await
    }
}

pub(super) async fn handle(request: Request, app: &mut App, store: &SessionStore) {
    if request.response.is_closed() {
        return;
    }
    let before = super::transcript_height(app, app.conversation_width);
    let result = apply(&request, app, store).await;
    super::preserve_manual_anchor(app, before);
    if result.is_err() {
        app.status = "Session checkpoint failed; stopping run".into();
    }
    let _ = request.response.send(result);
}
async fn apply(
    request: &Request,
    app: &mut App,
    store: &SessionStore,
) -> Result<(), CheckpointError> {
    let state = app.checkpoint.as_ref().ok_or(CheckpointError)?;
    if state.accepted
        || request.run_id.is_nil()
        || request.run_id != state.run_id
        || app.session.run_summaries.last().map(|s| s.run_id) != Some(request.run_id)
        || app
            .running
            .as_ref()
            .is_some_and(|running| running.cancel.is_cancelled())
    {
        return Err(CheckpointError);
    }
    let mut next = app.session.clone();
    match &request.update {
        Update::Canonical(messages, usage) => {
            next.usage = state.baseline.clone();
            next.recover_context_failure(&CanonicalRecovery {
                messages: messages
                    .iter()
                    .filter(|m| m.role != Role::System)
                    .cloned()
                    .collect(),
                usage: usage.clone(),
            })
            .map_err(|_| CheckpointError)?;
            if !state.prefix.is_empty()
                && next.messages.len() >= state.prefix.len()
                && serde_json::to_value(&next.messages[..state.prefix.len()]).ok()
                    == serde_json::to_value(&state.prefix).ok()
            {
                next.run_summaries
                    .last_mut()
                    .ok_or(CheckpointError)?
                    .message_start = Some(state.prefix.len() - 1);
            }
            next.set_run_partial_output("")
                .map_err(|_| CheckpointError)?;
            next.refresh_active_run_summary();
        }
        Update::Partial(text) => {
            let current = &next
                .run_summaries
                .last()
                .ok_or(CheckpointError)?
                .partial_output;
            if current
                .len()
                .checked_add(text.len())
                .ok_or(CheckpointError)?
                > 1024 * 1024
            {
                return Err(CheckpointError);
            }
            let mut output = current.clone();
            output.push_str(text);
            next.set_run_partial_output(&output)
                .map_err(|_| CheckpointError)?;
        }
        Update::Accepted(messages, usage, reason) => {
            if !state.canonical_saved {
                return Err(CheckpointError);
            }
            let canonical: Vec<_> = next
                .messages
                .iter()
                .filter(|m| {
                    !m.steering
                        .as_ref()
                        .is_some_and(|s| s.status == crate::model::SteeringStatus::Queued)
                })
                .collect();
            let expected: Vec<_> = messages.iter().filter(|m| m.role != Role::System).collect();
            if next.usage.input_tokens
                != state
                    .baseline
                    .input_tokens
                    .checked_add(usage.input_tokens)
                    .ok_or(CheckpointError)?
                || next.usage.output_tokens
                    != state
                        .baseline
                        .output_tokens
                        .checked_add(usage.output_tokens)
                        .ok_or(CheckpointError)?
                || serde_json::to_value(canonical).map_err(|_| CheckpointError)?
                    != serde_json::to_value(expected).map_err(|_| CheckpointError)?
                || !next
                    .run_summaries
                    .last()
                    .ok_or(CheckpointError)?
                    .partial_output
                    .is_empty()
            {
                return Err(CheckpointError);
            }
            next.finish_run_summary(reason);
        }
    }
    store.save(&mut next).await.map_err(|_| CheckpointError)?;
    app.session = next;
    let state = app.checkpoint.as_mut().ok_or(CheckpointError)?;
    match request.update {
        Update::Canonical(..) => state.canonical_saved = true,
        Update::Accepted(..) => state.accepted = true,
        Update::Partial(..) => {}
    }
    if matches!(request.update, Update::Canonical(..)) {
        app.live_messages.clear();
        app.streaming_response.clear();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::CompletionPhase,
        model::{SteeringReceipt, SteeringStatus},
        session::Session,
    };

    fn fixture(
        root: &std::path::Path,
    ) -> (
        App,
        SessionStore,
        UiCheckpoint,
        mpsc::UnboundedReceiver<UiEvent>,
    ) {
        let mut session = Session::new(root.into(), "fixture".into());
        session
            .messages
            .push(Message::new(Role::User, "root prompt"));
        session.usage.input_tokens = 40;
        session.usage.output_tokens = 12;
        let run_id = Uuid::new_v4();
        session.begin_run_summary(run_id);
        let mut app = App::new(session, vec![]);
        app.checkpoint = Some(State::new(&app.session, run_id));
        let (tx, rx) = mpsc::unbounded_channel();
        (
            app,
            SessionStore::new(root.join("sessions")),
            UiCheckpoint {
                run_id,
                tx,
                cancel: CancellationToken::new(),
            },
            rx,
        )
    }
    async fn update(
        app: &mut App,
        store: &SessionStore,
        update: Update,
    ) -> Result<(), CheckpointError> {
        let (response, receive) = oneshot::channel();
        handle(
            Request {
                run_id: app.checkpoint.as_ref().unwrap().run_id,
                update,
                response,
            },
            app,
            store,
        )
        .await;
        receive.await.unwrap()
    }

    #[tokio::test]
    async fn canonical_checkpoints_preserve_queued_input_and_count_usage_once() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, store, _, _) = fixture(root.path());
        let mut history = app.session.messages.clone();
        history.push(Message::new(Role::Assistant, "canonical answer"));
        let receipt = Uuid::new_v4();
        let mut queued = Message::new(Role::User, "late steering");
        queued.steering = Some(SteeringReceipt {
            id: receipt,
            status: SteeringStatus::Queued,
        });
        app.session.messages.push(queued);
        app.live_messages
            .push(Message::new(Role::Assistant, "preview"));
        app.streaming_response = "preview".into();
        let usage = Usage {
            input_tokens: 3,
            output_tokens: 7,
        };
        for _ in 0..2 {
            update(
                &mut app,
                &store,
                Update::Canonical(history.clone(), usage.clone()),
            )
            .await
            .unwrap();
            assert_eq!(app.session.usage.input_tokens, 43);
            assert_eq!(app.session.usage.output_tokens, 19);
            assert_eq!(app.session.messages.len(), 3);
            assert_eq!(
                app.session.messages[2].steering.as_ref().unwrap().id,
                receipt
            );
        }
        assert!(app.live_messages.is_empty());
        assert!(app.streaming_response.is_empty());
        update(
            &mut app,
            &store,
            Update::Accepted(history, usage, StopReason::Completed),
        )
        .await
        .unwrap();
        let loaded = store.load(app.session.id).await.unwrap();
        assert_eq!(
            loaded.run_summaries.last().unwrap().phase,
            CompletionPhase::Completed
        );
        assert_eq!(loaded.messages[1].content, "canonical answer");
        assert_eq!(loaded.usage.input_tokens, 43);
    }

    #[tokio::test]
    async fn partial_is_a_bounded_annotation_and_canonical_clears_it() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, store, _, _) = fixture(root.path());
        update(&mut app, &store, Update::Partial("unfinished 世界".into()))
            .await
            .unwrap();
        let loaded = store.load(app.session.id).await.unwrap();
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.run_summaries[0].partial_output, "unfinished 世界");
        let resumed = App::new(loaded, vec![]);
        for width in [1, 12, 80] {
            let text = super::super::conversation::transcript(&resumed, width).to_string();
            assert!(text.contains("interrupted partial response"));
        }
        let before = serde_json::to_value(&app.session).unwrap();
        assert!(
            update(&mut app, &store, Update::Partial("x".repeat(1024 * 1024)))
                .await
                .is_err()
        );
        assert_eq!(serde_json::to_value(&app.session).unwrap(), before);
        let history = app.session.messages.clone();
        update(
            &mut app,
            &store,
            Update::Canonical(history, Usage::default()),
        )
        .await
        .unwrap();
        assert!(app.session.run_summaries[0].partial_output.is_empty());
    }

    #[tokio::test]
    async fn accepted_requires_matching_previously_saved_canonical_state() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, store, _, _) = fixture(root.path());
        let mut history = app.session.messages.clone();
        history.push(Message::new(Role::Assistant, "answer"));
        assert!(
            update(
                &mut app,
                &store,
                Update::Accepted(history.clone(), Usage::default(), StopReason::Completed)
            )
            .await
            .is_err()
        );
        update(
            &mut app,
            &store,
            Update::Canonical(history.clone(), Usage::default()),
        )
        .await
        .unwrap();
        assert!(
            update(
                &mut app,
                &store,
                Update::Accepted(
                    history.clone(),
                    Usage {
                        input_tokens: 1,
                        output_tokens: 0
                    },
                    StopReason::Completed
                )
            )
            .await
            .is_err()
        );
        update(&mut app, &store, Update::Partial("new partial".into()))
            .await
            .unwrap();
        assert!(
            update(
                &mut app,
                &store,
                Update::Accepted(history, Usage::default(), StopReason::Completed)
            )
            .await
            .is_err()
        );
        assert_ne!(
            app.session.run_summaries[0].phase,
            CompletionPhase::Completed
        );
    }

    #[tokio::test]
    async fn failed_save_does_not_acknowledge_or_change_authoritative_session() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _, _, _) = fixture(root.path());
        let blocked = root.path().join("file");
        std::fs::write(&blocked, "not a directory").unwrap();
        let store = SessionStore::new(blocked);
        let before = serde_json::to_value(&app.session).unwrap();
        assert!(
            update(&mut app, &store, Update::Partial("must not commit".into()))
                .await
                .is_err()
        );
        assert_eq!(serde_json::to_value(&app.session).unwrap(), before);
    }

    #[tokio::test]
    async fn cancellation_and_closed_acknowledgements_leave_no_late_mutation() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, store, checkpoint, mut rx) = fixture(root.path());
        let cancel = checkpoint.cancel.clone();
        let task = tokio::spawn(async move { checkpoint.partial("private text").await });
        let UiEvent::Checkpoint(request) = rx.recv().await.unwrap() else {
            panic!("checkpoint expected")
        };
        assert!(
            !task.is_finished(),
            "callback must wait for durable acknowledgement"
        );
        assert!(!format!("{request:?}").contains("private text"));
        cancel.cancel();
        assert!(task.await.unwrap().is_err());
        handle(request, &mut app, &store).await;
        assert!(app.session.run_summaries[0].partial_output.is_empty());
        let (response, receive) = oneshot::channel();
        handle(
            Request {
                run_id: Uuid::new_v4(),
                update: Update::Partial("stale".into()),
                response,
            },
            &mut app,
            &store,
        )
        .await;
        assert!(receive.await.unwrap().is_err());
        assert!(app.session.run_summaries[0].partial_output.is_empty());
    }

    #[tokio::test]
    async fn channel_shutdown_and_usage_overflow_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, store, checkpoint, rx) = fixture(root.path());
        drop(rx);
        assert!(checkpoint.partial("undeliverable").await.is_err());
        let history = app.session.messages.clone();
        assert!(
            update(
                &mut app,
                &store,
                Update::Canonical(
                    history,
                    Usage {
                        input_tokens: u64::MAX,
                        output_tokens: 0
                    }
                )
            )
            .await
            .is_err()
        );
        assert_eq!(app.session.usage.input_tokens, 40);
    }
}
