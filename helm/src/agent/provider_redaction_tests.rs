//! Runtime boundaries, independent of provider transport/redaction internals.
use super::*;
use crate::model::{ModelResponse, Role, ToolCall, ToolDefinition};
use crate::provider::{ProviderDelta, ProviderStream, ProviderStreamEvent};
use crate::tools::{Tool, ToolError};
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

const SECRET: &str = "p秘密\"\\\n\u{1b}z";
const SAFE: &str = "public opaque bytes \\u001b Ω";

#[derive(Default)]
struct Evidence {
    requests: Mutex<Vec<ModelRequest>>,
    partials: Mutex<Vec<String>>,
    snapshots: Mutex<Vec<Vec<Message>>>,
    events: Mutex<Vec<String>>,
    effects: AtomicUsize,
}
#[async_trait]
impl EventSink for Evidence {
    async fn emit(&self, event: AgentEvent) {
        let text = match event {
            AgentEvent::AssistantText(text) | AgentEvent::AssistantTextDelta(text) => Some(text),
            AgentEvent::ToolStarted { arguments, .. } => Some(arguments.to_string()),
            _ => None,
        };
        if let Some(text) = text {
            self.events.lock().unwrap().push(text);
        }
    }
}
struct Checkpoint {
    evidence: Arc<Evidence>,
    run_id: uuid::Uuid,
}
impl Checkpoint {
    fn new(evidence: Arc<Evidence>) -> Self {
        Self {
            evidence,
            run_id: uuid::Uuid::new_v4(),
        }
    }
}
#[async_trait]
impl RunCheckpoint for Checkpoint {
    fn run_id(&self) -> uuid::Uuid {
        self.run_id
    }
    async fn canonical(&self, messages: &[Message], _: &Usage) -> Result<(), CheckpointError> {
        self.evidence
            .snapshots
            .lock()
            .unwrap()
            .push(messages.to_vec());
        Ok(())
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        self.evidence.partials.lock().unwrap().push(text.into());
        Ok(())
    }
    async fn unstreamed(&self, text: &str) -> Result<(), CheckpointError> {
        self.partial(text).await
    }
}
struct Effect(Arc<Evidence>);
#[async_trait]
impl Tool for Effect {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "effect".into(),
            description: "count fixture effects".into(),
            input_schema: json!({"type":"object"}),
        }
    }
    async fn execute(&self, _: Value, _: &ToolContext) -> Result<String, ToolError> {
        self.0.effects.fetch_add(1, Ordering::SeqCst);
        Ok("observed fixture effect".into())
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Streamed,
    Completed,
    Error,
    Cancel,
    SecretOpaque,
    SecretArguments,
    History,
}
struct Script {
    evidence: Arc<Evidence>,
    mode: Mode,
    cancel: CancellationToken,
}
fn state(text: &str, opaque: &str) -> Value {
    json!({"kind":"openai_responses_replay","version":1,"items":[
        {"type":"reasoning","id":"reasoning_1","encrypted_content":opaque,"summary":[]},
        {"type":"message","id":"message_1","role":"assistant","content":[{"type":"output_text","text":text,"annotations":[]}]}
    ]})
}
#[async_trait]
impl Provider for Script {
    async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
        unreachable!()
    }
    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        let turn = self.evidence.requests.lock().unwrap().len();
        self.evidence.requests.lock().unwrap().push(request);
        let mode = self.mode;
        let cancel = self.cancel.clone();
        Ok(Box::pin(async_stream::stream! {
            if turn > 0 || matches!(mode,Mode::History) {
                yield Ok(ProviderStreamEvent::Completed(ModelResponse { message:Message::new(Role::Assistant,"finished"),usage:Usage::default() }));
                return;
            }
            let text = format!("A long public prefix that should remain observable before a failed stream. {SECRET} public suffix");
            if matches!(mode,Mode::Streamed | Mode::Error | Mode::Cancel) {
                // Every Unicode scalar is its own valid UTF-8 delta, including
                // quote, backslash, newline and ESC inside the configured secret.
                for character in text.chars() {
                    yield Ok(ProviderStreamEvent::Delta(ProviderDelta::Text(character.to_string())));
                }
            }
            if matches!(mode,Mode::Error) {
                yield Err(ProviderError::Unavailable("synthetic stream failure".into()));
                return;
            }
            if matches!(mode,Mode::Cancel) {
                cancel.cancel();
                std::future::pending::<()>().await;
                return;
            }
            let mut message = Message::new(Role::Assistant,text.clone());
            message.provider_state = Some(state(&text,if matches!(mode,Mode::SecretOpaque) { SECRET } else { SAFE }));
            message.tool_calls.push(ToolCall { id:"effect_1".into(),name:"effect".into(),arguments:json!({"value":if matches!(mode,Mode::SecretArguments) { SECRET } else { "public" }}) });
            yield Ok(ProviderStreamEvent::Completed(ModelResponse { message,usage:Usage { input_tokens:7,output_tokens:3 } }));
        }))
    }
}
fn assert_safe(value: &Value) {
    match value {
        Value::String(text) => assert!(
            !text.contains(SECRET),
            "configured secret crossed runtime boundary"
        ),
        Value::Array(values) => values.iter().for_each(assert_safe),
        Value::Object(values) => {
            for (key, value) in values {
                assert!(!key.contains(SECRET));
                assert_safe(value);
            }
        }
        _ => (),
    }
}
fn setup(mode: Mode) -> (tempfile::TempDir, Agent, Arc<Evidence>, CancellationToken) {
    let directory = tempfile::tempdir().unwrap();
    let evidence = Arc::new(Evidence::default());
    let cancel = CancellationToken::new();
    let mut agent = tests::agent(
        Box::new(Script {
            evidence: evidence.clone(),
            mode,
            cancel: cancel.clone(),
        }),
        &directory,
    );
    agent.context.redactor = Arc::new(crate::tools::Redactor::new(vec![SECRET.into()]));
    agent.sink = evidence.clone();
    agent.tools.register(Effect(evidence.clone()));
    (directory, agent, evidence, cancel)
}

