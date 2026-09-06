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