#[tokio::test]
async fn streamed_and_completed_text_are_safe_in_checkpoints_and_provider_followup() {
    for mode in [Mode::Streamed, Mode::Completed] {
        let (_directory, agent, evidence, cancel) = setup(mode);
        let outcome = agent
            .run_checkpointed(
                vec![],
                "public request".into(),
                cancel,
                None,
                &Checkpoint::new(evidence.clone()),
                agent.model(),
            )
            .await
            .unwrap();
        assert_eq!(evidence.effects.load(Ordering::SeqCst), 1);
        let requests = evidence.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "exercise an actual follow-up provider request"
        );
        assert_safe(&serde_json::to_value(&*requests).unwrap());
        assert_safe(&serde_json::to_value(&outcome.messages).unwrap());
        assert_safe(&serde_json::to_value(&*evidence.snapshots.lock().unwrap()).unwrap());
        let partials = evidence.partials.lock().unwrap().concat();
        let events = evidence.events.lock().unwrap().concat();
        assert!(!partials.contains(SECRET));
        assert!(!events.contains(SECRET));
        assert!(partials.contains("[REDACTED]"));
        assert!(events.contains("[REDACTED]"));
        let replay = outcome
            .messages
            .iter()
            .find_map(|message| message.provider_state.as_ref())
            .unwrap();
        assert_eq!(
            replay["items"][0]["encrypted_content"], SAFE,
            "nonsecret opaque bytes changed"
        );
        assert!(
            replay["items"][1]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("[REDACTED]")
        );
    }
}

#[tokio::test]
async fn failed_and_cancelled_split_secret_streams_never_publish_secret_partials() {
    for mode in [Mode::Error, Mode::Cancel] {
        let (_directory, agent, evidence, cancel) = setup(mode);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            agent.run_checkpointed(
                vec![],
                "public request".into(),
                cancel,
                None,
                &Checkpoint::new(evidence.clone()),
                agent.model(),
            ),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        assert_eq!(evidence.effects.load(Ordering::SeqCst), 0);
        assert_eq!(
            evidence.requests.lock().unwrap().len(),
            1,
            "partial stream must not retry"
        );
        for text in [
            evidence.partials.lock().unwrap().concat(),
            evidence.events.lock().unwrap().concat(),
        ] {
            assert!(!text.contains(SECRET));
            assert!(
                text.contains("long public prefix"),
                "safe streaming text was wholly discarded"
            );
        }
        assert_safe(&serde_json::to_value(&*evidence.snapshots.lock().unwrap()).unwrap());
    }
}

#[tokio::test]
async fn secret_opaque_replay_and_tool_arguments_fail_before_tool_effects() {
    for mode in [Mode::SecretOpaque, Mode::SecretArguments] {
        let (_directory, agent, evidence, cancel) = setup(mode);
        let result = agent
            .run_checkpointed(
                vec![],
                "public request".into(),
                cancel,
                None,
                &Checkpoint::new(evidence.clone()),
                agent.model(),
            )
            .await;
        assert!(
            result.is_err(),
            "secret-bearing executable/opaque state must refuse"
        );
        assert_eq!(evidence.effects.load(Ordering::SeqCst), 0);
        assert_eq!(evidence.requests.lock().unwrap().len(), 1);
        assert_safe(&serde_json::to_value(&*evidence.snapshots.lock().unwrap()).unwrap());
        assert!(!evidence.events.lock().unwrap().concat().contains(SECRET));
    }
}

#[tokio::test]
async fn outgoing_history_projection_does_not_rewrite_original_history() {
    let (_directory, agent, evidence, _cancel) = setup(Mode::History);
    let mut prior = Message::new(Role::Assistant, format!("previous {SECRET}"));
    prior.provider_state = Some(state(&prior.content, SAFE));
    let history = vec![Message::new(Role::User, format!("earlier {SECRET}")), prior];
    let original = serde_json::to_value(&history).unwrap();
    let outcome = agent
        .run(history.clone(), "public follow-up".into())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&history).unwrap(), original);
    assert_eq!(
        serde_json::to_value(&outcome.messages[..2]).unwrap(),
        original,
        "request projection rewrote canonical past history"
    );
    let requests = evidence.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_safe(&serde_json::to_value(&*requests).unwrap());
    let replay = requests[0]
        .messages
        .iter()
        .find_map(|message| message.provider_state.as_ref())
        .unwrap();
    assert_eq!(replay["items"][0]["encrypted_content"], SAFE);
}

#[tokio::test]
async fn secret_bearing_historical_opaque_state_refuses_before_provider_dispatch() {
    let (_directory, agent, evidence, _cancel) = setup(Mode::History);
    let mut prior = Message::new(Role::Assistant, "public previous message");
    prior.provider_state = Some(state(&prior.content, SECRET));
    let history = vec![prior];
    let original = serde_json::to_value(&history).unwrap();
    assert!(
        agent
            .run(history.clone(), "public follow-up".into())
            .await
            .is_err()
    );
    assert!(evidence.requests.lock().unwrap().is_empty());
    assert_eq!(evidence.effects.load(Ordering::SeqCst), 0);
    assert_eq!(serde_json::to_value(history).unwrap(), original);
}
